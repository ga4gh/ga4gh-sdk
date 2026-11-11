//! Tests of the example against a mock TES service with extensions.
//!
//! Each test uses its own API root on the shared mock server, so that tests
//! running in parallel don't see each other's mocks.

use downstream_sdk::cli;
use downstream_sdk::task::PrioritizedTask;
use downstream_sdk::usage::{TaskUsage, UsageClient};
use ga4gh_sdk::clients::tes::models::{TesExecutor, TesTask};
use ga4gh_sdk::clients::tes::{TesView, TES};
use ga4gh_sdk::reqwest;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::{HttpError, Transport};
use mockito::{mock, Matcher, Mock};
use serde_json::json;
use std::process::ExitCode;
use url::Url;

/// The API root `/<root>/ga4gh/tes/v1` on the mock server.
fn api_root(root: &str) -> String {
    format!("{}/{root}/ga4gh/tes/v1", mockito::server_url())
}

/// A configuration for the API root `/<root>/ga4gh/tes/v1`.
fn config(root: &str) -> Configuration {
    Configuration::new(Url::parse(&api_root(root)).unwrap())
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

fn example_task() -> TesTask {
    let mut task = TesTask::new(vec![TesExecutor::new(
        "alpine".into(),
        vec!["echo".into(), "hello".into()],
    )]);
    task.name = Some("hello".into());
    task
}

#[tokio::test]
async fn usage_client_shares_the_tes_clients_transport() {
    let _info = service_info("shared");
    // Only the injected HTTP client sends this header, so the usage request
    // proves that the client is shared, not just the configuration.
    let _usage = mock("GET", "/shared/ga4gh/tes/v1/tasks/task%201/usage")
        .match_header("x-client", "injected")
        .with_status(200)
        .with_body(r#"{"id":"task 1","cpu_seconds":1.5,"peak_memory_bytes":1024,"node":"n1"}"#)
        .create();
    let config = config("shared");
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-client", "injected".parse().unwrap());
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    let mut tes = TES::new(&config).await.unwrap();
    tes.transport = Transport::with_client(&config, client);

    let usage = UsageClient::from_transport(tes.transport.clone())
        .task_usage("task 1")
        .await
        .unwrap();

    assert_eq!(
        usage,
        TaskUsage {
            id: "task 1".into(),
            cpu_seconds: Some(1.5),
            peak_memory_bytes: Some(1024),
            extra: json!({"node": "n1"}).as_object().unwrap().clone(),
        }
    );
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
async fn usage_rejects_invalid_bodies() {
    let _usage = mock("GET", "/invalid/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"cpu_seconds":1}"#)
        .create();

    let error = UsageClient::new(&config("invalid"))
        .task_usage("task-1")
        .await
        .unwrap_err();

    assert!(error.is::<serde_json::Error>(), "{error}");
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
    let mut spec_task = example_task();
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
fn prioritized_task_writes_priority_once_or_not_at_all() {
    let mut spec_task = example_task();
    spec_task.extra.insert("priority".into(), json!(3));
    let task = PrioritizedTask::new(spec_task.clone()).with_priority(9);
    let text = serde_json::to_string(&task).unwrap();
    assert_eq!(text.matches("\"priority\"").count(), 1, "{text}");
    assert!(text.contains("\"priority\":9"), "{text}");

    spec_task.extra.clear();
    let without = serde_json::to_value(PrioritizedTask::new(spec_task)).unwrap();
    assert!(without.get("priority").is_none(), "{without}");
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

    let error = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "usage"])
        .unwrap_err();
    assert_eq!(
        error.kind(),
        clap::error::ErrorKind::MissingRequiredArgument
    );
}

#[tokio::test]
async fn cli_extension_escapes_output_and_reports_errors() {
    let _usage = mock("GET", "/cli-usage/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"id":"task-1\u001b[2J","cpu_seconds":2.0}"#)
        .create();
    let _error = mock("GET", "/cli-usage/ga4gh/tes/v1/tasks/task-2/usage")
        .with_status(500)
        .with_body("broken")
        .create();
    let config = config("cli-usage");

    let matches = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "usage", "task-1"])
        .unwrap();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = cli::run_extension(&matches, &config, &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(exit_code, Some(ExitCode::SUCCESS));
    // The control character from the service is shown escaped.
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "task-1\\u{1b}[2J\t2\t-\n"
    );
    assert!(stderr.is_empty());

    let matches = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "usage", "task-2"])
        .unwrap();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = cli::run_extension(&matches, &config, &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(exit_code, Some(ExitCode::FAILURE));
    assert!(stdout.is_empty());
    assert!(String::from_utf8(stderr).unwrap().contains("500"));

    // Commands of the SDK are left to the SDK.
    let matches = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "tes", "list"])
        .unwrap();
    let exit_code = cli::run_extension(&matches, &config, &mut Vec::new(), &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(exit_code, None);
}

/// The binary's entry point, with the configuration file in a temporary
/// home directory. The only test that reads `HOME`.
#[tokio::test]
async fn cli_run_reads_the_config_file_for_added_and_built_in_commands() {
    let _info = service_info("cli-run");
    let _usage = mock("GET", "/cli-run/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"id":"task-1","peak_memory_bytes":5}"#)
        .create();
    let _tasks = mock("GET", "/cli-run/ga4gh/tes/v1/tasks")
        .with_status(200)
        .with_body(r#"{"tasks":[{"id":"task-from-sdk","state":"COMPLETE"}]}"#)
        .create();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    std::fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        json!({"TES": {"base_path": api_root("cli-run")}}).to_string(),
    )
    .unwrap();
    std::env::set_var("HOME", home.path());

    for (args, expected) in [
        (vec!["tes", "usage", "task-1"], "task-1\t-\t5\n"),
        (vec!["tes", "list"], "task-from-sdk"),
    ] {
        let matches = cli::command()
            .try_get_matches_from([cli::BIN_NAME].into_iter().chain(args.clone()))
            .unwrap();
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let exit_code = cli::run(&matches, &mut stdout, &mut stderr).await.unwrap();
        let stdout = String::from_utf8(stdout).unwrap();
        assert_eq!(exit_code, ExitCode::SUCCESS, "{args:?}: {stdout}");
        assert!(stdout.contains(expected), "{args:?}: {stdout}");
        assert!(stderr.is_empty(), "{args:?}");
    }
}
