//! Checks what the CLI writes to stdout and stderr, for every `tes` command.
//!
//! Runs the binary against a mock service with a config file full of marker
//! values, and expects none of these values in the output, at any log level.

// The CLI finds its config file through `HOME`, which Windows doesn't use.
#![cfg(unix)]

use mockito::{mock, Matcher, Mock};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
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

    fn run_with_stdin(&self, args: &[&str], input: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
        command
            .args(args)
            .env("HOME", self.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
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
fn create_accepts_file_inline_json_and_stdin() {
    let _service = service_info_mock("/ga4gh/tes/v1/service-info");
    let create = mock("POST", "/ga4gh/tes/v1/tasks")
        .match_body(Matcher::PartialJson(
            json!({"executors": [{"image": "alpine"}]}),
        ))
        .with_status(200)
        .with_body(json!({"id": TASK_ID}).to_string())
        .expect(3)
        .create();
    let base_path = format!("{}/ga4gh/tes/v1", mockito::server_url());
    let home = Home::new(&config_with_fields(&base_path));
    let file = home.path().join("task.json");
    let file = file.to_str().unwrap();
    let inline = fs::read_to_string(file).unwrap();

    for output in [
        home.run(&["tes", "create", file], None),
        home.run(&["tes", "create", &inline], None),
        home.run_with_stdin(&["tes", "create", "-"], &inline),
    ] {
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(stdout(&output), format!("{TASK_ID}\n"));
    }
    create.assert();
}

#[test]
fn list_sends_repeated_tags_and_displays_next_page_token() {
    let _service = service_info_mock("/ga4gh/tes/v1/service-info");
    let list = mock("GET", "/ga4gh/tes/v1/tasks")
        .match_query(Matcher::Exact(
            "page_token=first+page&tag_key=alpha&tag_key=beta&tag_value=x%2Fy&tag_value=z"
                .to_owned(),
        ))
        .with_status(200)
        .with_body(
            json!({
                "tasks": [{"id": TASK_ID, "state": "QUEUED"}],
                "next_page_token": "second page",
            })
            .to_string(),
        )
        .create();
    let base_path = format!("{}/ga4gh/tes/v1", mockito::server_url());
    let home = Home::new(&config_with_fields(&base_path));
    let output = home.run(
        &[
            "tes",
            "list",
            "-k",
            "alpha",
            "-k",
            "beta",
            "-v",
            "x/y",
            "-v",
            "z",
            "-t",
            "first page",
        ],
        None,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains(TASK_ID));
    assert!(stdout(&output).contains("Next page token: second page\n"));
    list.assert();
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
        .match_query(Matcher::UrlEncoded("view".into(), "BASIC".into()))
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"next"}"#)
        .expect(1)
        .create();
    let second = mock("GET", "/export/tasks")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("view".into(), "BASIC".into()),
            Matcher::UrlEncoded("page_token".into(), "next".into()),
        ]))
        .with_body(r#"{"tasks":[{"id":"two"}]}"#)
        .expect(1)
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/export",
        mockito::server_url()
    )));
    let output = home.run(
        &["tes", "list", "--all", "--format", "jsonl", "-w", "basic"],
        None,
    );
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

#[test]
fn malformed_creation_acknowledgements_fail_once_with_empty_stdout() {
    let _service = service_info_mock("/service-info");
    let home = Home::new(&config_with_fields(&mockito::server_url()));
    let task = home.path().join("task.json");
    for body in ["{}", r#"{"id":null}"#, r#"{"id":4}"#, r#"{"id":""}"#] {
        let request = mock("POST", "/tasks")
            .with_status(200)
            .with_body(body)
            .expect(1)
            .create();
        let output = home.run(&["tes", "create", task.to_str().unwrap()], None);
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("may have succeeded"));
        request.assert();
    }
}

/// A task ID from the server with a terminal sequence and a line break.
const CONTROL_ID: &str = "t1\u{1b}[2K\nforged";
/// `CONTROL_ID` as a path segment, as the SDK sends it.
const CONTROL_ID_PATH: &str = "t1%1B%5B2K%0Aforged";

/// Asserts that text output has no raw control or bidirectional characters.
fn assert_escaped(text: &str) {
    assert!(
        !text
            .chars()
            .any(|c| (c.is_control() && c != '\n')
                || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')),
        "raw control character in {text:?}"
    );
}

#[test]
fn server_text_is_escaped_in_text_output() {
    let prefix = "/escape";
    let _service = service_info_mock("/escape/service-info");
    let _create = mock("POST", "/escape/tasks")
        .with_status(200)
        .with_body(json!({"id": CONTROL_ID}).to_string())
        .create();
    let _list = mock("GET", Matcher::Regex(r"^/escape/tasks(\?.*)?$".into()))
        .with_status(200)
        .with_body(
            json!({
                "tasks": [{"id": CONTROL_ID, "state": "QUEUED"}],
                "next_page_token": "n\u{202E}x",
            })
            .to_string(),
        )
        .create();
    // Later requests use the original ID, not the escaped one.
    let status = mock(
        "GET",
        format!("{prefix}/tasks/{CONTROL_ID_PATH}?view=MINIMAL").as_str(),
    )
    .with_status(200)
    .with_body(json!({"id": CONTROL_ID, "state": "RUNNING"}).to_string())
    .create();
    let cancel = mock(
        "POST",
        format!("{prefix}/tasks/{CONTROL_ID_PATH}:cancel").as_str(),
    )
    .with_status(200)
    .with_body(json!({"note": "ok\u{1b}[31m"}).to_string())
    .create();
    let _failure = mock("GET", "/escape/tasks/broken?view=FULL")
        .with_status(500)
        .with_body("down\u{1b}]0;title\u{7}")
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}{prefix}",
        mockito::server_url()
    )));
    let task_file = home.path().join("task.json");

    let created = home.run(&["tes", "create", task_file.to_str().unwrap()], None);
    assert!(created.status.success(), "{}", stderr(&created));
    assert_eq!(stdout(&created), "t1\\u{1b}[2K\\nforged\n");

    let listed = home.run(&["tes", "list"], None);
    assert!(listed.status.success(), "{}", stderr(&listed));
    let table = stdout(&listed);
    assert_escaped(&table);
    assert!(table.contains("t1\\u{1b}[2K\\nforged"), "{table}");
    assert!(table.contains("Next page token: n\\u{202e}x\n"), "{table}");
    // Header, one task and the token; the table ends with an empty line.
    assert_eq!(
        table.lines().filter(|line| !line.is_empty()).count(),
        3,
        "{table}"
    );

    let state = home.run(&["tes", "status", CONTROL_ID], None);
    assert!(state.status.success(), "{}", stderr(&state));
    assert_eq!(stdout(&state), "RUNNING\n");
    status.assert();

    let canceled = home.run(&["tes", "cancel", CONTROL_ID], None);
    assert!(canceled.status.success(), "{}", stderr(&canceled));
    assert_escaped(&stdout(&canceled));
    assert_eq!(
        stdout(&canceled),
        "STATUS: Object {\"note\": String(\"ok\\u{1b}[31m\")}\n"
    );
    cancel.assert();

    let failed = home.run(&["tes", "get", "broken", "FULL"], None);
    assert_eq!(failed.status.code(), Some(1));
    let message = stderr(&failed);
    assert_escaped(&message);
    assert!(message.contains("down\\u{1b}]0;title\\u{7}"), "{message}");
}

#[test]
fn create_json_recovers_the_opaque_id_for_followup_requests() {
    let _service = service_info_mock("/create-json/service-info");
    let _create = mock("POST", "/create-json/tasks")
        .with_status(200)
        .with_body(json!({"id": CONTROL_ID}).to_string())
        .create();
    let status = mock(
        "GET",
        format!("/create-json/tasks/{CONTROL_ID_PATH}?view=MINIMAL").as_str(),
    )
    .with_status(200)
    .with_body(json!({"id": CONTROL_ID, "state": "RUNNING"}).to_string())
    .expect(1)
    .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/create-json",
        mockito::server_url()
    )));

    let created = home.run(
        &[
            "tes",
            "create",
            home.path().join("task.json").to_str().unwrap(),
            "--format",
            "json",
        ],
        None,
    );
    assert!(created.status.success(), "{}", stderr(&created));
    let create_json = stdout(&created);
    assert!(!create_json.contains('\u{1b}'));
    let id = serde_json::from_str::<Value>(&create_json).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(id, CONTROL_ID);
    let state = home.run(&["tes", "status", &id], None);
    assert!(state.status.success(), "{}", stderr(&state));
    status.assert();
}

#[test]
fn json_output_keeps_server_values() {
    let _service = service_info_mock("/escape-json/service-info");
    let task = json!({"id": CONTROL_ID, "state": "QUEUED", "name": "a\u{202E}b"});
    let body = task.to_string();
    let _get = mock(
        "GET",
        format!("/escape-json/tasks/{CONTROL_ID_PATH}?view=FULL").as_str(),
    )
    .with_status(200)
    .with_body(&body)
    .create();
    let _list = mock("GET", "/escape-json/tasks")
        .with_status(200)
        .with_body(json!({"tasks": [task.clone()]}).to_string())
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/escape-json",
        mockito::server_url()
    )));

    let got = home.run(&["tes", "get", CONTROL_ID, "FULL"], None);
    assert!(got.status.success(), "{}", stderr(&got));
    let got_json = stdout(&got);
    assert!(!got_json.contains('\u{202e}'));
    assert_eq!(serde_json::from_str::<Value>(&got_json).unwrap(), task);
    assert_eq!(got_json.replace("\\u202e", "\u{202e}"), body);

    let exported = home.run(&["tes", "list", "--all", "--format", "jsonl"], None);
    assert!(exported.status.success(), "{}", stderr(&exported));
    let record: Value = serde_json::from_str(stdout(&exported).trim()).unwrap();
    assert_eq!(record["id"], CONTROL_ID);
    assert_eq!(record["name"], "a\u{202E}b");
}

#[test]
fn get_reports_a_body_that_is_not_json() {
    let _service = service_info_mock("/escape-raw/service-info");
    let _get = mock("GET", "/escape-raw/tasks/t1?view=FULL")
        .with_status(200)
        .with_body("\u{1b}]0;title\u{7}ok")
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/escape-raw",
        mockito::server_url()
    )));
    let output = home.run(&["tes", "get", TASK_ID, "FULL"], None);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{}", stdout(&output));
    let message = stderr(&output);
    assert_escaped(&message);
    assert!(message.contains("not valid JSON"), "{message}");
}

#[test]
fn log_messages_are_escaped() {
    let _service = mock("GET", "/escape-log/service-info")
        .with_status(500)
        .with_body("down\u{202E}x\nforged")
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/escape-log",
        mockito::server_url()
    )));
    for log_level in LOG_LEVELS {
        let output = home.run(&["tes", "list"], log_level);
        assert_eq!(output.status.code(), Some(1), "{log_level:?}");
        let message = stderr(&output);
        assert_escaped(&message);
        assert!(
            !message.lines().any(|line| line.starts_with("forged")),
            "{log_level:?}: {message}"
        );
        assert!(message.contains("down\\u{202e}x\\nforged"), "{message}");
    }
}

#[test]
fn undefined_states_read_as_unknown_except_in_get_and_jsonl() {
    let _service = service_info_mock("/undefined-state/service-info");
    let task = json!({"id": TASK_ID, "state": "SUSPENDED"});
    let body = task.to_string();
    let _task = mock("GET", "/undefined-state/tasks/t1")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(&body)
        .create();
    let _list = mock("GET", "/undefined-state/tasks")
        .match_query(Matcher::Any)
        .with_status(200)
        .with_body(json!({"tasks": [task]}).to_string())
        .create();
    let home = Home::new(&config_with_fields(&format!(
        "{}/undefined-state",
        mockito::server_url()
    )));

    let state = home.run(&["tes", "status", TASK_ID], None);
    assert!(state.status.success(), "{}", stderr(&state));
    assert_eq!(stdout(&state), "UNKNOWN\n");
    let listed = home.run(&["tes", "list"], None);
    assert!(listed.status.success(), "{}", stderr(&listed));
    let table = stdout(&listed);
    assert!(
        table.contains(&format!("{TASK_ID:<25} {:<15}\n", "Unknown")),
        "{table}"
    );
    let got = home.run(&["tes", "get", TASK_ID, "FULL"], None);
    assert!(got.status.success(), "{}", stderr(&got));
    assert_eq!(stdout(&got), body);
    let exported = home.run(&["tes", "list", "--all", "--format", "jsonl"], None);
    assert!(exported.status.success(), "{}", stderr(&exported));
    let record: Value = serde_json::from_str(stdout(&exported).trim()).unwrap();
    assert_eq!(record["state"], "SUSPENDED");
}
