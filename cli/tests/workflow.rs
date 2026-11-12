use ga4gh_cli::{command, dispatch_with_config};
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::{json, Value};
use std::{path::Path, process::Command};
use url::Url;
fn run(args: &[&str], home: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}
#[test]
fn real_cli_preparation_rejects_bad_params_without_configuration_and_emits_reviewable_json() {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let invalid = root.join("examples/sequence-qc/invalid-request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let args = |file: &Path| {
        vec![
            "workflow".to_owned(),
            "prepare".into(),
            schema.to_str().unwrap().into(),
            file.to_str().unwrap().into(),
            "public-qc@1".into(),
            "--profile-revision".into(),
            pin.into(),
            "--vocabulary-revision".into(),
            pin.into(),
        ]
    };
    let values = args(&invalid);
    let args = values.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run(&args, temp.path());
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("/sequences/0"));
    assert!(stderr.contains("pattern"));
    let values = vec![
        "workflow",
        "prepare",
        schema.to_str().unwrap(),
        request.to_str().unwrap(),
        "public-qc@1",
        "--profile-revision",
        pin,
        "--vocabulary-revision",
        pin,
    ];
    let output = run(&values, temp.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prepared: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        prepared["request"]["workflow_params"]["sequences"],
        json!(["ACGT", "AC", "ACGTAC"])
    );
    assert!(prepared["fingerprint"].as_str().unwrap().len() == 64);
    assert_eq!(prepared["pins"]["profile_revision"], pin);
}
#[test]
fn cli_output_projection_keeps_complete_with_missing_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let log = temp.path().join("log.json");
    std::fs::write(&log,json!({"state":"COMPLETE","outputs":{"metrics":{"sequence_count":3,"total_length":12,"mean_length":4.0}}}).to_string()).unwrap();
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let output = run(
        &[
            "workflow",
            "results",
            schema.to_str().unwrap(),
            log.to_str().unwrap(),
            "wes/run1",
            "--profile-revision",
            pin,
            "--vocabulary-revision",
            pin,
        ],
        temp.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["execution_state"], "COMPLETE");
    assert_eq!(record["outputs"]["report"]["contract_status"], "missing");
}

#[test]
fn explicit_cli_submission_uploads_the_reviewed_request_once() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join(".ga4gh-cli");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.json"),
        json!({"WES":{"base_path":format!("{}/cli-wes",mockito::server_url()),"oauth_access_token":"cli-fixture-token"}}).to_string(),
    )
    .unwrap();
    let info = json!({"workflow_type_versions":{"CWL":{"workflow_type_version":["v1.2"]}},"supported_wes_versions":["1.0.0"],"supported_filesystem_protocols":["file"],"workflow_engine_versions":{},"default_workflow_engine_parameters":[],"system_state_counts":{},"tags":{}});
    let service = mockito::mock("GET", "/cli-wes/service-info")
        .match_header("authorization", "Bearer cli-fixture-token")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = mockito::mock("POST", "/cli-wes/runs")
        .match_header("authorization", "Bearer cli-fixture-token")
        .match_body(mockito::Matcher::Regex("Sequence QC".into()))
        .with_body(r#"{"run_id":"cli-qc-run"}"#)
        .expect(1)
        .create();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let descriptor = root.join("examples/sequence-qc/main.cwl");
    let attachment = format!("main.cwl={}", descriptor.display());
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let output = run(
        &[
            "workflow",
            "submit",
            schema.to_str().unwrap(),
            request.to_str().unwrap(),
            "public-qc@1",
            "--profile-revision",
            pin,
            "--vocabulary-revision",
            pin,
            "--attachment",
            &attachment,
        ],
        temp.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["run_id"], "cli-qc-run");
    assert_eq!(
        result["invocation"]["request"]["workflow_params"]["sequences"][0],
        "ACGT"
    );
    service.assert();
    submission.assert();
}

#[tokio::test]
async fn downstream_workflow_submission_uses_supplied_config() {
    let info = json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let service = mockito::mock("GET", "/injected-wes/service-info")
        .match_header("authorization", "Bearer injected-token")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = mockito::mock("POST", "/injected-wes/runs")
        .match_header("authorization", "Bearer injected-token")
        .with_body(r#"{"run_id":"injected-run"}"#)
        .expect(1)
        .create();
    let config =
        Configuration::new(Url::parse(&format!("{}/injected-wes", mockito::server_url())).unwrap())
            .with_oauth_access_token("injected-token".into());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let matches = command()
        .try_get_matches_from([
            "ga4gh-cli",
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
    let output = dispatch_with_config(&matches, &config)
        .await
        .unwrap()
        .expect("workflow is a built-in command");
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    let response: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(response["run_id"], "injected-run");
    service.assert();
    submission.assert();
}
