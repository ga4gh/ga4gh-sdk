//! Profiles of the configuration file: `--profile`, the profile variable and
//! the file's `default_profile`.
//!
//! Each test uses its own environment variable and API root on the shared
//! mock server, so that tests running in parallel don't affect each other.
//! No test here changes `HOME` or the default profile variable.

use ga4gh_cli::CliContext;
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

/// A file with the profiles `local` (the default), `remote` and `default`.
fn profiles(dir: &Path) -> PathBuf {
    write_config(
        dir,
        json!({
            "default_profile": "local",
            "profiles": {
                "local": {"TES": {"base_path": "http://localhost:8000/ga4gh/tes/v1"}},
                "remote": {"TES": {"base_path": "https://tes.example.org/ga4gh/tes/v1"}},
                "default": {"TES": {"base_path": "https://default.example.org/"}}
            }
        }),
    )
}

/// A context for `path` whose profile variable is `var`.
fn context(path: &Path, var: &str) -> CliContext {
    CliContext::new("other-cli", "1.0.0")
        .with_config_path(path)
        .with_profile_env(var)
}

fn tes_base_path(context: &CliContext) -> String {
    context
        .load_config(ServiceType::TES)
        .unwrap()
        .base_path
        .to_string()
}

#[test]
fn only_the_default_context_has_a_profile_variable() {
    assert_eq!(
        CliContext::default().profile_env(),
        Some("GA4GH_CLI_PROFILE")
    );
    assert_eq!(
        CliContext::new("downstream-cli", "1.0.0").profile_env(),
        None
    );
    let context = CliContext::new("my-cli", "1.0.0").with_profile_env("OTHER");
    assert_eq!(context.profile_env(), Some("OTHER"));
    assert_eq!(context.without_profile_env().profile_env(), None);
    assert_eq!(CliContext::default().profile(), None);
}

#[test]
fn profile_is_selected_by_name_then_variable_then_file() {
    const VAR: &str = "GA4GH_CLI_TEST_PROFILE_PRECEDENCE";
    let dir = tempfile::tempdir().unwrap();
    let path = profiles(dir.path());

    std::env::remove_var(VAR);
    assert_eq!(
        tes_base_path(&context(&path, VAR)),
        "http://localhost:8000/ga4gh/tes/v1"
    );
    std::env::set_var(VAR, "");
    let error = context(&path, VAR).selected_profile().unwrap_err();
    assert!(error.to_string().contains("empty profile name"), "{error}");
    std::env::set_var(VAR, "default");
    assert_eq!(
        tes_base_path(&context(&path, VAR)),
        "https://default.example.org/"
    );
    let selected = context(&path, VAR).with_profile("remote");
    assert_eq!(selected.profile(), Some("remote"));
    assert_eq!(
        selected.selected_profile().unwrap().as_deref(),
        Some("remote")
    );
    assert_eq!(
        tes_base_path(&selected),
        "https://tes.example.org/ga4gh/tes/v1"
    );
    // Without a profile variable, the variable's value doesn't count.
    assert_eq!(
        tes_base_path(&context(&path, VAR).without_profile_env()),
        "http://localhost:8000/ga4gh/tes/v1"
    );
}

#[test]
fn unknown_profile_is_an_error_naming_the_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let path = profiles(dir.path());
    let error = context(&path, "GA4GH_CLI_TEST_PROFILE_UNKNOWN")
        .with_profile("staging")
        .load_config(ServiceType::TES)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "Profile 'staging' not found; profiles in the configuration file: default, local, remote"
    );
}

#[test]
fn files_without_profiles_are_the_profile_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        json!({"TES": {"base_path": "https://tes.example.org/"}}),
    );
    let context = context(&path, "GA4GH_CLI_TEST_PROFILE_FLAT");
    assert_eq!(tes_base_path(&context), "https://tes.example.org/");
    assert_eq!(
        tes_base_path(&context.clone().with_profile("default")),
        "https://tes.example.org/"
    );
    let error = context
        .with_profile("remote")
        .load_config(ServiceType::TES)
        .unwrap_err();
    assert!(
        error.to_string().contains("no 'profiles' object"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn non_unicode_profile_variable_is_an_error_without_its_value() {
    use std::os::unix::ffi::OsStrExt;
    const VAR: &str = "GA4GH_CLI_TEST_PROFILE_NOT_UNICODE";
    let dir = tempfile::tempdir().unwrap();
    let path = profiles(dir.path());
    std::env::set_var(VAR, std::ffi::OsStr::from_bytes(b"SENTINEL\xff"));
    let error = context(&path, VAR)
        .load_config(ServiceType::TES)
        .unwrap_err()
        .to_string();
    assert!(error.contains(VAR), "{error}");
    assert!(error.contains("not valid Unicode"), "{error}");
    assert!(!error.contains("SENTINEL"), "{error}");
    // An explicit profile doesn't read the variable.
    assert_eq!(
        tes_base_path(&context(&path, VAR).with_profile("remote")),
        "https://tes.example.org/ga4gh/tes/v1"
    );
}

#[test]
fn profile_option_is_global() {
    let command = CliContext::default().command();
    command.clone().debug_assert();
    for args in [
        ["ga4gh-cli", "--profile", "remote", "tes", "list"],
        ["ga4gh-cli", "tes", "--profile", "remote", "list"],
        ["ga4gh-cli", "tes", "list", "--profile", "remote"],
    ] {
        let matches = command.clone().try_get_matches_from(args).unwrap();
        assert_eq!(
            matches
                .get_one::<String>("ga4gh-cli-profile")
                .map(String::as_str),
            Some("remote"),
            "{args:?}"
        );
        let context = CliContext::default().with_profile_from(&matches);
        assert_eq!(context.profile(), Some("remote"), "{args:?}");
    }
    let matches = command
        .clone()
        .try_get_matches_from(["ga4gh-cli", "tes", "list"])
        .unwrap();
    let context = CliContext::default().with_profile("kept");
    assert_eq!(context.with_profile_from(&matches).profile(), Some("kept"));
    // An empty explicit name cannot fall back to a different service.
    let error = command
        .clone()
        .try_get_matches_from(["ga4gh-cli", "--profile", "", "tes", "list"])
        .unwrap_err();
    assert!(error.to_string().contains("a value is required"), "{error}");
    assert!(context.with_profile("").selected_profile().is_err());
    // A subcommand's own argument `profile` isn't the configuration profile.
    let matches = CliContext::default()
        .command()
        .subcommand(clap::Command::new("deploy").arg(clap::Arg::new("profile")))
        .try_get_matches_from(["ga4gh-cli", "deploy", "aws-prod"])
        .unwrap();
    assert_eq!(
        CliContext::default().with_profile_from(&matches).profile(),
        None
    );
    // A command tree without the option leaves the context unchanged.
    let matches = clap::Command::new("bare")
        .try_get_matches_from(["bare"])
        .unwrap();
    assert_eq!(
        CliContext::default().with_profile_from(&matches).profile(),
        None
    );
    let help = command.clone().render_help().to_string();
    assert!(help.contains("--profile <NAME>"), "{help}");
    assert!(help.contains("[env: GA4GH_CLI_PROFILE]"), "{help}");
    let help = CliContext::new("other-cli", "1.0.0")
        .without_profile_env()
        .command()
        .render_help()
        .to_string();
    assert!(!help.contains("[env:"), "{help}");
}

/// A mock TES below `/<prefix>/ga4gh/tes/v1` that lists one task named `task`.
fn mock_tes(prefix: &str, task: &str) -> (mockito::Mock, mockito::Mock, String) {
    let service_info = mock("GET", format!("/{prefix}/ga4gh/tes/v1/service-info").as_str())
        .with_status(200)
        .with_body(r#"{"id":"test","name":"Test","type":{"group":"org.ga4gh","artifact":"tes","version":"1.1.0"},"organization":{"name":"Test","url":"https://example.org"},"version":"1.0.0"}"#)
        .create();
    let tasks = mock("GET", format!("/{prefix}/ga4gh/tes/v1/tasks").as_str())
        .with_status(200)
        .with_body(json!({"tasks": [{"id": task, "state": "COMPLETE"}]}).to_string())
        .create();
    let base_path = format!("{}/{prefix}/ga4gh/tes/v1", mockito::server_url());
    (service_info, tasks, base_path)
}

#[tokio::test]
async fn dispatch_uses_the_profile_option() {
    const VAR: &str = "GA4GH_CLI_TEST_PROFILE_DISPATCH";
    let (_local_info, _local_tasks, local) = mock_tes("profile-local", "task-local");
    let (_remote_info, _remote_tasks, remote) = mock_tes("profile-remote", "task-remote");
    let dir = tempfile::tempdir().unwrap();
    let path = write_config(
        dir.path(),
        json!({
            "default_profile": "local",
            "profiles": {
                "local": {"TES": {"base_path": local}},
                "remote": {"TES": {"base_path": remote}}
            }
        }),
    );
    // The option takes precedence over the variable.
    std::env::set_var(VAR, "local");
    let context = context(&path, VAR);

    let list = |args: &[&str]| context.command().try_get_matches_from(args).unwrap();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = context
        .dispatch_to(
            &list(&["other-cli", "tes", "list", "--profile", "remote"]),
            &mut stdout,
            &mut stderr,
        )
        .await
        .unwrap();
    assert_eq!(exit_code, Some(ExitCode::SUCCESS));
    let stdout = String::from_utf8(stdout).unwrap();
    assert!(stdout.contains("task-remote"), "{stdout}");
    assert!(stderr.is_empty());

    let output = context
        .dispatch(&list(&["other-cli", "--profile", "remote", "tes", "list"]))
        .await
        .unwrap()
        .unwrap();
    assert!(output.stdout.contains("task-remote"), "{}", output.stdout);
    let output = context
        .dispatch(&list(&["other-cli", "tes", "list"]))
        .await
        .unwrap()
        .unwrap();
    assert!(output.stdout.contains("task-local"), "{}", output.stdout);

    let error = context
        .dispatch(&list(&["other-cli", "--profile", "staging", "tes", "list"]))
        .await
        .err()
        .expect("the profile doesn't exist");
    assert!(error.to_string().contains("'staging' not found"), "{error}");
}

/// `workflow submit` sends a configured token, so it shows that a profile's
/// section, with its token in a named variable, reaches the service.
#[tokio::test]
async fn workflow_submission_uses_the_profile_and_its_token_variable() {
    const TOKEN: &str = "GA4GH_CLI_TEST_PROFILE_WORKFLOW_TOKEN";
    let info = json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let service = mock("GET", "/profile-wes/service-info")
        .match_header("authorization", "Bearer token-of-the-profile")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = mock("POST", "/profile-wes/runs")
        .match_header("authorization", "Bearer token-of-the-profile")
        .with_body(r#"{"run_id":"profile-run"}"#)
        .expect(1)
        .create();
    let dir = tempfile::tempdir().unwrap();
    let base_path = format!("{}/profile-wes", mockito::server_url());
    let path = write_config(
        dir.path(),
        json!({
            "profiles": {
                "default": {"WES": {"base_path": "http://localhost:1/unused"}},
                "remote": {"WES": {"base_path": base_path, "oauth_access_token_env": TOKEN}}
            }
        }),
    );
    std::env::set_var(TOKEN, "token-of-the-profile");
    let context = context(&path, "GA4GH_CLI_TEST_PROFILE_WORKFLOW");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let matches = context
        .command()
        .try_get_matches_from([
            "other-cli",
            "--profile",
            "remote",
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
    assert_eq!(response["run_id"], "profile-run");
    service.assert();
    submission.assert();
}
