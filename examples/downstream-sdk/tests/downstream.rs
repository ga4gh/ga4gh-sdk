//! Tests of the example against a mock TES service with extensions.
//!
//! Each test uses its own API root on the shared mock server, so that tests
//! running in parallel don't see each other's mocks.

use downstream_sdk::cli;
use downstream_sdk::task::PrioritizedTask;
use downstream_sdk::usage::{TaskUsage, UsageClient};
use ga4gh_sdk::clients::tes::models::{TesExecutor, TesTask};
use ga4gh_sdk::clients::tes::{TesView, TES};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::HttpError;
use mockito::{mock, Matcher, Mock};
use serde_json::json;
use std::process::ExitCode;
use url::Url;

/// A configuration for the API root `/<root>/ga4gh/tes/v1` on the mock server.
fn config(root: &str) -> Configuration {
    let url = format!("{}/{root}/ga4gh/tes/v1", mockito::server_url());
    Configuration::new(Url::parse(&url).unwrap())
}

/// A TES service-info response beneath `root`.
fn service_info(root: &str) -> Mock {
    mock("GET", format!("/{root}/ga4gh/tes/v1/service-info").as_str())
        .with_status(200)
        .with_body(
            json!({
                "id": "org.example.tes",
                "name": "Example TES",
                "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0"
            })
            .to_string(),
        )
        .create()
}

#[tokio::test]
async fn usage_client_shares_the_tes_clients_transport() {
    let _info = service_info("shared");
    let _usage = mock("GET", "/shared/ga4gh/tes/v1/tasks/task%201/usage")
        .match_header("user-agent", "downstream-test")
        .with_status(200)
        .with_body(r#"{"id":"task 1","cpu_seconds":1.5,"peak_memory_bytes":1024,"node":"n1"}"#)
        .create();
    let config = config("shared").with_user_agent("downstream-test".into());
    let tes = TES::new(&config).await.unwrap();

    let client = UsageClient::from_transport(tes.transport.clone());
    let usage = client.task_usage("task 1").await.unwrap();

    assert_eq!(
        usage,
        TaskUsage {
            id: "task 1".into(),
            cpu_seconds: Some(1.5),
            peak_memory_bytes: Some(1024),
            extra: json!({"node": "n1"}).as_object().unwrap().clone(),
        }
    );
    assert_eq!(client.transport().config.base_path, config.base_path);
}

#[tokio::test]
async fn usage_errors_keep_the_http_status() {
    let _usage = mock("GET", "/missing/ga4gh/tes/v1/tasks/gone/usage")
        .with_status(404)
        .with_body(r#"{"msg":"not found"}"#)
        .create();

    let error = UsageClient::new(&config("missing"))
        .task_usage("gone")
        .await
        .unwrap_err();

    let error = error.downcast_ref::<HttpError>().expect("an HTTP error");
    assert_eq!(error.status.as_u16(), 404);
}

#[tokio::test]
async fn prioritized_task_sends_and_reads_the_extra_field() {
    let _info = service_info("priority");
    let created = mock("POST", "/priority/ga4gh/tes/v1/tasks")
        .match_body(Matcher::Json(json!({
            "name": "hello",
            "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
            "priority": 7,
            "x-other": true
        })))
        .with_status(200)
        .with_body(r#"{"id":"task-7"}"#)
        .create();
    let _fetched = mock("GET", "/priority/ga4gh/tes/v1/tasks/task-7?view=BASIC")
        .with_status(200)
        .with_body(r#"{"id":"task-7","state":"QUEUED","executors":[],"priority":7,"x-other":true}"#)
        .create();
    let tes = TES::new(&config("priority")).await.unwrap();
    let mut spec_task = TesTask::new(vec![TesExecutor::new(
        "alpine".into(),
        vec!["echo".into(), "hello".into()],
    )]);
    spec_task.name = Some("hello".into());
    spec_task.extra.insert("x-other".into(), json!(true));

    let task = PrioritizedTask::new(spec_task).with_priority(7);
    let submitted = task.submit(&tes).await.unwrap();
    created.assert();
    assert_eq!(submitted.id, "task-7");

    let fetched = PrioritizedTask::fetch(&tes, TesView::Basic, "task-7")
        .await
        .unwrap();
    assert_eq!(fetched.priority, Some(7));
    assert_eq!(fetched.task.id.as_deref(), Some("task-7"));
    // `priority` is consumed by the outer model, not left in `extra`.
    assert_eq!(
        fetched.task.extra,
        *json!({"x-other": true}).as_object().unwrap()
    );
}

#[test]
fn cli_keeps_the_built_in_commands_and_adds_one() {
    let command = cli::command();
    assert_eq!(command.get_name(), cli::BIN_NAME);
    let tes = command.find_subcommand("tes").unwrap();
    for name in ["create", "list", "get", "status", "cancel", "usage"] {
        assert!(tes.find_subcommand(name).is_some(), "missing tes {name}");
    }
    for name in ["drs", "trs", "wes", "workflow", "registry"] {
        assert!(command.find_subcommand(name).is_some(), "missing {name}");
    }
    cli::command().debug_assert();
}

#[tokio::test]
async fn cli_runs_the_added_subcommand() {
    let _usage = mock("GET", "/cli-usage/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"id":"task-1\u001b[2J","cpu_seconds":2.0}"#)
        .create();
    let matches = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "usage", "task-1"])
        .unwrap();
    assert_eq!(cli::usage_task_id(&matches), Some("task-1"));
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());

    let exit_code = cli::run_with_config(&matches, &config("cli-usage"), &mut stdout, &mut stderr)
        .await
        .unwrap();

    assert_eq!(exit_code, Some(ExitCode::SUCCESS));
    // The control character from the service is shown escaped.
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "task-1\\u{1b}[2J\t2\t-\n"
    );
    assert!(stderr.is_empty());
}

#[tokio::test]
async fn cli_reports_errors_of_the_added_subcommand() {
    let _usage = mock("GET", "/cli-error/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(500)
        .with_body("broken")
        .create();
    let matches = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "usage", "task-1"])
        .unwrap();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());

    let exit_code = cli::run_with_config(&matches, &config("cli-error"), &mut stdout, &mut stderr)
        .await
        .unwrap();

    assert_eq!(exit_code, Some(ExitCode::FAILURE));
    assert!(stdout.is_empty());
    assert!(String::from_utf8(stderr).unwrap().contains("500"));
}

#[tokio::test]
async fn cli_passes_built_in_commands_to_the_sdk() {
    let _info = service_info("cli-builtin");
    let _tasks = mock("GET", "/cli-builtin/ga4gh/tes/v1/tasks")
        .with_status(200)
        .with_body(r#"{"tasks":[{"id":"task-from-sdk","state":"COMPLETE"}]}"#)
        .create();
    let matches = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "list"])
        .unwrap();
    assert_eq!(cli::usage_task_id(&matches), None);
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());

    let exit_code =
        cli::run_with_config(&matches, &config("cli-builtin"), &mut stdout, &mut stderr)
            .await
            .unwrap();

    assert_eq!(exit_code, Some(ExitCode::SUCCESS));
    assert!(String::from_utf8(stdout).unwrap().contains("task-from-sdk"));
    assert!(stderr.is_empty());
}
