//! DRS commands use the DRS config section and expose the client's results.

// The CLI locates its config through HOME, which Windows does not use.
#![cfg(unix)]

use mockito::{mock, Mock};
use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

fn home_with_drs_config(base_path: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".ga4gh-cli");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("config.json"),
        json!({"DRS": {"base_path": base_path}}).to_string(),
    )
    .unwrap();
    home
}

fn run(home: &TempDir, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
    command.args(args).env("HOME", home.path());
    for proxy in [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
    ] {
        command.env_remove(proxy);
    }
    command.output().unwrap()
}

fn service_info() -> Mock {
    mock("GET", "/ga4gh/drs/v1/service-info")
        .with_status(200)
        .with_body(
            json!({
                "id": "org.example.drs",
                "name": "Example DRS",
                "type": {"group": "org.ga4gh", "artifact": "drs", "version": "1.4.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0",
            })
            .to_string(),
        )
        .expect(2)
        .create()
}

fn stdout_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    serde_json::from_str(stdout).unwrap()
}

#[test]
fn get_and_access_print_json_and_encode_ids() {
    let service = service_info();
    let object_id = "a/b ?#";
    let access_id = "x/y ?#";
    let object_body = json!({
        "id": object_id,
        "self_uri": "drs://example.org/a%2Fb%20%3F%23",
        "size": 42,
        "created_time": null,
        "checksums": [],
        "access_methods": [{"type": "https", "access_id": access_id}],
        "vendor_field": {"kept": true},
    })
    .to_string();
    let object = mock("GET", "/ga4gh/drs/v1/objects/a%2Fb%20%3F%23")
        .with_status(200)
        .with_body(&object_body)
        .create();
    let access_body = json!({
        "url": "https://example.org/data",
        "headers": ["X-Test: value"],
        "vendor_field": {"kept": true},
    })
    .to_string();
    let access = mock(
        "GET",
        "/ga4gh/drs/v1/objects/a%2Fb%20%3F%23/access/x%2Fy%20%3F%23",
    )
    .with_status(200)
    .with_body(&access_body)
    .create();
    let home = home_with_drs_config(&format!("{}/ga4gh/drs/v1", mockito::server_url()));

    let get_output = run(&home, &["drs", "get", object_id]);
    let metadata = stdout_json(&get_output);
    assert_eq!(metadata["id"], object_id);
    assert_eq!(metadata["size"], 42);
    assert_eq!(metadata["vendor_field"]["kept"], true);
    assert_eq!(get_output.stdout, object_body.as_bytes());
    let access_output = run(&home, &["drs", "access", object_id, access_id]);
    let access_url = stdout_json(&access_output);
    assert_eq!(access_url["url"], "https://example.org/data");
    assert_eq!(access_url["headers"][0], "X-Test: value");
    assert_eq!(access_url["vendor_field"]["kept"], true);
    assert_eq!(access_output.stdout, access_body.as_bytes());
    service.assert();
    object.assert();
    access.assert();
}

#[test]
fn missing_drs_config_and_access_argument_are_errors() {
    let empty_home = TempDir::new().unwrap();
    let missing = run(&empty_home, &["drs", "get", "object-id"]);
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains(".ga4gh-cli/config.json"));

    let usage = run(&empty_home, &["drs", "access", "object-id"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(usage.stdout.is_empty());
    assert!(String::from_utf8_lossy(&usage.stderr).contains("ACCESS_ID"));
}

#[test]
fn service_failures_have_nonzero_exit_codes() {
    let service = service_info();
    let object = mock("GET", "/ga4gh/drs/v1/objects/missing")
        .with_status(404)
        .with_body("object not found")
        .create();
    let access = mock("GET", "/ga4gh/drs/v1/objects/known/access/missing")
        .with_status(404)
        .with_body("access not found")
        .create();
    let home = home_with_drs_config(&format!("{}/ga4gh/drs/v1", mockito::server_url()));

    for (args, expected) in [
        (vec!["drs", "get", "missing"], "object not found"),
        (
            vec!["drs", "access", "known", "missing"],
            "access not found",
        ),
    ] {
        let output = run(&home, &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    }
    service.assert();
    object.assert();
    access.assert();
}

#[test]
fn pending_drs_responses_have_nonzero_exit_codes() {
    let service = service_info();
    let object = mock("GET", "/ga4gh/drs/v1/objects/pending")
        .with_status(202)
        .with_header("retry-after", "2")
        .create();
    let access = mock("GET", "/ga4gh/drs/v1/objects/known/access/pending")
        .with_status(202)
        .with_header("retry-after", "2")
        .create();
    let home = home_with_drs_config(&format!("{}/ga4gh/drs/v1", mockito::server_url()));

    for args in [
        vec!["drs", "get", "pending"],
        vec!["drs", "access", "known", "pending"],
    ] {
        let output = run(&home, &args);
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("retry after 2 seconds"));
    }
    service.assert();
    object.assert();
    access.assert();
}
