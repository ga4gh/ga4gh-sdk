//! Runs tasks through all `tes` commands of the CLI binary against Funnel.
//!
//! Run through `run-integration-tests.sh`, which starts Funnel at
//! `localhost:8000`, as for the Funnel tests of the library.
#![cfg(all(unix, feature = "integration_tests"))]

use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const FUNNEL_TES_URL: &str = "http://localhost:8000/ga4gh/tes/v1";
/// Time limit for a task to reach a state, including the image pull.
const STATE_TIMEOUT: Duration = Duration::from_secs(180);
const TERMINAL_STATES: [&str; 5] = [
    "COMPLETE",
    "EXECUTOR_ERROR",
    "SYSTEM_ERROR",
    "CANCELED",
    "PREEMPTED",
];

/// A home directory with a CLI config for the TES API of Funnel.
fn home() -> TempDir {
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        json!({"TES": {"base_path": FUNNEL_TES_URL}}).to_string(),
    )
    .unwrap();
    home
}

fn cli(home: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home.path())
        .current_dir(home.path())
        .output()
        .unwrap()
}

/// The standard output of a command that must succeed.
fn stdout(home: &TempDir, args: &[&str]) -> String {
    let output = cli(home, args);
    assert!(
        output.status.success(),
        "{args:?} failed (is Funnel running? see run-integration-tests.sh): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Creates a task without inputs or outputs from a task file; returns its ID.
fn create(home: &TempDir, name: &str, command: &[&str]) -> String {
    let task = json!({
        "name": name,
        "tags": {"sdk.suite": name},
        "executors": [{"image": "alpine:3.22", "command": command}],
    });
    fs::write(home.path().join("task.json"), task.to_string()).unwrap();
    let id = stdout(home, &["tes", "create", "task.json"])
        .trim()
        .to_string();
    assert!(!id.is_empty(), "no task ID");
    id
}

/// Polls `tes status` until the task has one of `states`; returns that state.
fn await_state(home: &TempDir, id: &str, states: &[&str]) -> String {
    let deadline = Instant::now() + STATE_TIMEOUT;
    loop {
        let state = stdout(home, &["tes", "status", id]).trim().to_string();
        if states.contains(&state.as_str()) {
            return state;
        }
        assert!(
            !TERMINAL_STATES.contains(&state.as_str()),
            "task {id} ended {state}, expected one of {states:?}"
        );
        assert!(
            Instant::now() < deadline,
            "task {id} still {state} after {STATE_TIMEOUT:?}"
        );
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn task_runs_to_completion() {
    let home = home();
    let id = create(&home, "cli-complete", &["echo", "hello from the CLI"]);

    let state = await_state(&home, &id, &TERMINAL_STATES);
    let task: Value = serde_json::from_str(&stdout(&home, &["tes", "get", &id, "FULL"])).unwrap();
    assert_eq!(state, "COMPLETE", "task: {task:#}");
    assert_eq!(task["id"], id.as_str());
    assert_eq!(task["state"], "COMPLETE");
    assert_eq!(task["name"], "cli-complete");
    assert_eq!(
        task["logs"][0]["logs"][0]["stdout"], "hello from the CLI\n",
        "task: {task:#}"
    );

    let table = stdout(
        &home,
        &[
            "tes",
            "list",
            "--tag_key",
            "sdk.suite",
            "--tag_value",
            "cli-complete",
            "--view",
            "MINIMAL",
        ],
    );
    // The table format itself is checked against a mock in output.rs
    assert!(table.starts_with("TASK ID"), "{table}");
    assert!(
        table
            .lines()
            .any(|line| line.starts_with(&id) && line.contains("Complete")),
        "task {id} not listed as complete: {table}"
    );
}

#[test]
fn canceled_task_ends_canceled() {
    let home = home();
    let id = create(&home, "cli-cancel", &["sleep", "600"]);
    // Cancel a task whose executor runs, not one that is still queued
    await_state(&home, &id, &["RUNNING"]);

    let output = stdout(&home, &["tes", "cancel", &id]);
    assert!(output.starts_with("STATUS: "), "{output}");
    assert_eq!(await_state(&home, &id, &TERMINAL_STATES), "CANCELED");
}

#[test]
fn unknown_task_is_an_error() {
    let home = home();
    let output = cli(&home, &["tes", "get", "no-such-task", "MINIMAL"]);
    assert!(!output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    // Funnel answers, with 404, rather than the request failing
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("404"), "{stderr}");
}
