//! Browses a real TRS-Filer (TRS 2.0.2) through the `trs` commands of the
//! CLI binary.
//!
//! Run through `tests/run-trs-filer-integration.sh`, which starts a fresh
//! TRS-Filer, registers the tool in `tests/fixtures/trs-filer/tool.json` and
//! sets `TRS_FILER_BASE_URL` and `TRS_FILER_TOOL_ID`.
#![cfg(all(unix, feature = "trs_filer_integration_tests"))]

use serde_json::{json, Value};
use std::fs;
use std::process::Command;
use tempfile::TempDir;

const FIXTURE: &str = include_str!("../../tests/fixtures/trs-filer/tool.json");

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is unset; run through tests/run-trs-filer-integration.sh")
    })
}

// The registered content of the fixture file at `path`.
fn content(fixture: &Value, path: &str) -> Value {
    fixture["versions"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["tool_file"]["path"] == path)
        .map(|file| file["file_wrapper"]["content"].clone())
        .unwrap_or_else(|| panic!("{path} is not in the fixture"))
}

fn cli(home: &TempDir, args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
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
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let id = env("TRS_FILER_TOOL_ID");
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        json!({"TRS": {"base_path": env("TRS_FILER_BASE_URL")}}).to_string(),
    )
    .unwrap();

    let mut args = vec!["trs", "tools"];
    for tag in fixture["tags"].as_array().unwrap() {
        args.extend(["--tag", tag.as_str().unwrap()]);
    }
    let tools = json_output(&home, &args);
    assert!(
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["id"] == id.as_str()),
        "{tools}"
    );

    let versions = json_output(&home, &["trs", "versions", &id]);
    let version_id = versions[0]["id"].as_str().unwrap().to_owned();
    let version = json_output(&home, &["trs", "version", &id, &version_id]);
    assert_eq!(
        version["description"],
        fixture["versions"][0]["description"]
    );

    // TRS-Filer doesn't serve the `PLAIN_` types (`--plain`) yet.
    let descriptor = json_output(
        &home,
        &["trs", "descriptor", &id, &version_id, "--type", "CWL"],
    );
    assert_eq!(descriptor["content"], content(&fixture, "main.cwl"));
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
    assert_eq!(schema["content"], content(&fixture, "inputs.schema.json"));

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
