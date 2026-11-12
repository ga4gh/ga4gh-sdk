//! Browses a real TRS-Filer (TRS 2.0.2) through the `trs` commands of the
//! CLI binary.
//!
//! Run through `tests/run-trs-filer-integration.sh`, which starts TRS-Filer
//! and sets `TRS_FILER_BASE_URL`. The test registers its own tool with a
//! unique tag.
#![cfg(all(unix, feature = "trs_filer_integration_tests"))]

use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

const MAIN: &str = "cwlVersion: v1.2\nclass: Workflow\ninputs: []\nsteps: []\noutputs: []\n";
const SCHEMA: &str = r#"{"type":"object"}"#;

fn base_url() -> String {
    std::env::var("TRS_FILER_BASE_URL")
        .expect("TRS_FILER_BASE_URL is unset; run through tests/run-trs-filer-integration.sh")
}

// Registers a tool with `curl`, which the integration script needs anyway, so
// that the CLI's dev-dependencies stay as they are.
fn register(tag: &str) -> String {
    let tool = json!({
        "organization": "GA4GH SDK integration",
        "name": "cli-workflow",
        "toolclass": {"name": "Workflow", "description": "A workflow"},
        "tags": [tag],
        "versions": [{
            "name": "1.0.0",
            "description": "CLI release",
            "descriptor_type": ["CWL"],
            "files": [
                {"tool_file": {"path": "main.cwl", "file_type": "PRIMARY_DESCRIPTOR"},
                 "file_wrapper": {"content": MAIN}, "type": "CWL"},
                {"tool_file": {"path": "inputs.schema.json", "file_type": "OTHER"},
                 "file_wrapper": {"content": SCHEMA}, "type": "CWL"},
            ],
        }],
    });
    let output = Command::new("curl")
        .args(["--silent", "--show-error", "--fail-with-body", "-X", "POST"])
        .args(["-H", "Content-Type: application/json", "--data-binary"])
        .arg(tool.to_string())
        .arg(format!("{}/tools", base_url()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "registration failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn cli(home: &TempDir, args: &[&str]) -> String {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn json_output(home: &TempDir, args: &[&str]) -> Value {
    serde_json::from_str(&cli(home, args)).unwrap()
}

#[test]
fn cli_browses_tools_and_downloads_files() {
    let tag = format!("sdk-cli-{}", std::process::id());
    let id = register(&tag);
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        json!({"TRS": {"base_path": base_url()}}).to_string(),
    )
    .unwrap();

    let tools = json_output(&home, &["trs", "tools", "--tag", &tag]);
    assert_eq!(tools.as_array().unwrap().len(), 1);
    assert_eq!(tools[0]["id"], id.as_str());

    let versions = json_output(&home, &["trs", "versions", &id]);
    let version_id = versions[0]["id"].as_str().unwrap().to_owned();
    let version = json_output(&home, &["trs", "version", &id, &version_id]);
    assert_eq!(version["description"], "CLI release");

    // TRS-Filer doesn't serve the `PLAIN_` types (`--plain`) yet.
    let descriptor = json_output(
        &home,
        &["trs", "descriptor", &id, &version_id, "--type", "CWL"],
    );
    assert_eq!(descriptor["content"], MAIN);
    let schema = json_output(
        &home,
        &[
            "trs",
            "descriptor",
            &id,
            &version_id,
            "--type",
            "CWL",
            "--path",
            "inputs.schema.json",
        ],
    );
    assert_eq!(schema["content"], SCHEMA);

    let files = json_output(&home, &["trs", "files", &id, &version_id, "--type", "CWL"]);
    let other: Vec<_> = files
        .as_array()
        .unwrap()
        .iter()
        .filter(|file| file["file_type"] == "OTHER")
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(other, ["inputs.schema.json"]);

    let archive = home.path().join("workflow.zip");
    let written = cli(
        &home,
        &[
            "trs",
            "files",
            &id,
            &version_id,
            "--type",
            "CWL",
            "--zip",
            archive.to_str().unwrap(),
        ],
    );
    assert!(written.starts_with("Wrote "), "{written}");
    assert!(fs::read(&archive).unwrap().starts_with(b"PK\x03\x04"));
}
