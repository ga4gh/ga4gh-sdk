//! Runs tasks through all `tes` commands of the CLI binary against proTES,
//! a TES gateway that requires access tokens and relays tasks to Funnel.
//!
//! The CLI takes the access token from its configuration file. Run through
//! `tests/run-protes-integration.sh`, which starts the services and sets
//! `PROTES_TES_URL` and `PROTES_ACCESS_TOKEN`. The task helpers follow
//! `funnel_cli.rs`; keep both suites in sync.
#![cfg(all(unix, feature = "protes_integration_tests"))]

use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Polling deadline, including the image pull; in-flight requests can extend it.
const STATE_TIMEOUT: Duration = Duration::from_secs(180);
const TERMINAL_STATES: [&str; 5] = [
    "COMPLETE",
    "EXECUTOR_ERROR",
    "SYSTEM_ERROR",
    "CANCELED",
    "PREEMPTED",
];

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set; run this suite with tests/run-protes-integration.sh")
    })
}

/// A home directory with a CLI config for the TES API of proTES; with
/// `token`, the config holds it as an OAuth access token.
fn home(token: Option<&str>) -> TempDir {
    let mut section = json!({"base_path": env("PROTES_TES_URL")});
    if let Some(token) = token {
        section["oauth_access_token"] = token.into();
    }
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        json!({ "TES": section }).to_string(),
    )
    .unwrap();
    home
}

/// A home directory whose config holds the access token of a test user.
fn authorized_home() -> TempDir {
    home(Some(&env("PROTES_ACCESS_TOKEN")))
}

fn cli(home: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home.path())
        // The token must come from the config file only, without a profile,
        // and go to proTES rather than a proxy (as in registry.rs)
        .env_remove("PROTES_ACCESS_TOKEN")
        .env_remove("GA4GH_CLI_PROFILE")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .current_dir(home.path())
        .output()
        .unwrap()
}

/// The standard output of a command that must succeed.
fn stdout(home: &TempDir, args: &[&str]) -> String {
    let output = cli(home, args);
    assert!(
        output.status.success(),
        "{args:?} failed (is proTES running? see tests/run-protes-integration.sh): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Writes a task without inputs or outputs to `task.json` of `home`.
fn write_task(home: &TempDir, name: &str, command: &[&str]) {
    let task = json!({
        "name": name,
        "tags": {"sdk.suite": name},
        "executors": [{"image": "alpine:3.22", "command": command}],
    });
    fs::write(home.path().join("task.json"), task.to_string()).unwrap();
}

/// Creates a task without inputs or outputs from a task file; returns its ID.
fn create(home: &TempDir, name: &str, command: &[&str]) -> String {
    write_task(home, name, command);
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

/// Asserts that a command failed with HTTP 401 and printed nothing to stdout.
fn assert_unauthorized(home: &TempDir, args: &[&str]) {
    let output = cli(home, args);
    assert!(!output.status.success(), "{args:?} succeeded: {output:?}");
    assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // As `HttpError` prints the status, so that a 401 elsewhere in the
    // message (e.g. in a task ID) doesn't count
    assert!(stderr.contains("status: 401"), "{args:?}: {stderr}");
}

#[test]
fn task_runs_to_completion_with_the_configured_token() {
    let home = authorized_home();
    let id = create(&home, "cli-protes-complete", &["echo", "hello from proTES"]);

    let state = await_state(&home, &id, &TERMINAL_STATES);
    let task: Value = serde_json::from_str(&stdout(&home, &["tes", "get", &id, "FULL"])).unwrap();
    assert_eq!(state, "COMPLETE", "task: {task:#}");
    assert_eq!(task["id"], id.as_str());
    assert_eq!(task["state"], "COMPLETE");
    assert_eq!(
        task["logs"][0]["logs"][0]["stdout"], "hello from proTES\n",
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
            "cli-protes-complete",
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

    // A positive match alone would also pass if the filter were dropped
    let no_match = format!("no-match-{id}");
    let empty = stdout(
        &home,
        &[
            "tes",
            "list",
            "--tag_key",
            "sdk.suite",
            "--tag_value",
            &no_match,
            "--view",
            "MINIMAL",
        ],
    );
    assert!(empty.starts_with("TASK ID"), "{empty}");
    assert!(
        empty.lines().skip(1).all(|line| line.trim().is_empty()),
        "nonmatching tag filter returned tasks: {empty}"
    );
}

#[test]
fn canceled_task_ends_canceled_with_the_configured_token() {
    let home = authorized_home();
    let id = create(&home, "cli-protes-cancel", &["sleep", "120"]);
    // Cancel a task whose executor runs, not one that is still queued
    await_state(&home, &id, &["RUNNING"]);

    let output = stdout(&home, &["tes", "cancel", &id]);
    assert!(output.starts_with("STATUS: "), "{output}");
    assert_eq!(await_state(&home, &id, &TERMINAL_STATES), "CANCELED");
}

#[test]
fn commands_without_a_configured_token_are_unauthorized() {
    // A task of the test user, so that the task commands below address an
    // existing task and fail only for the missing token; it is left to
    // finish, and the script removes the services afterwards
    let authorized = authorized_home();
    let id = create(&authorized, "cli-protes-anonymous", &["true"]);

    let anonymous = home(None);
    write_task(&anonymous, "cli-protes-anonymous", &["true"]);
    for args in [
        &["tes", "create", "task.json"][..],
        &["tes", "status", &id],
        &["tes", "get", &id, "MINIMAL"],
        &["tes", "list"],
        &["tes", "cancel", &id],
    ] {
        assert_unauthorized(&anonymous, args);
    }
}
