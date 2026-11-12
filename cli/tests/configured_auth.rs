//! A configured credential reaches every built-in service command family.

use ga4gh_cli::{command, dispatch_with_config};
use ga4gh_sdk::utils::configuration::{BasicAuth, Configuration};
use mockito::mock;
use serde_json::json;
use url::Url;

fn config(prefix: &str) -> Configuration {
    Configuration::new(Url::parse(&format!("{}{prefix}", mockito::server_url())).unwrap())
        .with_basic_auth(BasicAuth {
            username: "cli".into(),
            password: Some("secret".into()),
        })
}

fn service_info(artifact: &str) -> String {
    json!({
        "id": format!("org.example.{artifact}"),
        "name": "Protected service",
        "type": {"group": "org.ga4gh", "artifact": artifact, "version": "2.0.2"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0"
    })
    .to_string()
}

// WES discovery uses its native capability document instead of the common
// ServiceInfo shape used by TES, DRS and TRS.
fn wes_service_info() -> String {
    json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    })
    .to_string()
}

async fn dispatch(args: &[&str], config: &Configuration) -> ga4gh_cli::CommandOutput {
    let matches = command().try_get_matches_from(args).unwrap();
    dispatch_with_config(&matches, config)
        .await
        .unwrap()
        .expect("built-in command")
}

#[tokio::test]
async fn configured_basic_reaches_discovery_and_operations_in_every_service_family() {
    const HEADER: &str = "Basic Y2xpOnNlY3JldA==";
    for (family, args, discovery, endpoint, body) in [
        (
            "tes",
            vec!["ga4gh-cli", "tes", "list"],
            service_info("tes"),
            "tasks",
            json!({"tasks": []}).to_string(),
        ),
        (
            "drs",
            vec!["ga4gh-cli", "drs", "get", "object-1"],
            service_info("drs"),
            "objects/object-1",
            json!({"id": "object-1"}).to_string(),
        ),
        (
            "trs",
            vec!["ga4gh-cli", "trs", "tools"],
            service_info("trs"),
            "tools",
            "[]".into(),
        ),
        (
            "wes",
            vec!["ga4gh-cli", "wes", "list"],
            wes_service_info(),
            "runs",
            json!({"runs": []}).to_string(),
        ),
    ] {
        let prefix = format!("/cli-auth-{family}");
        let info = mock("GET", format!("{prefix}/service-info").as_str())
            .match_header("authorization", HEADER)
            .with_body(discovery)
            .create();
        let operation = mock("GET", format!("{prefix}/{endpoint}").as_str())
            .match_header("authorization", HEADER)
            .with_body(body)
            .create();
        let output = dispatch(&args, &config(&prefix)).await;
        assert_eq!(
            output.exit_code,
            std::process::ExitCode::SUCCESS,
            "{family}: {}",
            output.stderr
        );
        info.assert();
        operation.assert();
    }

    let prefix = "/cli-auth-registry";
    let operation = mock("GET", "/cli-auth-registry/services")
        .match_header("authorization", HEADER)
        .with_body("[]")
        .create();
    let output = dispatch(&["ga4gh-cli", "registry", "list"], &config(prefix)).await;
    assert_eq!(
        output.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        output.stderr
    );
    operation.assert();
}

#[tokio::test]
async fn wes_run_uses_the_same_configured_credential_for_discovery_and_submission() {
    let prefix = "/cli-auth-submit";
    let discovery = mock("GET", "/cli-auth-submit/service-info")
        .match_header("authorization", "Basic Y2xpOnNlY3JldA==")
        .with_body(wes_service_info())
        .create();
    let submission = mock("POST", "/cli-auth-submit/runs")
        .match_header("authorization", "Basic Y2xpOnNlY3JldA==")
        .with_body(r#"{"run_id":"run-1"}"#)
        .expect(1)
        .create();
    let output = dispatch(
        &[
            "ga4gh-cli",
            "wes",
            "run",
            "https://example.org/main.cwl",
            "--type",
            "CWL",
            "--type_version",
            "v1.2",
        ],
        &config(prefix),
    )
    .await;
    assert_eq!(
        output.exit_code,
        std::process::ExitCode::SUCCESS,
        "{}",
        output.stderr
    );
    discovery.assert();
    submission.assert();
}

#[tokio::test]
async fn wrong_credential_yields_inspectable_401() {
    let unauthorized = mock("GET", "/cli-auth-401/services")
        .match_header("authorization", "Basic Y2xpOnNlY3JldA==")
        .with_status(401)
        .with_body("authentication required")
        .expect(1)
        .create();
    let output = dispatch(&["ga4gh-cli", "registry", "list"], &config("/cli-auth-401")).await;
    assert_eq!(output.exit_code, std::process::ExitCode::FAILURE);
    assert!(output.stderr.contains("401"), "{}", output.stderr);
    unauthorized.assert();
}

#[cfg(unix)]
#[test]
fn file_section_reads_explicit_token_variable_and_sends_it() {
    let home = tempfile::TempDir::new().unwrap();
    let config_dir = home.path().join(".ga4gh-cli");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.json"),
        json!({"REGISTRY": {
            "base_path": format!("{}/cli-auth-env", mockito::server_url()),
            "bearer_access_token_env": "GA4GH_SDK_AUTH203_TEST_TOKEN"
        }})
        .to_string(),
    )
    .unwrap();
    let request = mock("GET", "/cli-auth-env/services")
        .match_header("authorization", "Bearer from-env")
        .with_body("[]")
        .create();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(["registry", "list"])
        .env("HOME", home.path())
        .env("GA4GH_SDK_AUTH203_TEST_TOKEN", "from-env")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    request.assert();
}
