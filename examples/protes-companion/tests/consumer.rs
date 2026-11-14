use ga4gh_sdk::utils::{
    configuration::Configuration,
    transport::{HttpStatusError, Transport},
};
use protes_companion::{
    explain, fetch, prepare_rerun, BackendMode, BackendSelection, Choices, RerunPlan,
};
use serde_json::json;
#[test]
fn preserves_order_unknown_fields_builds_and_unavailable_replay() {
    let raw = json!({"pipeline":{"source":"preset","resolved_hash":"sha256:abc","new_field":1},"builds":{"build1":{"version":"1.0"}},"executions":[{"step":"rewrite","phase":"request","outcome":"ok","changes":{"paths":["/resources/ram_gb"],"before":"hmac-sha256:before","after":"hmac-sha256:after"}}],"attempts":[{"number":1,"backend":"a","outcome":"rejected"},{"number":2,"backend":"b","outcome":"unknown"}],"final_reason":"outcome_unknown","future":true});
    let e = explain(raw.clone(), false).unwrap();
    assert_eq!(e.original_record, raw);
    assert_eq!(e.record.attempts.unwrap()[1].number, Some(2));
    assert_eq!(
        e.record.executions.unwrap()[0]
            .changes
            .as_ref()
            .unwrap()
            .paths,
        vec!["/resources/ram_gb"]
    );
    assert!(e.replay_reconstruction.starts_with("unavailable"));
    assert_eq!(e.record.extra["future"], true);
}
#[test]
fn plan_discards_server_fields_and_requires_explicit_gateway_choices() {
    let raw = json!({"id":"old","state":"COMPLETE","creation_time":"yesterday","logs":[],"executors":[{"image":"busybox","command":["echo","ok"]}],"tags":{"study":"qc","gateway.backend":"b","gateway.pipeline":"old","gateway.backends":"old","gateway.route":"[]","gateway.backend_task_id":"old"}});
    let plan = prepare_rerun(&raw, Choices::default()).unwrap();
    assert_eq!(plan.request()["tags"], json!({"study":"qc"}));
    for key in ["id", "state", "creation_time", "logs"] {
        assert!(plan.request().get(key).is_none());
    }
    assert_eq!(plan.removed_gateway_tags().len(), 5);
    let plan = prepare_rerun(
        &raw,
        Choices {
            pipeline: Some("qc".into()),
            backends: Some(BackendSelection {
                mode: BackendMode::Allow,
                backends: vec!["https://node.example/tes".into()],
            }),
        },
    )
    .unwrap();
    assert_eq!(plan.request()["tags"]["gateway.pipeline"], "qc");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            plan.request()["tags"]["gateway.backends"].as_str().unwrap()
        )
        .unwrap(),
        json!({"type":"allow","backends":["https://node.example/tes"]})
    );
}

#[test]
fn saved_plan_is_the_request_that_create_will_submit() {
    let plan = prepare_rerun(
        &json!({"executors":[{"image":"alpine","command":["true"],"ignore_error":true}],"tags":{"study":"qc"}}),
        Choices {
            pipeline: Some("qc".into()),
            backends: Some(BackendSelection {
                mode: BackendMode::Allow,
                backends: vec!["https://node.example/tes".into()],
            }),
        },
    )
    .unwrap();
    let saved = serde_json::to_value(&plan).unwrap();
    let loaded = RerunPlan::from_value(saved.clone()).unwrap();
    assert_eq!(loaded.request(), plan.request());
    assert_eq!(loaded.request()["executors"][0]["ignore_error"], true);

    let mut with_server_field = saved.clone();
    with_server_field["request"]["id"] = json!("old-task");
    assert!(RerunPlan::from_value(with_server_field).is_err());
    let mut with_gateway_state = saved.clone();
    with_gateway_state["request"]["tags"]["gateway.route"] = json!("old-backend");
    assert!(RerunPlan::from_value(with_gateway_state).is_err());
    let mut with_unknown_version = saved;
    with_unknown_version["format_version"] = json!(2);
    assert!(RerunPlan::from_value(with_unknown_version).is_err());
    assert!(serde_json::from_value::<Choices>(json!({"unsupported_choice":"qc"})).is_err());
}

#[test]
fn cli_create_submits_the_saved_plan_once() {
    use ga4gh_sdk::clients::serviceinfo::models::{Service, ServiceType};
    use std::{
        fs,
        process::Command,
        time::{SystemTime, UNIX_EPOCH},
    };

    let home = std::env::temp_dir().join(format!(
        "protes-companion-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(home.join(".ga4gh-cli")).unwrap();
    let base_path = format!("{}/cli", mockito::server_url());
    fs::write(
        home.join(".ga4gh-cli/config.json"),
        json!({"TES":{"base_path":base_path,"oauth_access_token":"fixture-token"}}).to_string(),
    )
    .unwrap();
    let plan = prepare_rerun(
        &json!({"executors":[{"image":"alpine","command":["true"],"ignore_error":true}]}),
        Choices::default(),
    )
    .unwrap();
    let plan_path = home.join("plan.json");
    fs::write(&plan_path, serde_json::to_vec(&plan).unwrap()).unwrap();
    let service = Service {
        r#type: Box::new(ServiceType {
            artifact: "tes".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let discovery = mockito::mock("GET", "/cli/service-info")
        .match_header("authorization", "Bearer fixture-token")
        .with_body(serde_json::to_string(&service).unwrap())
        .expect(1)
        .create();
    let submission = mockito::mock("POST", "/cli/tasks")
        .match_header("authorization", "Bearer fixture-token")
        .match_body(mockito::Matcher::Regex("\\\"ignore_error\\\":true".into()))
        .with_body(r#"{"id":"new-task"}"#)
        .expect(1)
        .create();
    let result = Command::new(env!("CARGO_BIN_EXE_protes-companion"))
        .arg("create")
        .arg(&plan_path)
        .env("HOME", &home)
        .output()
        .unwrap();
    fs::remove_dir_all(home).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&result.stdout).unwrap(),
        json!({"task_id":"new-task"})
    );
    discovery.assert();
    submission.assert();
}

#[tokio::test]
async fn configured_transport_honors_the_request_timeout() {
    use ga4gh_sdk::utils::timeouts::Timeouts;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let config = Configuration::new(
        url::Url::parse(&format!("http://{}/tes", listener.local_addr().unwrap())).unwrap(),
    )
    .with_timeouts(Timeouts {
        request: Duration::from_millis(100),
        ..Default::default()
    });
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let bytes_read = socket.read(&mut request).unwrap();
        assert!(bytes_read > 0);
        std::thread::sleep(Duration::from_millis(300));
        let _ = socket.write_all(
            b"HTTP/1.1 200 OK\r\ncontent-length: 15\r\nconnection: close\r\n\r\n{\"future\":true}",
        );
    });

    let error = fetch(
        &protes_companion::configured_transport(&config).unwrap(),
        "one",
    )
    .await
    .unwrap_err();
    assert!(error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(reqwest::Error::is_timeout));
    server.join().unwrap();
}
#[tokio::test]
async fn fetch_retains_403_404_semantics_and_never_posts() {
    let transport = Transport::new(&Configuration::new(
        url::Url::parse(&mockito::server_url()).unwrap(),
    ));
    for status in [403, 404] {
        let mock = mockito::mock("GET", "/tasks/id%2Fencoded/provenance")
            .with_status(status)
            .with_body("{\"detail\":\"Unavailable\"}")
            .expect(1)
            .create();
        let e = fetch(&transport, "id/encoded").await.unwrap_err();
        assert_eq!(
            e.downcast_ref::<HttpStatusError>().unwrap().status.as_u16(),
            status as u16
        );
        mock.assert();
    }
}

#[tokio::test]
async fn oauth_authentication_is_shared_by_provenance_and_explicit_tes_create() {
    use ga4gh_sdk::clients::serviceinfo::models::{Service, ServiceType};
    let mut config =
        Configuration::new(url::Url::parse(&format!("{}/auth", mockito::server_url())).unwrap());
    config.oauth_access_token = Some("fixture-token".into());
    let provenance = mockito::mock("GET", "/auth/tasks/one/provenance")
        .match_header("authorization", "Bearer fixture-token")
        .with_body("{\"future\":true}")
        .expect(1)
        .create();
    protes_companion::fetch(
        &protes_companion::configured_transport(&config).unwrap(),
        "one",
    )
    .await
    .unwrap();
    provenance.assert();
    let service = Service {
        r#type: Box::new(ServiceType {
            artifact: "tes".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let discovery = mockito::mock("GET", "/auth/service-info")
        .match_header("authorization", "Bearer fixture-token")
        .with_body(serde_json::to_string(&service).unwrap())
        .expect(1)
        .create();
    let create = mockito::mock("POST", "/auth/tasks")
        .match_body(mockito::Matcher::Regex("\\\"ignore_error\\\":true".into()))
        .match_header("authorization", "Bearer fixture-token")
        .with_body("{\"id\":\"second\"}")
        .expect(1)
        .create();
    let client = protes_companion::tes_client(&config).await.unwrap();
    let plan = prepare_rerun(
        &json!({"executors":[{"image":"alpine","command":["true"],"ignore_error":true}]}),
        Choices::default(),
    )
    .unwrap();
    assert_eq!(
        protes_companion::create_plan(&client, &plan)
            .await
            .unwrap()
            .id,
        "second"
    );
    discovery.assert();
    create.assert();
}

#[tokio::test]
async fn malformed_acknowledgements_do_not_replay_the_submission() {
    use ga4gh_sdk::{
        clients::serviceinfo::models::{Service, ServiceType},
        utils::acknowledgement::AcknowledgementError,
    };
    let config =
        Configuration::new(url::Url::parse(&format!("{}/ack", mockito::server_url())).unwrap());
    let service = Service {
        r#type: Box::new(ServiceType {
            artifact: "tes".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let discovery = mockito::mock("GET", "/ack/service-info")
        .with_body(serde_json::to_string(&service).unwrap())
        .expect(1)
        .create();
    let client = protes_companion::tes_client(&config).await.unwrap();
    discovery.assert();
    let plan = prepare_rerun(
        &json!({"executors":[{"image":"alpine","command":["true"]}]}),
        Choices::default(),
    )
    .unwrap();
    for body in ["{}", r#"{"id":null}"#, r#"{"id":4}"#, r#"{"id":""}"#] {
        let submission = mockito::mock("POST", "/ack/tasks")
            .with_body(body)
            .expect(1)
            .create();
        let error = protes_companion::create_plan(&client, &plan)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<AcknowledgementError>().is_some());
        submission.assert();
    }
}

#[tokio::test]
async fn server_error_is_ambiguous_but_client_error_retains_status() {
    use ga4gh_sdk::{
        clients::serviceinfo::models::{Service, ServiceType},
        utils::operation::AmbiguousSubmissionError,
    };
    let config =
        Configuration::new(url::Url::parse(&format!("{}/outcome", mockito::server_url())).unwrap());
    let service = Service {
        r#type: Box::new(ServiceType {
            artifact: "tes".into(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let discovery = mockito::mock("GET", "/outcome/service-info")
        .with_body(serde_json::to_string(&service).unwrap())
        .expect(1)
        .create();
    let client = protes_companion::tes_client(&config).await.unwrap();
    discovery.assert();
    let plan = prepare_rerun(
        &json!({"executors":[{"image":"alpine","command":["true"]}]}),
        Choices::default(),
    )
    .unwrap();

    for status in [503, 403] {
        let submission = mockito::mock("POST", "/outcome/tasks")
            .with_status(status)
            .with_body("gateway detail")
            .expect(1)
            .create();
        let error = protes_companion::create_plan(&client, &plan)
            .await
            .unwrap_err();
        if status == 503 {
            assert!(error.downcast_ref::<AmbiguousSubmissionError>().is_some());
            assert!(!error.to_string().contains("gateway detail"));
        } else {
            let http = error.downcast_ref::<HttpStatusError>().unwrap();
            assert_eq!(http.status.as_u16(), 403);
            assert_eq!(http.body, "gateway detail");
        }
        submission.assert();
    }
}
