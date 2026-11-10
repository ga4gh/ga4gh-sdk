//! Checks the `wes` commands of the CLI binary against a mock WES server.

// The CLI finds its config file through `HOME`, which Windows doesn't use.
#![cfg(unix)]

use mockito::{mock, Matcher, Mock};
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::process::{Command, Output, Stdio};
use tempfile::TempDir;

/// A home directory whose config file has a `WES` section for `prefix`.
fn home_with_wes_config(prefix: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".ga4gh-cli");
    fs::create_dir(&directory).unwrap();
    let base_path = format!("{}{prefix}", mockito::server_url());
    fs::write(
        directory.join("config.json"),
        json!({"WES": {"base_path": base_path}}).to_string(),
    )
    .unwrap();
    home
}

fn command(home: &TempDir, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
    command
        .args(args)
        .env("HOME", home.path())
        .current_dir(home.path());
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
    command
}

fn run(home: &TempDir, args: &[&str]) -> Output {
    command(home, args).output().unwrap()
}

fn run_with_stdin(home: &TempDir, args: &[&str], input: &str) -> Output {
    let mut child = command(home, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn service_info(prefix: &str) -> Mock {
    mock("GET", format!("{prefix}/service-info").as_str())
        .with_status(200)
        .with_body(
            json!({
                "id": "org.example.wes",
                "name": "Example WES",
                "type": {"group": "org.ga4gh", "artifact": "wes", "version": "1.1.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0",
            })
            .to_string(),
        )
        .create()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_success(output: &Output) {
    assert!(output.status.success(), "{}", stderr(output));
    assert!(output.stderr.is_empty(), "{}", stderr(output));
}

/// Input errors fail before any request and don't panic.
fn assert_input_error(output: &Output, expected: &str) {
    let message = stderr(output);
    assert!(!output.status.success(), "{message}");
    assert!(output.stdout.is_empty(), "{}", stdout(output));
    assert!(message.contains(expected), "{message}");
    assert!(!message.contains("panicked"), "{message}");
}

const RUN_ARGS: [&str; 6] = [
    "wes",
    "run",
    "https://example.org/workflow.cwl",
    "--type",
    "CWL",
    "--type_version",
];

#[test]
fn run_submits_a_multipart_request_and_prints_the_run_id() {
    let prefix = "/run/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let submit = mock("POST", format!("{prefix}/runs").as_str())
        .match_header(
            "content-type",
            Matcher::Regex("^multipart/form-data; boundary=".into()),
        )
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex(r#"name="workflow_type"\s+CWL\s"#.into()),
            Matcher::Regex(r#"name="workflow_type_version"\s+v1.2\s"#.into()),
            Matcher::Regex(r#"name="workflow_url"\s+https://example.org/workflow.cwl\s"#.into()),
            Matcher::Regex(r#"name="workflow_params"\s+\{"reads":"a.fastq"\}\s"#.into()),
            Matcher::Regex(r#"name="tags"\s+\{"project":"demo=1"\}\s"#.into()),
            Matcher::Regex(r#"name="workflow_engine"\s+cwltool\s"#.into()),
            Matcher::Regex(r#"name="workflow_engine_version"\s+3.1\s"#.into()),
            Matcher::Regex(r#"name="workflow_engine_parameters"\s+\{"--debug":"true"\}\s"#.into()),
            Matcher::Regex(
                r#"name="workflow_attachment"; filename="tools/step.cwl"[\s\S]*cwlVersion: v1.2"#
                    .into(),
            ),
        ]))
        .with_status(200)
        .with_body(r#"{"run_id":"run-1"}"#)
        .expect(1)
        .create();
    let home = home_with_wes_config(prefix);
    fs::create_dir(home.path().join("tools")).unwrap();
    fs::write(home.path().join("tools/step.cwl"), "cwlVersion: v1.2\n").unwrap();
    fs::write(home.path().join("params.json"), r#"{"reads": "a.fastq"}"#).unwrap();

    let mut args = RUN_ARGS.to_vec();
    args.extend([
        "v1.2",
        "--params",
        "params.json",
        "--attachment",
        "tools/step.cwl",
        "--tag",
        "project=demo=1",
        "--engine",
        "cwltool",
        "--engine_version",
        "3.1",
        "--engine_parameter",
        "--debug=true",
    ]);
    let output = run(&home, &args);
    assert_success(&output);
    assert_eq!(stdout(&output), "run-1\n");
    submit.assert();
}

#[test]
fn run_reads_params_inline_from_stdin_or_defaults_to_an_empty_object() {
    let prefix = "/params/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let with_params = mock("POST", format!("{prefix}/runs").as_str())
        .match_body(Matcher::Regex(
            r#"name="workflow_params"\s+\{"n":1\}\s"#.into(),
        ))
        .with_status(200)
        .with_body(r#"{"run_id":"with-params"}"#)
        .expect(2)
        .create();
    let without_params = mock("POST", format!("{prefix}/runs").as_str())
        .match_body(Matcher::Regex(r#"name="workflow_params"\s+\{\}\s"#.into()))
        .with_status(200)
        .with_body(r#"{"run_id":"empty-params"}"#)
        .expect(1)
        .create();
    let home = home_with_wes_config(prefix);

    let mut inline = RUN_ARGS.to_vec();
    inline.extend(["v1.2", "--params", r#"{"n": 1}"#]);
    let output = run(&home, &inline);
    assert_success(&output);
    assert_eq!(stdout(&output), "with-params\n");

    let mut piped = RUN_ARGS.to_vec();
    piped.extend(["v1.2", "--params", "-"]);
    let output = run_with_stdin(&home, &piped, r#"{"n": 1}"#);
    assert_success(&output);
    assert_eq!(stdout(&output), "with-params\n");

    let mut defaults = RUN_ARGS.to_vec();
    defaults.push("v1.2");
    let output = run(&home, &defaults);
    assert_success(&output);
    assert_eq!(stdout(&output), "empty-params\n");
    with_params.assert();
    without_params.assert();
}

#[test]
fn run_input_errors_are_reported_before_any_request() {
    // No mocks: any request to the server would fail with a different message.
    let home = home_with_wes_config("/input-errors/ga4gh/wes/v1");
    fs::write(home.path().join("list.json"), "[1, 2]").unwrap();
    fs::write(home.path().join("broken.json"), "{").unwrap();
    let with = |extra: &[&'static str]| {
        let mut args = RUN_ARGS.to_vec();
        args.push("v1.2");
        args.extend(extra);
        args
    };
    let cases: Vec<(Vec<&str>, &str)> = vec![
        (with(&["--params", "broken.json"]), "Failed to parse JSON"),
        (with(&["--params", "list.json"]), "must be a JSON object"),
        (
            with(&["--params", "missing.json"]),
            "File does not exist: missing.json",
        ),
        (
            with(&["--tag", "no-separator"]),
            "Invalid tag 'no-separator'",
        ),
        (with(&["--tag", "=value"]), "Invalid tag '=value'"),
        (
            with(&["--engine_parameter", "flag"]),
            "Invalid engine_parameter 'flag'",
        ),
        (
            with(&["--attachment", "missing.cwl"]),
            "Cannot read attachment 'missing.cwl'",
        ),
        (
            with(&["--attachment", "/etc/hostname"]),
            "safe relative path",
        ),
        (
            with(&["--attachment", "../outside.cwl"]),
            "safe relative path",
        ),
        (with(&["--attachment", "."]), "safe relative path"),
        (
            with(&["--attachment", "list.json", "--attachment", "list.json"]),
            "Duplicate attachment 'list.json'",
        ),
    ];
    for (args, expected) in cases {
        assert_input_error(&run(&home, &args), expected);
    }
}

#[test]
fn run_requires_type_and_version_and_missing_config_is_an_error() {
    let home = home_with_wes_config("/usage/ga4gh/wes/v1");
    let usage = run(&home, &RUN_ARGS[..5]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(
        stderr(&usage).contains("--type_version"),
        "{}",
        stderr(&usage)
    );

    let empty_home = TempDir::new().unwrap();
    let missing = run(&empty_home, &["wes", "status", "run-1"]);
    assert_input_error(&missing, ".ga4gh-cli/config.json");
    let tes_only = TempDir::new().unwrap();
    fs::create_dir(tes_only.path().join(".ga4gh-cli")).unwrap();
    fs::write(
        tes_only.path().join(".ga4gh-cli/config.json"),
        json!({"TES": {"base_path": "https://example.org"}}).to_string(),
    )
    .unwrap();
    assert_input_error(&run(&tes_only, &["wes", "list"]), "'WES' object");
}

#[test]
fn run_reports_rejected_submissions_and_wrong_services() {
    let prefix = "/rejected/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let submit = mock("POST", format!("{prefix}/runs").as_str())
        .with_status(400)
        .with_body(r#"{"msg":"unsupported workflow type","status_code":400}"#)
        .expect(1)
        .create();
    let home = home_with_wes_config(prefix);
    let mut args = RUN_ARGS.to_vec();
    args.push("v1.2");
    let output = run(&home, &args);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(
        stderr(&output).contains("unsupported workflow type"),
        "{}",
        stderr(&output)
    );
    submit.assert();

    let missing = home_with_wes_config("/no-wes-here");
    let _not_found = mock("GET", "/no-wes-here/service-info")
        .with_status(404)
        .create();
    let output = run(&missing, &args);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("base_path is the WES API root"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn list_prints_a_table_with_the_next_page_token() {
    let prefix = "/list/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let list = mock("GET", format!("{prefix}/runs").as_str())
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("page_size".into(), "2".into()),
            Matcher::UrlEncoded("page_token".into(), "a+/ %".into()),
        ]))
        .with_status(200)
        .with_body(
            json!({
                "runs": [
                    {"run_id": "3f2a0b6e-58c2-4e8a-9d1e-0c5b7a1d2e3f", "state": "RUNNING"},
                    {"run_id": "short\u{1b}[2K", "state": "EXECUTOR_ERROR"},
                    {"run_id": "no-state"},
                ],
                "next_page_token": "next",
            })
            .to_string(),
        )
        .expect(1)
        .create();
    let home = home_with_wes_config(prefix);
    let output = run(&home, &["wes", "list", "-p", "2", "-t", "a+/ %"]);
    assert_success(&output);
    assert_eq!(
        stdout(&output),
        "RUN ID                                STATE\n\
         3f2a0b6e-58c2-4e8a-9d1e-0c5b7a1d2e3f  RUNNING\n\
         short\\u{1b}[2K                        EXECUTOR_ERROR\n\
         no-state                              UNKNOWN\n\
         Next page token: next\n"
    );
    list.assert();
}

#[test]
fn list_all_streams_jsonl_and_rejects_mixed_formats() {
    let prefix = "/export/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let first = mock("GET", format!("{prefix}/runs").as_str())
        .match_query(Matcher::Missing)
        .with_body(r#"{"runs":[{"run_id":"one","custom":true}],"next_page_token":"next"}"#)
        .expect(1)
        .create();
    let second = mock("GET", format!("{prefix}/runs").as_str())
        .match_query(Matcher::UrlEncoded("page_token".into(), "next".into()))
        .with_body(r#"{"runs":[{"run_id":"two"}]}"#)
        .expect(1)
        .create();
    let home = home_with_wes_config(prefix);
    let output = run(&home, &["wes", "list", "--all", "--format", "jsonl"]);
    assert_success(&output);
    let records = stdout(&output)
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["run_id"], "one");
    assert_eq!(records[0]["custom"], true);
    assert_eq!(records[1]["run_id"], "two");
    first.assert();
    second.assert();

    let limited = run(
        &home,
        &[
            "wes",
            "list",
            "--all",
            "--format",
            "jsonl",
            "--max-items",
            "1",
        ],
    );
    assert_eq!(limited.status.code(), Some(1));
    assert_eq!(stdout(&limited).lines().count(), 1);
    assert!(
        stderr(&limited).contains("resume checkpoint"),
        "{}",
        stderr(&limited)
    );

    for (args, expected) in [
        (
            vec!["wes", "list", "--all"],
            "--all requires --format jsonl",
        ),
        (
            vec!["wes", "list", "--format", "jsonl"],
            "--format jsonl requires --all",
        ),
        (
            vec!["wes", "list", "-p", "many"],
            "Invalid page_size 'many'",
        ),
    ] {
        assert_input_error(&run(&home, &args), expected);
    }
}

#[test]
fn get_status_and_cancel_encode_ids_and_print_results() {
    let prefix = "/run-id/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let run_id = "a/b ?#";
    let log_body = json!({
        "run_id": run_id,
        "state": "COMPLETE",
        "outputs": {"result.txt": "file:///result.txt"},
        "vendor_field": {"kept": true},
    })
    .to_string();
    let log = mock("GET", format!("{prefix}/runs/a%2Fb%20%3F%23").as_str())
        .with_status(200)
        .with_body(&log_body)
        .expect(1)
        .create();
    let status = mock(
        "GET",
        format!("{prefix}/runs/a%2Fb%20%3F%23/status").as_str(),
    )
    .with_status(200)
    .with_body(json!({"run_id": run_id, "state": "COMPLETE"}).to_string())
    .expect(1)
    .create();
    let cancel = mock(
        "POST",
        format!("{prefix}/runs/a%2Fb%20%3F%23/cancel").as_str(),
    )
    .with_status(200)
    .with_body(json!({"run_id": run_id}).to_string())
    .expect(1)
    .create();
    let home = home_with_wes_config(prefix);

    let get = run(&home, &["wes", "get", run_id]);
    assert_success(&get);
    assert_eq!(stdout(&get), log_body);
    let state = run(&home, &["wes", "status", run_id]);
    assert_success(&state);
    assert_eq!(stdout(&state), "COMPLETE\n");
    let canceled = run(&home, &["wes", "cancel", run_id]);
    assert_success(&canceled);
    assert_eq!(stdout(&canceled), format!("{run_id}\n"));
    log.assert();
    status.assert();
    cancel.assert();
}

#[test]
fn service_failures_have_nonzero_exit_codes() {
    let prefix = "/failures/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let _log = mock("GET", format!("{prefix}/runs/missing").as_str())
        .with_status(404)
        .with_body("run not found")
        .create();
    let _status = mock("GET", format!("{prefix}/runs/missing/status").as_str())
        .with_status(404)
        .with_body("status not found")
        .create();
    let _cancel = mock("POST", format!("{prefix}/runs/done/cancel").as_str())
        .with_status(200)
        .with_body("null")
        .create();
    let _list = mock("GET", format!("{prefix}/runs").as_str())
        .with_status(503)
        .with_body("list unavailable")
        .create();
    let home = home_with_wes_config(prefix);
    for (args, expected) in [
        (vec!["wes", "get", "missing"], "run not found"),
        (vec!["wes", "status", "missing"], "status not found"),
        (
            vec!["wes", "cancel", "done"],
            "cancellation cannot be confirmed",
        ),
        (vec!["wes", "list"], "list unavailable"),
    ] {
        let output = run(&home, &args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {}", stdout(&output));
        assert!(
            stderr(&output).contains(expected),
            "{args:?}: {}",
            stderr(&output)
        );
    }
}

#[test]
fn list_all_escapes_server_text_in_records_and_errors() {
    let prefix = "/export-escape/ga4gh/wes/v1";
    let _service = service_info(prefix);
    let first = mock("GET", format!("{prefix}/runs").as_str())
        .match_query(Matcher::Missing)
        .with_body(
            json!({"runs": [{"run_id": "a\u{202E}b\u{9b}c"}], "next_page_token": "next"})
                .to_string(),
        )
        .expect(1)
        .create();
    let second = mock("GET", format!("{prefix}/runs").as_str())
        .match_query(Matcher::UrlEncoded("page_token".into(), "next".into()))
        .with_status(503)
        .with_body("down\u{1b}]0;title\u{7}")
        .expect(1)
        .create();
    let home = home_with_wes_config(prefix);
    let output = run(&home, &["wes", "list", "--all", "--format", "jsonl"]);
    assert_eq!(output.status.code(), Some(1));
    let records = stdout(&output);
    assert!(
        !records.contains('\u{202E}') && !records.contains('\u{9b}'),
        "{records}"
    );
    let record: Value = serde_json::from_str(records.trim()).unwrap();
    assert_eq!(record["run_id"], "a\u{202E}b\u{9b}c");
    let diagnostics = stderr(&output);
    assert!(
        diagnostics.contains(r"down\u{1b}]0;title\u{7}"),
        "{diagnostics}"
    );
    assert!(!diagnostics.contains('\u{1b}'), "{diagnostics}");
    first.assert();
    second.assert();
}

#[test]
fn empty_run_ids_are_usage_errors() {
    let home = home_with_wes_config("/empty-id/ga4gh/wes/v1");
    for command in ["get", "status", "cancel"] {
        let output = run(&home, &["wes", command, ""]);
        assert_eq!(output.status.code(), Some(2), "{command}");
        assert!(output.stdout.is_empty(), "{command}");
    }
}
