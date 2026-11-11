//! A downstream binary with its own name, version, configuration file and
//! token variable, built with `CliContext`.
//!
//! Each test uses its own environment variable and API root on the shared
//! mock server, so that tests running in parallel don't affect each other.
//! No test here changes `HOME`.

use clap::Command;
use ga4gh_cli::{CliContext, CommandOutput};
use ga4gh_sdk::clients::ServiceType;
use mockito::mock;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Writes `document` as `config.json` into `dir` and returns its path.
fn write_config(dir: &Path, document: serde_json::Value) -> PathBuf {
    let path = dir.join("config.json");
    std::fs::write(&path, document.to_string()).unwrap();
    path
}

fn context(config_path: &Path) -> CliContext {
    CliContext::new("other-cli", "9.8.7")
        .with_about("Another CLI")
        .with_config_path(config_path)
}

#[test]
fn command_has_the_binary_name_version_and_built_in_commands() {
    let command = context(Path::new("unused.json")).command();
    assert_eq!(command.get_name(), "other-cli");
    assert_eq!(command.get_bin_name(), Some("other-cli"));
    assert_eq!(command.get_version(), Some("9.8.7"));
    assert_eq!(
        command.get_about().map(ToString::to_string).as_deref(),
        Some("Another CLI")
    );
    for name in ["drs", "tes", "trs", "wes", "workflow", "registry"] {
        assert!(command.find_subcommand(name).is_some(), "missing {name}");
    }
    command.clone().debug_assert();

    let error = command
        .try_get_matches_from(["other-cli", "--version"])
        .unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::DisplayVersion);
    assert_eq!(error.to_string(), "other-cli 9.8.7\n");
}

#[test]
fn default_context_is_the_ga4gh_cli() {
    let context = CliContext::default();
    assert_eq!(context.name(), "ga4gh-cli");
    assert_eq!(context.version(), env!("CARGO_PKG_VERSION"));
    assert_eq!(context.token_env(), None);
    let command = context.command();
    assert_eq!(command.get_name(), ga4gh_cli::command().get_name());
    assert_eq!(command.get_version(), ga4gh_cli::command().get_version());
    if let Some(home) = dirs::home_dir() {
        assert_eq!(
            context.config_path().unwrap(),
            home.join(".ga4gh-cli/config.json")
        );
    }
}

#[test]
fn config_path_defaults_to_a_directory_named_after_the_binary() {
    if let Some(home) = dirs::home_dir() {
        let context = CliContext::new("other-cli", "1.0.0");
        assert_eq!(
            context.config_path().unwrap(),
            home.join(".other-cli/config.json")
        );
    }
    for name in ["", ".", "..", "a/b", "a\\b"] {
        let error = CliContext::new(name, "1.0.0").config_path().unwrap_err();
        assert!(
            error.to_string().contains("set a configuration path"),
            "{error}"
        );
    }
    // An explicit path doesn't depend on the name.
    let context = CliContext::new("a/b", "1.0.0").with_config_path("/etc/x.json");
    assert_eq!(context.config_path().unwrap(), Path::new("/etc/x.json"));
}

#[test]
fn load_config_reads_the_configured_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        json!({
            "TES": {"base_path": "https://tes.example.org/ga4gh/tes/v1"},
            "MY_SERVICE": {"base_path": "https://my.example.org/", "timeout": 5}
        }),
    );
    let context = context(&path);

    let tes = context.load_config(ServiceType::TES).unwrap();
    assert_eq!(
        tes.base_path.as_str(),
        "https://tes.example.org/ga4gh/tes/v1"
    );
    let custom = context
        .load_config(ServiceType::Custom("MY_SERVICE".into()))
        .unwrap();
    assert_eq!(custom.extra.get("timeout"), Some(&json!(5)));

    let error = context.load_config(ServiceType::WES).unwrap_err();
    assert!(error.to_string().contains("'WES'"), "{error}");
    let error = missing_file_context(dir.path())
        .load_config(ServiceType::TES)
        .unwrap_err();
    assert!(
        error.to_string().contains("missing.json"),
        "the error names the file: {error}"
    );
}

/// A context whose configuration file doesn't exist.
fn missing_file_context(dir: &Path) -> CliContext {
    context(&dir.join("missing.json"))
}

#[test]
fn token_variable_fills_sections_without_credentials() {
    const VAR: &str = "GA4GH_CLI_TEST_CONTEXT_TOKEN_FILLS";
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        json!({
            "TES": {"base_path": "https://tes.example.org/"},
            "WES": {"base_path": "https://wes.example.org/", "oauth_access_token": "from-file"},
            "DRS": {"base_path": "https://drs.example.org/", "basic_auth": {"username": "user"}}
        }),
    );
    std::env::set_var(VAR, "from-env");
    let context = context(&path).with_token_env(VAR);
    assert_eq!(context.token_env(), Some(VAR));

    let tes = context.load_config(ServiceType::TES).unwrap();
    assert_eq!(tes.oauth_access_token.as_deref(), Some("from-env"));
    // Credentials in the section take precedence over the variable.
    let wes = context.load_config(ServiceType::WES).unwrap();
    assert_eq!(wes.oauth_access_token.as_deref(), Some("from-file"));
    let drs = context.load_config(ServiceType::DRS).unwrap();
    assert_eq!(drs.oauth_access_token, None);
    assert!(drs.basic_auth.is_some());
    // The debug output still hides the token.
    assert!(!format!("{tes:?}").contains("from-env"));
}

#[test]
fn unset_or_empty_token_variable_leaves_the_section_unauthenticated() {
    const UNSET: &str = "GA4GH_CLI_TEST_CONTEXT_TOKEN_UNSET";
    const EMPTY: &str = "GA4GH_CLI_TEST_CONTEXT_TOKEN_EMPTY";
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        json!({"TES": {"base_path": "https://tes.example.org/"}}),
    );
    std::env::remove_var(UNSET);
    std::env::set_var(EMPTY, "");
    for var in [UNSET, EMPTY] {
        let config = context(&path)
            .with_token_env(var)
            .load_config(ServiceType::TES)
            .unwrap();
        assert_eq!(config.oauth_access_token, None, "{var}");
    }
    let config = context(&path).load_config(ServiceType::TES).unwrap();
    assert_eq!(config.oauth_access_token, None);
}

#[cfg(unix)]
#[test]
fn non_unicode_token_variable_is_an_error_without_its_value() {
    use std::os::unix::ffi::OsStrExt;
    const VAR: &str = "GA4GH_CLI_TEST_CONTEXT_TOKEN_NOT_UNICODE";
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        json!({"TES": {"base_path": "https://tes.example.org/"}}),
    );
    std::env::set_var(VAR, std::ffi::OsStr::from_bytes(b"SENTINEL\xff"));
    let error = context(&path)
        .with_token_env(VAR)
        .load_config(ServiceType::TES)
        .unwrap_err()
        .to_string();
    assert!(error.contains(VAR), "{error}");
    assert!(error.contains("not valid Unicode"), "{error}");
    assert!(!error.contains("SENTINEL"), "{error}");
}

#[tokio::test]
async fn dispatch_to_reads_the_configured_file() {
    let _service_info = mock("GET", "/context/ga4gh/tes/v1/service-info")
        .with_status(200)
        .with_body(r#"{"id":"test","name":"Test","type":{"group":"org.ga4gh","artifact":"tes","version":"1.1.0"},"organization":{"name":"Test","url":"https://example.org"},"version":"1.0.0"}"#)
        .create();
    let _tasks = mock("GET", "/context/ga4gh/tes/v1/tasks")
        .with_status(200)
        .with_body(r#"{"tasks":[{"id":"task-from-context","state":"COMPLETE"}]}"#)
        .create();
    let dir = tempfile::tempdir().unwrap();
    let base_path = format!("{}/context/ga4gh/tes/v1", mockito::server_url());
    let path = write_config(dir.path(), json!({"TES": {"base_path": base_path}}));
    let context = context(&path);
    let matches = context
        .command()
        .try_get_matches_from(["other-cli", "tes", "list"])
        .unwrap();

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = context
        .dispatch_to(&matches, &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(exit_code, Some(ExitCode::SUCCESS));
    assert!(String::from_utf8(stdout)
        .unwrap()
        .contains("task-from-context"));
    assert!(stderr.is_empty());

    let output = context.dispatch(&matches).await.unwrap().unwrap();
    assert!(output.stdout.contains("task-from-context"));
}

#[tokio::test]
async fn added_commands_return_none_without_reading_the_file() {
    let dir = tempfile::tempdir().unwrap();
    // The file doesn't exist, so reading it would fail.
    let context = missing_file_context(dir.path());
    let matches = context
        .command()
        .subcommand(Command::new("custom"))
        .try_get_matches_from(["other-cli", "custom"])
        .unwrap();
    assert!(context.dispatch(&matches).await.unwrap().is_none());
    let exit_code = context
        .dispatch_to(&matches, &mut Vec::new(), &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(exit_code, None);

    let matches = context
        .command()
        .try_get_matches_from(["other-cli", "tes", "list"])
        .unwrap();
    let error = context
        .dispatch(&matches)
        .await
        .err()
        .expect("the file is missing");
    assert!(error.to_string().contains("missing.json"), "{error}");
}

#[test]
fn output_helpers_are_public() {
    let output = CommandOutput::success("done\n".into());
    assert_eq!(output.stdout, "done\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_code, ExitCode::SUCCESS);

    let output = CommandOutput::failure("bad\u{1b}[2J");
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, "bad\\u{1b}[2J\n");
    assert_eq!(output.exit_code, ExitCode::FAILURE);

    // JSON allows raw bidirectional controls in strings; they are escaped.
    let output = CommandOutput::json("{\"id\":\"a\u{202e}\"}".into());
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(output.stdout, "{\"id\":\"a\\u202e\"}");
    let output = CommandOutput::json("not json".into());
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stderr.contains("not valid JSON"));
}

#[test]
fn write_to_writes_both_streams_and_returns_the_exit_code() {
    let output = CommandOutput {
        stdout: "out".into(),
        stderr: "err".into(),
        exit_code: ExitCode::FAILURE,
    };
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = output.write_to(&mut stdout, &mut stderr).unwrap();
    assert_eq!(exit_code, ExitCode::FAILURE);
    assert_eq!(stdout, b"out");
    assert_eq!(stderr, b"err");
}

/// `workflow submit` is the one command that sends a configured token, so it
/// shows that the token variable reaches the service.
#[tokio::test]
async fn workflow_submission_uses_the_context_file_and_token_variable() {
    const VAR: &str = "GA4GH_CLI_TEST_CONTEXT_TOKEN_WORKFLOW";
    let info = json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let service = mock("GET", "/context-wes/service-info")
        .match_header("authorization", "Bearer token-from-variable")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = mock("POST", "/context-wes/runs")
        .match_header("authorization", "Bearer token-from-variable")
        .with_body(r#"{"run_id":"context-run"}"#)
        .expect(1)
        .create();
    let dir = tempfile::tempdir().unwrap();
    let base_path = format!("{}/context-wes", mockito::server_url());
    let path = write_config(dir.path(), json!({"WES": {"base_path": base_path}}));
    std::env::set_var(VAR, "token-from-variable");
    let context = context(&path).with_token_env(VAR);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let matches = context
        .command()
        .try_get_matches_from([
            "other-cli",
            "workflow",
            "submit",
            schema.to_str().unwrap(),
            request.to_str().unwrap(),
            "public-qc@1",
            "--profile-revision",
            pin,
            "--vocabulary-revision",
            pin,
        ])
        .unwrap();

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = context
        .dispatch_to(&matches, &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(
        exit_code,
        Some(ExitCode::SUCCESS),
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(response["run_id"], "context-run");
    service.assert();
    submission.assert();
}
