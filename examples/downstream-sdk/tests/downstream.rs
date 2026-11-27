//! Tests of the example against a mock TES service with extensions.
//!
//! Each test uses its own API root on the shared mock server, so that tests
//! running in parallel don't see each other's mocks. The binary test sets
//! `HOME` and the token variable only in its child process.

use downstream_sdk::cli;
use downstream_sdk::task::PrioritizedTask;
use downstream_sdk::usage::{TaskUsage, UsageClient};
use ga4gh_sdk::clients::tes::models::{TesExecutor, TesTask};
use ga4gh_sdk::clients::tes::{TesView, TES};
use ga4gh_sdk::reqwest;
use ga4gh_sdk::utils::configuration::{BasicAuth, Configuration};
use ga4gh_sdk::utils::transport::{HttpError, Transport};
use mockito::{Matcher, Mock};
use serde_json::json;
#[cfg(unix)]
use std::process::Command;
use std::process::ExitCode;
use url::Url;

#[test]
fn native_extensions_use_explicit_downstream_paths() {
    use ga4gh_sdk::utils::extension_manager::ExtensionManager;

    fn send_sync<T: Send + Sync>() {}
    send_sync::<ExtensionManager>();
    send_sync::<ga4gh_sdk::utils::extension::ResolvedExtension>();
    send_sync::<ga4gh_sdk::utils::extension::LoadedModule>();
    send_sync::<ga4gh_sdk::utils::extension::ExtensionInstance>();
    send_sync::<ga4gh_sdk::utils::extension::ExtensionMethod>();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut config =
        Configuration::from_provider(figment::providers::Serialized::defaults(json!({
            "base_path":"https://service.example.org/",
            "extensions":[
                {"name":"optional","required":false,"configuration":{}}
            ]
        })))
        .unwrap();
    // SAFETY: this registry is absent, so no native code can be loaded.
    let manager = ExtensionManager::with_paths(&path, dir.path().join("native-artifacts")).unwrap();
    let instances = unsafe { manager.from_configuration(&config) }.unwrap();
    assert!(instances.is_empty());
    config.extra["extensions"][0]["required"] = json!(true);
    let error = unsafe { manager.from_configuration(&config) }.unwrap_err();
    assert_eq!(
        error.kind,
        ga4gh_sdk::utils::extension::ExtensionErrorKind::Missing
    );
    assert!(!path.exists());
}

/// The API root `/<root>/ga4gh/tes/v1` on the mock server.
fn api_root(server: &mockito::ServerGuard, root: &str) -> String {
    format!("{}/{root}/ga4gh/tes/v1", server.url())
}

/// A configuration for the API root `/<root>/ga4gh/tes/v1`.
fn config(server: &mockito::ServerGuard, root: &str) -> Configuration {
    Configuration::new(Url::parse(&api_root(server, root)).unwrap())
}

/// The body of a TES service-info response.
fn service_info_body() -> String {
    json!({
        "id": "org.example.tes",
        "name": "Example TES",
        "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0"
    })
    .to_string()
}

/// A TES service-info response beneath `root`.
fn service_info(server: &mut mockito::ServerGuard, root: &str) -> Mock {
    server
        .mock("GET", format!("/{root}/ga4gh/tes/v1/service-info").as_str())
        .with_status(200)
        .with_body(service_info_body())
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
    let mut server = mockito::Server::new_async().await;
    // Only the injected HTTP client sends this header, so the service-info
    // and usage requests prove that the client is shared, not just the
    // configuration.
    let info = server
        .mock("GET", "/shared/ga4gh/tes/v1/service-info")
        .match_header("x-client", "injected")
        .with_status(200)
        .with_body(service_info_body())
        .expect(1)
        .create();
    let _usage = server
        .mock("GET", "/shared/ga4gh/tes/v1/tasks/task%201/usage")
        .match_header("x-client", "injected")
        .with_status(200)
        .with_body(r#"{"id":"task 1","cpu_seconds":1.5,"peak_memory_bytes":1024,"node":"n1"}"#)
        .create();
    let config = config(&server, "shared");
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-client", "injected".parse().unwrap());
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    let tes = TES::with_transport(Transport::with_client(&config, client))
        .await
        .unwrap();
    info.assert();
    assert_eq!(tes.configuration().base_path, config.base_path);
    assert_eq!(tes.discovered_service_info().id, "org.example.tes");

    let usage = UsageClient::from_transport(tes.transport().clone())
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
async fn usage_client_shares_configured_auth_with_tes() {
    let mut server = mockito::Server::new_async().await;
    const AUTH: &str = "Basic ZGVtbzpzZWNyZXQ=";
    let info = server
        .mock("GET", "/auth/ga4gh/tes/v1/service-info")
        .match_header("authorization", AUTH)
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
        .create();
    let usage_request = server
        .mock("GET", "/auth/ga4gh/tes/v1/tasks/task-1/usage")
        .match_header("authorization", AUTH)
        .with_body(r#"{"id":"task-1","cpu_seconds":2.0}"#)
        .create();
    let config = config(&server, "auth").with_basic_auth(BasicAuth {
        username: "demo".into(),
        password: Some("secret".into()),
    });
    let tes = TES::new(&config).await.unwrap();
    info.assert();

    let usage = UsageClient::from_transport(tes.transport().clone())
        .task_usage("task-1")
        .await
        .unwrap();
    assert_eq!(usage.cpu_seconds, Some(2.0));
    usage_request.assert();
}

#[tokio::test]
async fn usage_errors_keep_the_http_status() {
    let mut server = mockito::Server::new_async().await;
    let _usage = server
        .mock("GET", "/missing/ga4gh/tes/v1/tasks/gone/usage")
        .with_status(404)
        .with_body(r#"{"msg":"not found"}"#)
        .create();

    let error = UsageClient::new(&config(&server, "missing"))
        .task_usage("gone")
        .await
        .unwrap_err();

    match error {
        ga4gh_sdk::Error::Http(HttpError { status, .. }) => assert_eq!(status.as_u16(), 404),
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

#[tokio::test]
async fn usage_rejects_invalid_bodies() {
    let mut server = mockito::Server::new_async().await;
    let _usage = server
        .mock("GET", "/invalid/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"cpu_seconds":1}"#)
        .create();

    let error = UsageClient::new(&config(&server, "invalid"))
        .task_usage("task-1")
        .await
        .unwrap_err();

    assert!(matches!(error, ga4gh_sdk::Error::Parse(_)), "{error}");
}

#[tokio::test]
async fn prioritized_task_sends_and_reads_the_extra_field() {
    let mut server = mockito::Server::new_async().await;
    let _info = service_info(&mut server, "priority");
    let created = server
        .mock("POST", "/priority/ga4gh/tes/v1/tasks")
        .match_body(Matcher::Json(json!({
            "name": "hello",
            "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
            "priority": 7,
            "x-other": true
        })))
        .with_status(200)
        .with_body(r#"{"id":"task-7"}"#)
        .create();
    let _fetched = server
        .mock("GET", "/priority/ga4gh/tes/v1/tasks/task-7?view=BASIC")
        .with_status(200)
        .with_body(r#"{"id":"task-7","state":"QUEUED","executors":[],"priority":7,"x-other":true}"#)
        .create();
    let tes = TES::new(&config(&server, "priority")).await.unwrap();
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

#[tokio::test]
async fn tes_client_accepts_the_artifact_of_the_extended_service() {
    let mut server = mockito::Server::new_async().await;
    let _info = server
        .mock("GET", "/artifact/ga4gh/tes/v1/service-info")
        .with_status(200)
        .with_body(
            json!({
                "id": "org.example.prioritized",
                "name": "Example TES with priorities",
                "type": {"group": "org.example", "artifact": "prioritized-tes", "version": "1.1.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0"
            })
            .to_string(),
        )
        .create();
    let created = server
        .mock("POST", "/artifact/ga4gh/tes/v1/tasks")
        .match_body(Matcher::PartialJson(json!({"priority": 2})))
        .with_status(200)
        .with_body(r#"{"id":"task-2"}"#)
        .create();
    let config = config(&server, "artifact");

    let error = TES::new(&config).await.unwrap_err();
    assert!(
        matches!(error, ga4gh_sdk::Error::UnexpectedService(_)),
        "{error:?}"
    );

    let tes = TES::with_artifact(Transport::new(&config), "prioritized-tes")
        .await
        .unwrap();
    let task = PrioritizedTask::new(example_task()).with_priority(2);
    assert_eq!(task.submit(&tes).await.unwrap().id, "task-2");
    created.assert();
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
fn cli_keeps_the_built_in_commands_and_adds_its_own() {
    let command = cli::command();
    assert_eq!(command.get_name(), cli::BIN_NAME);
    let tes = command.find_subcommand("tes").unwrap();
    for name in [
        "create", "list", "get", "status", "cancel", "files", "log", "save", "usage",
    ] {
        assert!(tes.find_subcommand(name).is_some(), "missing tes {name}");
    }
    for name in [
        "drs",
        "trs",
        "wes",
        "workflow",
        "registry",
        "service-info",
        "login",
        "logout",
        "quota",
    ] {
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
    let version = cli::command()
        .try_get_matches_from([cli::BIN_NAME, "--version"])
        .unwrap_err()
        .to_string();
    assert_eq!(
        version,
        format!("{} {}\n", cli::BIN_NAME, env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(cli::context().token_env(), Some(cli::TOKEN_ENV));
}

#[tokio::test]
async fn quota_uses_the_sdk_output_helpers() {
    let mut server = mockito::Server::new_async().await;
    let _quota = server
        .mock("GET", "/quota-ok/quota")
        .with_body("{\"user\":\"a\u{202e}\",\"cpu_hours\":10}")
        .create();
    let _error = server
        .mock("GET", "/quota-error/quota")
        .with_status(403)
        .with_body("denied")
        .create();
    let quota_config =
        |root: &str| Configuration::new(Url::parse(&format!("{}/{root}/", server.url())).unwrap());
    let output = cli::quota(&quota_config("quota-ok")).await;
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(output.stdout, "{\"user\":\"a\\u202e\",\"cpu_hours\":10}");
    let output = cli::quota(&quota_config("quota-error")).await;
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.contains("403"), "{}", output.stderr);
}

#[tokio::test]
async fn cli_extension_escapes_output_and_reports_errors() {
    let mut server = mockito::Server::new_async().await;
    let _usage = server
        .mock("GET", "/cli-usage/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"id":"task-1\u001b[2J","cpu_seconds":2.0}"#)
        .create();
    let _error = server
        .mock("GET", "/cli-usage/ga4gh/tes/v1/tasks/task-2/usage")
        .with_status(500)
        .with_body("broken")
        .create();
    let config = config(&server, "cli-usage");

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

/// Run the binary with a temporary home, leaving the test process's HOME alone.
#[cfg(unix)]
#[test]
fn cli_binary_reads_the_config_file_for_added_and_built_in_commands() {
    let mut server = mockito::Server::new();
    let _info = service_info(&mut server, "cli-run");
    let _usage = server
        .mock("GET", "/cli-run/ga4gh/tes/v1/tasks/task-1/usage")
        .with_status(200)
        .with_body(r#"{"id":"task-1","peak_memory_bytes":5}"#)
        .create();
    let _tasks = server
        .mock("GET", "/cli-run/ga4gh/tes/v1/tasks")
        .with_status(200)
        .with_body(r#"{"tasks":[{"id":"task-from-sdk","state":"COMPLETE"}]}"#)
        .create();
    let _quota = server
        .mock("GET", "/cli-run-quota/quota")
        .match_header("authorization", "Bearer from-file")
        .with_body(r#"{"cpu_hours":3}"#)
        .create();
    // The built-in `service-info` command reads the binary's own sections too.
    let _quota_info = server
        .mock("GET", "/cli-run-quota/service-info")
        .match_header("authorization", "Bearer from-file")
        .with_body(
            json!({
                "id": "org.example.quota",
                "name": "Example quota",
                "type": {"group": "org.example", "artifact": "quota", "version": "1.0.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0",
                "x-limit": 3
            })
            .to_string(),
        )
        .create();
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join(".downstream-cli")).unwrap();
    std::fs::write(
        home.path().join(".downstream-cli/config.json"),
        json!({
            "TES": {"base_path": api_root(&server, "cli-run")},
            "QUOTA": {"base_path": format!("{}/cli-run-quota/", server.url()), "oauth_access_token": "from-file"}
        }).to_string(),
    )
    .unwrap();
    std::fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    std::fs::write(home.path().join(".ga4gh-cli/config.json"), "{}").unwrap();
    for (args, expected) in [
        (vec!["tes", "usage", "task-1"], "task-1\t-\t5\n"),
        (vec!["tes", "list"], "task-from-sdk"),
        (vec!["quota"], "{\"cpu_hours\":3}"),
        (
            vec!["service-info", "--service", cli::QUOTA_SECTION],
            "\"x-limit\":3",
        ),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_downstream-cli"))
            .args(&args)
            .env("HOME", home.path())
            .env(cli::TOKEN_ENV, "from-env")
            .output()
            .unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(stdout.contains(expected), "{args:?}: {stdout}");
        assert!(output.stderr.is_empty(), "{args:?}: {stdout}");
    }
    let output = Command::new(env!("CARGO_BIN_EXE_downstream-cli"))
        .args(["tes", "list"])
        .env("HOME", home.path())
        .env_remove(cli::TOKEN_ENV)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains(cli::TOKEN_ENV));
}

#[cfg(unix)]
#[test]
fn cli_binary_selects_a_profile_for_its_own_command() {
    let mut server = mockito::Server::new();
    let quota = server
        .mock("GET", "/cli-run-metered-quota/quota")
        .match_header("authorization", "Bearer metered-file-token")
        .with_body(r#"{"cpu_hours":7}"#)
        .create();
    let home = tempfile::tempdir().unwrap();
    let config_dir = home.path().join(".downstream-cli");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.json"),
        json!({
            "profiles": {
                "default": {"TES": {"base_path": api_root(&server, "unused")}},
                "metered": {"QUOTA": {
                    "base_path": format!("{}/cli-run-metered-quota/", server.url()),
                    "oauth_access_token": "metered-file-token"
                }}
            }
        })
        .to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_downstream-cli"))
        .args(["--profile", "metered", "quota"])
        .env("HOME", home.path())
        .env_remove(cli::TOKEN_ENV)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        r#"{"cpu_hours":7}"#
    );
    quota.assert();
}

#[tokio::test]
async fn layered_configuration_drives_a_downstream_authenticated_request() {
    use figment::providers::Serialized;
    use serde::Deserialize;
    #[derive(Deserialize)]
    struct Routing {
        queue: String,
    }
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("GET", "/custom/tasks/task-1/usage")
        .match_header("authorization", "Bearer downstream-token")
        .with_body(r#"{"id":"task-1","cpu_seconds":4}"#)
        .create_async()
        .await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("custom.json");
    std::fs::write(
        &path,
        json!({"profiles":{"site":{"CUSTOM":{
            "base_path":"https://unused.example.org/", "routing":{"queue":"batch"}
        }}}})
        .to_string(),
    )
    .unwrap();
    let sources = Configuration::sources(&path, Some("site"), "CUSTOM")
        .unwrap()
        .merge(Serialized::defaults(json!({
            "base_path":format!("{}/custom/", server.url()),
            "bearer_access_token":"downstream-token"
        })));
    let config = Configuration::from_provider(sources).unwrap();
    assert_eq!(
        config
            .extra_as::<Routing>("routing")
            .unwrap()
            .unwrap()
            .queue,
        "batch"
    );
    assert_eq!(
        UsageClient::new(&config)
            .task_usage("task-1")
            .await
            .unwrap()
            .cpu_seconds,
        Some(4.0)
    );
    request.assert_async().await;
}

#[test]
fn json_store_supports_downstream_locked_updates() {
    use ga4gh_sdk::utils::json_store::{JsonStore, JsonStoreIoError};

    #[derive(Debug, PartialEq)]
    struct RejectedChange;

    impl std::fmt::Display for RejectedChange {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("application rejected the change")
        }
    }

    impl std::error::Error for RejectedChange {}

    fn rejected_change(items: &mut Vec<String>) -> Result<(), RejectedChange> {
        items.push("rejected item".into());
        Err(RejectedChange)
    }

    let dir = tempfile::tempdir().unwrap();
    let store = JsonStore::new(dir.path().join("state.json"), "application state");
    let count = store
        .update::<Vec<String>, _>(|items| {
            items.push("new item".into());
            Ok(items.len())
        })
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        store.read::<Vec<String>>().unwrap(),
        Some(vec!["new item".into()])
    );

    let before = std::fs::read(store.path()).unwrap();
    let error = store
        .update::<Vec<String>, _>(|items| rejected_change(items).map_err(ga4gh_sdk::Error::other))
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<RejectedChange>(),
        Some(&RejectedChange)
    );
    assert_eq!(std::fs::read(store.path()).unwrap(), before);

    let invalid = JsonStore::new(dir.path().join("invalid\0.json"), "application state");
    let error = invalid.read::<Vec<String>>().unwrap_err();
    assert!(!error
        .downcast_ref::<JsonStoreIoError>()
        .unwrap()
        .replacement_completed());
}
