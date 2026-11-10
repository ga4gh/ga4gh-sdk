use ga4gh_sdk::{
    clients::{
        serviceinfo::models::{Service, ServiceType},
        tes::{models::TesTask, TES},
        wes::Run,
    },
    utils::{
        acknowledgement::AcknowledgementError, configuration::Configuration, transport::Transport,
    },
};
use mockito::{mock, server_url};

#[tokio::test]
async fn malformed_tes_acknowledgements_never_return_handles_or_replay() {
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
    let tes = TES {
        transport: Transport::new(&config),
        config,
        service: Ok(Service {
            r#type: Box::new(ServiceType {
                artifact: "tes".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
    for body in [
        "{}",
        r#"{"id":null}"#,
        r#"{"id":123}"#,
        r#"{"id":""}"#,
        "null",
        "invalid",
    ] {
        let request = mock("POST", "/tasks")
            .with_status(200)
            .with_body(body)
            .expect(1)
            .create();
        let error = tes.create(TesTask::default()).await.unwrap_err();
        assert!(
            error.downcast_ref::<AcknowledgementError>().is_some(),
            "{error}"
        );
        assert!(error.to_string().contains("may have succeeded"));
        request.assert();
    }
}

#[tokio::test]
async fn opaque_tes_id_survives_creation_and_later_requests() {
    let id = "\"opaque / % id\"";
    let create = mock("POST", "/tasks")
        .with_status(200)
        .with_body(serde_json::json!({"id":id}).to_string())
        .expect(1)
        .create();
    let status = mock("GET", "/tasks/%22opaque%20%2F%20%25%20id%22?view=MINIMAL")
        .with_status(200)
        .with_body(r#"{"state":"COMPLETE","executors":[]}"#)
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
    let tes = TES {
        transport: Transport::new(&config),
        config,
        service: Ok(Service {
            r#type: Box::new(ServiceType {
                artifact: "tes".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
    let task = tes.create(TesTask::default()).await.unwrap();
    assert_eq!(task.id, id);
    task.status().await.unwrap();
    create.assert();
    status.assert();
}

#[tokio::test]
async fn wes_cancel_requires_the_exact_requested_id_without_replay() {
    let transport = Transport::new(&Configuration::new(url::Url::parse(&server_url()).unwrap()));
    let run = Run::new("target-run".into(), transport);
    for body in [
        "{}",
        r#"{"run_id":""}"#,
        r#"{"run_id":"another-run"}"#,
        r#"{"run_id":null}"#,
        r#"{"run_id":5}"#,
    ] {
        let request = mock("POST", "/runs/target-run/cancel")
            .with_status(200)
            .with_body(body)
            .expect(1)
            .create();
        assert!(run
            .cancel()
            .await
            .unwrap_err()
            .downcast_ref::<AcknowledgementError>()
            .is_some());
        request.assert();
    }
    let request = mock("POST", "/runs/target-run/cancel")
        .with_status(200)
        .with_body(r#"{"run_id":"target-run"}"#)
        .expect(1)
        .create();
    assert_eq!(
        run.cancel().await.unwrap().run_id.as_deref(),
        Some("target-run")
    );
    request.assert();
}

#[tokio::test]
async fn wes_creation_validates_once_and_preserves_opaque_ids() {
    use ga4gh_sdk::clients::wes::{models::WesRunRequest, WES};
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
    let wes = WES {
        transport: Transport::new(&config),
        config,
        service: Ok(Service {
            r#type: Box::new(ServiceType {
                artifact: "wes".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
    let request = WesRunRequest {
        workflow_type: Some("CWL".into()),
        workflow_type_version: Some("v1.2".into()),
        workflow_url: Some("main.cwl".into()),
        workflow_params: Some(serde_json::json!({})),
        ..Default::default()
    };
    for body in [
        "{}",
        r#"{"run_id":null}"#,
        r#"{"run_id":4}"#,
        r#"{"run_id":""}"#,
        "invalid",
    ] {
        let submission = mock("POST", "/runs")
            .with_status(200)
            .with_body(body)
            .expect(1)
            .create();
        assert!(wes
            .run_workflow(request.clone())
            .await
            .unwrap_err()
            .downcast_ref::<AcknowledgementError>()
            .is_some());
        submission.assert();
    }
    let id = "\"opaque / % id\"";
    let submission = mock("POST", "/runs")
        .with_status(200)
        .with_body(serde_json::json!({"run_id":id}).to_string())
        .expect(1)
        .create();
    assert_eq!(wes.run_workflow(request).await.unwrap().id, id);
    submission.assert();
}
