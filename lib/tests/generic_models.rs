//! Client methods that take and return downstream types, and clients for
//! services whose service info names another artifact.

use ga4gh_sdk::clients::drs::DRS;
use ga4gh_sdk::clients::tes::models::{TesExecutor, TesTask};
use ga4gh_sdk::clients::tes::{TesView, TES};
use ga4gh_sdk::clients::wes::WES;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::Transport;
use ga4gh_sdk::Error;
use mockito::{Matcher, Mock};
use serde::{Deserialize, Serialize};
use serde_json::json;

fn service_info(
    server: &mut mockito::ServerGuard,
    path: &str,
    group: &str,
    artifact: &str,
) -> Mock {
    server
        .mock("GET", format!("{path}/service-info").as_str())
        .with_body(
            json!({
                "id": "org.example.service",
                "name": "Example",
                "type": {"group": group, "artifact": artifact, "version": "1.1.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0"
            })
            .to_string(),
        )
        .create()
}

fn transport(server: &mockito::ServerGuard, path: &str) -> Transport {
    Transport::new(&Configuration::new(
        url::Url::parse(&format!("{}{path}", server.url())).unwrap(),
    ))
}

/// A TES task with a field of a downstream service.
#[derive(Debug, Serialize, Deserialize)]
struct LabelledTask {
    #[serde(flatten)]
    task: TesTask,
    label: Option<String>,
}

/// Reads only the fields that a downstream caller needs.
#[derive(Debug, Deserialize)]
struct Labelled {
    id: String,
    label: String,
}

#[tokio::test]
async fn tes_sends_and_reads_downstream_types() {
    let mut server = mockito::Server::new_async().await;
    let _info = service_info(&mut server, "/generic/tes", "org.ga4gh", "tes");
    let created = server
        .mock("POST", "/generic/tes/tasks")
        .match_body(Matcher::PartialJson(json!({"label": "blue"})))
        .with_body(r#"{"id":"task-1"}"#)
        .create();
    let _fetched = server
        .mock("GET", "/generic/tes/tasks/task-1?view=BASIC")
        .with_body(r#"{"id":"task-1","executors":[],"label":"blue"}"#)
        .create();
    let tes = TES::with_transport(transport(&server, "/generic/tes"))
        .await
        .unwrap();
    let task = LabelledTask {
        task: TesTask::new(vec![TesExecutor::new(
            "alpine".to_owned(),
            vec!["true".to_owned()],
        )]),
        label: Some("blue".to_owned()),
    };

    assert_eq!(tes.create_as(&task).await.unwrap().id, "task-1");
    created.assert();
    let fetched: LabelledTask = tes.get_as(TesView::Basic, "task-1").await.unwrap();
    assert_eq!(fetched.label.as_deref(), Some("blue"));
    let labelled: Labelled = tes.get_as(TesView::Basic, "task-1").await.unwrap();
    assert_eq!(
        (labelled.id.as_str(), labelled.label.as_str()),
        ("task-1", "blue")
    );
}

#[tokio::test]
async fn tes_create_as_rejects_values_that_are_not_tasks() {
    let mut server = mockito::Server::new_async().await;
    let _info = service_info(&mut server, "/generic/invalid", "org.ga4gh", "tes");
    let tes = TES::with_transport(transport(&server, "/generic/invalid"))
        .await
        .unwrap();
    let error = tes.create_as(&json!({"name": 1})).await.unwrap_err();
    assert!(matches!(error, Error::Parse(_)), "{error:?}");
}

#[tokio::test]
async fn wes_and_drs_read_downstream_types() {
    let mut server = mockito::Server::new_async().await;
    let _wes_info = service_info(&mut server, "/generic/wes", "org.ga4gh", "wes");
    let _run = server
        .mock("GET", "/generic/wes/runs/run-1")
        .with_body(r#"{"run_id":"run-1","state":"QUEUED","label":"green"}"#)
        .create();
    let _drs_info = service_info(&mut server, "/generic/drs", "org.ga4gh", "drs");
    let _object = server
        .mock("GET", "/generic/drs/objects/object-1")
        .with_body(r#"{"id":"object-1","label":"red"}"#)
        .create();

    #[derive(Deserialize)]
    struct Run {
        run_id: String,
        label: String,
    }
    let wes = WES::with_transport(transport(&server, "/generic/wes"))
        .await
        .unwrap();
    let run: Run = wes.get_run_as("run-1").await.unwrap();
    assert_eq!(
        (run.run_id.as_str(), run.label.as_str()),
        ("run-1", "green")
    );
    let drs = DRS::with_transport(transport(&server, "/generic/drs"))
        .await
        .unwrap();
    let object: Labelled = drs.get_object_as("object-1").await.unwrap();
    assert_eq!(
        (object.id.as_str(), object.label.as_str()),
        ("object-1", "red")
    );
}

#[tokio::test]
async fn generic_reads_report_parse_and_pending_errors() {
    let mut server = mockito::Server::new_async().await;
    let _tes_info = service_info(&mut server, "/edge/tes", "org.ga4gh", "tes");
    let _task = server
        .mock("GET", "/edge/tes/tasks/task-1?view=BASIC")
        .with_body(r#"{"id":"task-1","executors":[]}"#)
        .create();
    let _run = server
        .mock("GET", "/edge/wes/runs/run-1")
        .with_body(r#"{"run_id":"run-1","label":"green"}"#)
        .create();
    let _drs_info = service_info(&mut server, "/edge/drs", "org.ga4gh", "drs");
    let _pending = server
        .mock("GET", "/edge/drs/objects/object-1")
        .with_status(202)
        .with_header("Retry-After", "1")
        .create();

    // The task has no `label`, which `Labelled` requires.
    let tes = TES::with_transport(transport(&server, "/edge/tes"))
        .await
        .unwrap();
    let error = tes
        .get_as::<Labelled>(TesView::Basic, "task-1")
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Parse(_)), "{error:?}");

    #[derive(Deserialize)]
    struct Run {
        label: String,
    }
    let run =
        ga4gh_sdk::clients::wes::Run::new("run-1".to_owned(), transport(&server, "/edge/wes"));
    assert_eq!(run.log_as::<Run>().await.unwrap().label, "green");

    let drs = DRS::with_transport(transport(&server, "/edge/drs"))
        .await
        .unwrap();
    let error = drs.get_object_as::<Labelled>("object-1").await.unwrap_err();
    assert!(
        error
            .downcast_ref::<ga4gh_sdk::clients::drs::resolution::PendingResolution>()
            .is_some(),
        "{error:?}"
    );
}

#[tokio::test]
async fn clients_accept_the_artifact_they_are_given() {
    let mut server = mockito::Server::new_async().await;
    let _tes = service_info(&mut server, "/artifact/tes", "org.example", "labelled-tes");
    let _wes = service_info(&mut server, "/artifact/wes", "org.example", "labelled-wes");
    let _drs = service_info(&mut server, "/artifact/drs", "org.example", "labelled-drs");
    let created = server
        .mock("POST", "/artifact/tes/tasks")
        .with_body(r#"{"id":"task-2"}"#)
        .create();

    let tes = TES::with_artifact(transport(&server, "/artifact/tes"), "labelled-tes")
        .await
        .unwrap();
    assert_eq!(
        tes.discovered_service_info().r#type.artifact,
        "labelled-tes"
    );
    // Methods no longer check the artifact again.
    let task = TesTask::new(vec![TesExecutor::new(
        "alpine".to_owned(),
        vec!["true".to_owned()],
    )]);
    assert_eq!(tes.create(task).await.unwrap().id, "task-2");
    created.assert();
    let started = server
        .mock("POST", "/artifact/wes/runs")
        .with_body(r#"{"run_id":"run-2"}"#)
        .create();
    let wes = WES::with_artifact(transport(&server, "/artifact/wes"), "labelled-wes")
        .await
        .unwrap();
    let mut request = ga4gh_sdk::clients::wes::models::RunRequest::new(
        "CWL".to_owned(),
        "v1.2".to_owned(),
        "https://example.org/hello.cwl".to_owned(),
    );
    request.workflow_params = Some(json!({}));
    assert_eq!(wes.run_workflow(request).await.unwrap().id, "run-2");
    started.assert();
    // `with_artifact` accepts any group; `with_transport` needs `org.ga4gh`.
    DRS::with_artifact(transport(&server, "/artifact/drs"), "labelled-drs")
        .await
        .unwrap();
}

#[tokio::test]
async fn clients_reject_other_artifacts() {
    let mut server = mockito::Server::new_async().await;
    let _tes = service_info(&mut server, "/other/tes", "org.ga4gh", "tes");
    let _drs = service_info(&mut server, "/other/drs", "org.example", "drs");

    let errors = [
        TES::with_artifact(transport(&server, "/other/tes"), "labelled-tes")
            .await
            .unwrap_err(),
        WES::with_artifact(transport(&server, "/other/tes"), "labelled-wes")
            .await
            .unwrap_err(),
        DRS::with_artifact(transport(&server, "/other/tes"), "labelled-drs")
            .await
            .unwrap_err(),
    ];
    for (error, artifact) in errors
        .iter()
        .zip(["labelled-tes", "labelled-wes", "labelled-drs"])
    {
        assert!(matches!(error, Error::UnexpectedService(_)), "{error:?}");
        assert_eq!(
            error.to_string(),
            format!("The endpoint is not an instance of '{artifact}'")
        );
    }
    // The default artifacts keep their messages, and DRS keeps its group.
    let error = TES::with_artifact(transport(&server, "/other/drs"), "tes")
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "The endpoint is not an instance of TES");
    let error = DRS::with_transport(transport(&server, "/other/drs"))
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "The endpoint is not an instance of DRS");
}
