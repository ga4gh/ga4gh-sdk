//! Checks what the CLI writes to stdout and stderr, for every `tes` command.
//!
//! Runs the binary against a mock service with a config file full of marker
//! values, and expects none of these values in the output, at any log level.

// The CLI finds its config file through `HOME`, which Windows doesn't use.
#![cfg(unix)]

use mockito::{mock, Matcher, Mock};
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const MARKER: &str = "SENTINEL";
// The start of the marker in Base64, as in a Basic authorization header.
const ENCODED_MARKER: &str = "U0VOVElORUxf";
const LOG_LEVELS: [Option<&str>; 3] = [None, Some("debug"), Some("trace")];
const TASK_ID: &str = "t1";

/// Config with marker values in all authentication fields, read by the CLI or not.
fn config_with_fields(base_path: &str) -> Value {
    json!({"TES": {
        "base_path": base_path,
        "basic_auth": {"username": "SENTINEL_USER", "password": "SENTINEL_PASSWORD"},
        "oauth_access_token": "SENTINEL_OAUTH",
        "bearer_access_token": "SENTINEL_BEARER",
        "api_key": {"prefix": "Token", "key": "SENTINEL_KEY"},
    }})
}

/// Config with marker values in the base path credentials only.
fn config_with_url(base_path: &str) -> Value {
    let base_path = base_path.replacen("http://", "http://SENTINEL_USER:SENTINEL_PASSWORD@", 1);
    json!({"TES": {
        "base_path": format!("{base_path}/"),
        "basic_auth": null,
        "oauth_access_token": null,
    }})
}

/// A home directory with a config file and a task file.
struct Home(TempDir);

impl Home {
    fn new(config: &Value) -> Self {
        let home = TempDir::new().unwrap();
        let config_dir = home.path().join(".ga4gh-cli");
        fs::create_dir(&config_dir).unwrap();
        fs::write(config_dir.join("config.json"), config.to_string()).unwrap();
        let task = json!({"executors": [{"image": "alpine", "command": ["true"]}]});
        fs::write(home.path().join("task.json"), task.to_string()).unwrap();
        Home(home)
    }

    fn path(&self) -> &Path {
        self.0.path()
    }

    /// Runs the CLI, and checks that its output holds no marker value.
    fn run(&self, args: &[&str], log_level: Option<&str>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
        command.args(args).env("HOME", self.path());
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
        match log_level {
            Some(level) => command.env("RUST_LOG", level),
            None => command.env_remove("RUST_LOG"),
        };
        let output = command.output().unwrap();
        for stream in [&output.stdout, &output.stderr] {
            let text = String::from_utf8_lossy(stream);
            assert!(
                !text.contains(MARKER) && !text.contains(ENCODED_MARKER),
                "marker value in the output of {args:?} with RUST_LOG={log_level:?}:\n{text}",
            );
        }
        output
    }

    /// Runs all commands at all log levels.
    fn run_all(&self) -> Vec<(Vec<String>, Output)> {
        let task_file = self.path().join("task.json");
        let task_file = task_file.to_str().unwrap();
        let commands: [&[&str]; 5] = [
            &["tes", "create", task_file],
            &["tes", "list"],
            &["tes", "get", TASK_ID, "FULL"],
            &["tes", "status", TASK_ID],
            &["tes", "cancel", TASK_ID],
        ];
        let mut outputs = Vec::new();
        for args in commands {
            for log_level in LOG_LEVELS {
                let args_owned = args.iter().map(ToString::to_string).collect();
                outputs.push((args_owned, self.run(args, log_level)));
            }
        }
        outputs
    }
}

fn service_info_mock(path: &str) -> Mock {
    let service_info = json!({
        "id": "org.example.tes",
        "name": "Mock",
        "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0",
    });
    mock("GET", path)
        .with_status(200)
        .with_body(service_info.to_string())
        .create()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn test_output_on_success() {
    let task = json!({"id": TASK_ID, "state": "COMPLETE", "executors": [], "vendor_field": {"kept": true}});
    let task_json = task.to_string();
    let prefix = "/ga4gh/tes/v1";
    let _mocks = [
        service_info_mock("/ga4gh/tes/v1/service-info"),
        mock("POST", "/ga4gh/tes/v1/tasks")
            .with_status(200)
            .with_body(json!({"id": TASK_ID}).to_string())
            .create(),
        mock(
            "GET",
            Matcher::Regex(r"^/ga4gh/tes/v1/tasks(\?.*)?$".to_string()),
        )
        .with_status(200)
        .with_body(json!({"tasks": [task]}).to_string())
        .create(),
        mock("GET", "/ga4gh/tes/v1/tasks/t1?view=FULL")
            .with_status(200)
            .with_body(task_json.clone())
            .create(),
        mock("GET", "/ga4gh/tes/v1/tasks/t1?view=MINIMAL")
            .with_status(200)
            .with_body(task.to_string())
            .create(),
        mock("POST", "/ga4gh/tes/v1/tasks/t1:cancel")
            .with_status(200)
            .with_body("{}")
            .create(),
    ];

    let base_path = format!("{}{prefix}", mockito::server_url());
    for config in [config_with_fields(&base_path), config_with_url(&base_path)] {
        for (args, output) in Home::new(&config).run_all() {
            assert!(output.status.success(), "{args:?}: {}", stderr(&output));
            match args[1].as_str() {
                "create" => assert_eq!(stdout(&output), format!("{TASK_ID}\n")),
                "get" => assert_eq!(stdout(&output), task_json),
                "status" => assert_eq!(stdout(&output), "COMPLETE\n"),
                "cancel" => assert!(stdout(&output).starts_with("STATUS: "), "{args:?}"),
                _ => assert!(stdout(&output).contains(TASK_ID), "{args:?}"),
            }
        }
    }
}

#[test]
fn test_output_on_error_response() {
    for status in [401, 404, 500] {
        let body = format!("mock failure {status}");
        let _mocks = [
            service_info_mock("/service-info"),
            mock("POST", Matcher::Any)
                .with_status(status)
                .with_body(&body)
                .create(),
            mock("GET", Matcher::Regex(r"^/tasks".to_string()))
                .with_status(status)
                .with_body(&body)
                .create(),
        ];

        let base_path = mockito::server_url();
        for config in [config_with_fields(&base_path), config_with_url(&base_path)] {
            for (args, output) in Home::new(&config).run_all() {
                assert_eq!(
                    output.status.code(),
                    Some(1),
                    "{args:?}: {}",
                    stderr(&output)
                );
                assert!(output.stdout.is_empty(), "{args:?}: {}", stdout(&output));
                let message = stderr(&output);
                assert!(message.contains(&status.to_string()), "{args:?}: {message}");
                assert!(message.contains(&body), "{args:?}: {message}");
            }
        }
    }
}

#[test]
fn service_info_404_keeps_api_root_hint() {
    let _service_info = mock("GET", "/service-info")
        .with_status(404)
        .with_body("service missing")
        .create();
    let home = Home::new(&config_with_fields(&mockito::server_url()));
    let task_file = home.path().join("task.json");
    let output = home.run(&["tes", "create", task_file.to_str().unwrap()], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(stdout(&output).is_empty());
    let message = stderr(&output);
    assert!(message.contains("404"), "{message}");
    assert!(message.contains("service missing"), "{message}");
    assert!(
        message.contains("base_path is the TES API root"),
        "{message}"
    );
}

#[test]
fn canceling_terminal_task_reports_server_conflict() {
    for state in ["COMPLETE", "SYSTEM_ERROR"] {
        let body = format!("task already {state}");
        let _cancel = mock("POST", "/tasks/t1:cancel")
            .with_status(409)
            .with_body(&body)
            .create();
        let home = Home::new(&config_with_fields(&mockito::server_url()));
        let output = home.run(&["tes", "cancel", TASK_ID], None);
        assert_eq!(output.status.code(), Some(1));
        assert!(stdout(&output).is_empty());
        let message = stderr(&output);
        assert!(message.contains("409"), "{message}");
        assert!(message.contains(&body), "{message}");
    }
}

#[test]
fn test_output_without_service() {
    // Bind a free port and release it, so that connections to it are refused.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };

    let base_path = format!("http://127.0.0.1:{port}");
    for config in [config_with_fields(&base_path), config_with_url(&base_path)] {
        for (args, output) in Home::new(&config).run_all() {
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert!(output.stdout.is_empty(), "{args:?}");
            assert!(!output.stderr.is_empty(), "{args:?}");
        }
    }
}

#[test]
fn test_output_with_invalid_base_path() {
    let base_paths = [
        "http://SENTINEL_USER:SENTINEL_PASSWORD@127.0.0.1:invalid",
        "SENTINEL_USER:SENTINEL_PASSWORD@127.0.0.1/v1",
        "http://SENTINEL_USER:SENTINEL_PASSWORD@127.0.0.1/v1/?key=SENTINEL_QUERY",
        "http://SENTINEL_USER:SENTINEL_PASSWORD@127.0.0.1/v1/#SENTINEL_FRAGMENT",
    ];
    for base_path in base_paths {
        for (args, output) in Home::new(&config_with_fields(base_path)).run_all() {
            assert!(!output.status.success(), "{args:?}");
        }
    }
}

#[test]
fn all_jsonl_streams_records_and_reports_a_failed_later_page_on_stderr() {
    let _service = service_info_mock("/service-info");
    let first = mock("GET", "/tasks")
        .with_body(r#"{"tasks":[{"id":"one","custom":true}],"next_page_token":"a+/ %"}"#)
        .expect(1)
        .create();
    let second = mock("GET", "/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "a+/ %".into()))
        .with_status(503)
        .expect(1)
        .create();
    let home = Home::new(&config_with_fields(&mockito::server_url()));
    let output = home.run(&["tes", "list", "--all", "--format", "jsonl"], None);
    assert!(!output.status.success());
    let lines = stdout(&output);
    let records = lines
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["id"], "one");
    assert_eq!(records[0]["custom"], true);
    let diagnostics = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostics.contains("Resume checkpoint"));
    assert!(diagnostics.contains("a+/ %"));
    first.assert();
    second.assert();
}

#[test]
fn all_jsonl_exports_complete_pages_without_table_or_diagnostics() {
    let _service = service_info_mock("/export/service-info");
    let first = mock("GET", "/export/tasks")
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"next"}"#)
        .expect(1)
        .create();
    let second = mock("GET", "/export/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "next".into()))
        .with_body(r#"{"tasks":[{"id":"two"}]}"#)
        .expect(1)
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/export",
        mockito::server_url()
    )));
    let output = home.run(&["tes", "list", "--all", "--format", "jsonl"], None);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let records = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["id"], "one");
    assert_eq!(records[1]["id"], "two");
    first.assert();
    second.assert();
}
