#![cfg(unix)]

use mockito::{mock, Matcher};
use serde_json::json;
use std::{
    fs,
    process::{Command, Output},
};
use tempfile::TempDir;

fn run(prefix: &str, args: &[&str]) -> Output {
    let home = TempDir::new().unwrap();
    let dir = home.path().join(".ga4gh-cli");
    fs::create_dir(&dir).unwrap();
    fs::write(
        dir.join("config.json"),
        json!({"TRS":{"base_path":format!("{}/{prefix}", mockito::server_url())}}).to_string(),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
    cmd.env("HOME", home.path()).args(["trs"]).args(args);
    for key in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        cmd.env_remove(key);
    }
    cmd.output().unwrap()
}
fn info(version: &str) -> String {
    json!({"id":"test", "name":"TRS", "type":{"group":"org.ga4gh","artifact":"trs","version":version},
        "organization":{"name":"test","url":"https://example.org"},"version":"server"}).to_string()
}

#[test]
fn tag_filter_requires_discovery_support_or_an_explicit_override() {
    let discovery = mock("GET", "/cli-compat-tags/service-info")
        .with_status(200)
        .with_body(info("2.0.1"))
        .expect(2)
        .create();
    let tools = mock("GET", "/cli-compat-tags/tools")
        .match_query(Matcher::UrlEncoded("tags".into(), "genomics".into()))
        .with_status(200)
        .with_body("[]")
        .expect(1)
        .create();
    let denied = run("cli-compat-tags", &["tools", "--tag", "genomics"]);
    assert!(!denied.status.success());
    assert!(denied.stdout.is_empty());
    assert!(String::from_utf8_lossy(&denied.stderr).contains("tags"));
    let allowed = run(
        "cli-compat-tags",
        &["tools", "--tag", "genomics", "--tag-filter", "enabled"],
    );
    assert!(
        allowed.status.success(),
        "{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&allowed.stdout).unwrap(),
        json!([])
    );
    discovery.assert();
    tools.assert();
}

#[test]
fn strict_mode_rejects_unknown_declarations_without_tools_request() {
    let discovery = mock("GET", "/cli-compat-strict/service-info")
        .with_status(200)
        .with_body(info("2.1.0"))
        .create();
    let tools = mock("GET", "/cli-compat-strict/tools").expect(0).create();
    let output = run("cli-compat-strict", &["--api-version", "strict", "tools"]);
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not recognized"));
    discovery.assert();
    tools.assert();
}

#[test]
fn no_discovery_requires_explicit_version_before_network_io() {
    let discovery = mock("GET", "/cli-compat-no-version/service-info")
        .expect(0)
        .create();
    let tools = mock("GET", "/cli-compat-no-version/tools")
        .expect(0)
        .create();
    let output = run("cli-compat-no-version", &["--no-discovery", "tools"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires --api-version"));
    discovery.assert();
    tools.assert();
}

#[test]
fn selected_profile_controls_decoding_and_keeps_description_in_output() {
    let discovery = mock("GET", "/cli-compat-profile/service-info")
        .expect(0)
        .create();
    let version = mock("GET", "/cli-compat-profile/tools/t/versions/v1")
        .with_status(200)
        .with_body(r#"{"id":"v1","url":"https://example.org","description":{"extension":42}}"#)
        .expect(2)
        .create();
    let args = [
        "--no-discovery",
        "--api-version",
        "2.0.1",
        "version",
        "t",
        "v1",
    ];
    let old = run("cli-compat-profile", &args);
    assert!(
        old.status.success(),
        "{}",
        String::from_utf8_lossy(&old.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&old.stdout).unwrap()["description"],
        json!({"extension":42})
    );
    let new = run(
        "cli-compat-profile",
        &[
            "--no-discovery",
            "--api-version",
            "2.0.2",
            "version",
            "t",
            "v1",
        ],
    );
    assert!(!new.status.success());
    assert!(new.stdout.is_empty());
    discovery.assert();
    version.assert();
}
