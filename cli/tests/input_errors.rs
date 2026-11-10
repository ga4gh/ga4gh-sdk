//! User input errors should be reported without panicking or contacting a service.

#![cfg(unix)]

use mockito::{mock, Matcher};
use serde_json::json;
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

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

fn home_with_config(config: serde_json::Value) -> TempDir {
    let home = TempDir::new().unwrap();
    let config_dir = home.path().join(".ga4gh-cli");
    fs::create_dir(&config_dir).unwrap();
    fs::write(config_dir.join("config.json"), config.to_string()).unwrap();
    home
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_input_error(output: &Output, expected: &str) {
    let message = stderr(output);
    assert!(!output.status.success(), "{message}");
    assert!(message.contains(expected), "{message}");
    assert!(!message.contains("panicked"), "{message}");
}

#[test]
fn missing_configuration_names_expected_path() {
    let home = TempDir::new().unwrap();
    let output = run(&home, &["tes", "list"]);
    assert_input_error(&output, ".ga4gh-cli/config.json");
}

#[test]
fn incomplete_configuration_is_an_error() {
    let cases = [
        (json!({"TES": {}}), "base_path"),
        (
            json!({"DRS": {"base_path": "https://example.org"}}),
            "'TES' object",
        ),
    ];
    for (config, expected) in cases {
        let home = home_with_config(config);
        assert_input_error(&run(&home, &["tes", "list"]), expected);
    }
}

#[test]
fn invalid_list_arguments_are_errors() {
    let home = home_with_config(json!({"TES": {"base_path": mockito::server_url()}}));
    let cases: [(&[&str], &str); 3] = [
        (&["tes", "list", "-s", "NOT_A_STATE"], "Invalid state"),
        (&["tes", "list", "-p", "abc"], "Invalid page_size"),
        (
            &["tes", "list", "-p", "99999999999999999999"],
            "Invalid page_size",
        ),
    ];
    for (args, expected) in cases {
        assert_input_error(&run(&home, args), expected);
    }
    assert_input_error(
        &run(&home, &["tes", "list", "-v", "orphan"]),
        "Each tag_value needs a corresponding tag_key",
    );
}

#[test]
fn minimal_configuration_and_unquoted_states_work() {
    let base_path = format!("{}/ga4gh/tes/v1", mockito::server_url());
    let home = home_with_config(json!({"TES": {"base_path": base_path}}));
    let service_info = json!({
        "id": "org.example.tes",
        "name": "Mock",
        "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0"
    });
    let _service_info = mock("GET", "/ga4gh/tes/v1/service-info")
        .with_status(200)
        .with_body(service_info.to_string())
        .create();
    let queued = mock("GET", "/ga4gh/tes/v1/tasks")
        .match_query(Matcher::UrlEncoded("state".into(), "QUEUED".into()))
        .with_status(200)
        .with_body(r#"{"tasks":[]}"#)
        .expect(3)
        .create();
    for state in ["QUEUED", "queued", "\"Queued\""] {
        let output = run(&home, &["tes", "list", "-s", state]);
        assert!(output.status.success(), "{state}: {}", stderr(&output));
    }
    queued.assert();
}

#[test]
fn missing_task_state_is_an_error() {
    let base_path = format!("{}/ga4gh/tes/v1", mockito::server_url());
    let home = home_with_config(json!({"TES": {"base_path": base_path}}));
    let task = mock("GET", "/ga4gh/tes/v1/tasks/t1?view=MINIMAL")
        .with_status(200)
        .with_body(r#"{"id":"t1"}"#)
        .create();
    assert_input_error(&run(&home, &["tes", "status", "t1"]), "missing 'state'");
    task.assert();
}

#[test]
fn clap_usage_errors_exit_with_code_two() {
    let home = TempDir::new().unwrap();
    let output = run(&home, &["tes", "get", "t1"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("required"));
}
