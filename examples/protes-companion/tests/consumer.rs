use ga4gh_sdk::utils::{
    configuration::Configuration,
    transport::{HttpStatusError, Transport},
};
use protes_companion::{explain, fetch, prepare_rerun, BackendMode, BackendSelection, Choices};
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
    use ga4gh_sdk::clients::{
        serviceinfo::models::{Service, ServiceType},
        tes::TES,
    };
    let config =
        Configuration::new(url::Url::parse(&format!("{}/ack", mockito::server_url())).unwrap());
    let client = TES {
        config: config.clone(),
        transport: Transport::new(&config),
        service: Ok(Service {
            r#type: Box::new(ServiceType {
                artifact: "tes".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
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
        assert!(protes_companion::create_plan(&client, &plan)
            .await
            .unwrap_err()
            .to_string()
            .contains("reconcile"));
        submission.assert();
    }
}
