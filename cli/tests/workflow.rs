use serde_json::{json, Value};
use std::{path::Path, process::Command};
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
    let schema = root.join("examples/qc-demo/profile.json");
    let request = root.join("examples/qc-demo/request.json");
    let invalid = root.join("examples/qc-demo/invalid-request.json");
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
    let schema = root.join("examples/qc-demo/profile.json");
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
