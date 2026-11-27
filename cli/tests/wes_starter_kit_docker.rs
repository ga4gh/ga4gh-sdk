//! Runs a workflow to completion through the `wes` commands of the CLI binary.
//!
//! Run through `tests/run-wes-docker-integration.sh`, which starts Starter
//! Kit WES and sets the environment variables read below.
#![cfg(all(unix, feature = "wes_docker_integration_tests"))]

use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is unset; run through tests/run-wes-docker-integration.sh")
    })
}

fn cli(home: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home.path())
        .current_dir(home.path())
        .output()
        .unwrap()
}

fn stdout(args: &[&str], output: &Output) -> String {
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn cli_runs_a_workflow_to_completion() {
    let base_path = env("WES_DOCKER_BASE_URL");
    let revision = env("WES_DOCKER_FIXTURE_REVISION");
    let run_timeout = std::env::var("WES_RUN_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        json!({"WES": {"base_path": base_path}}).to_string(),
    )
    .unwrap();
    fs::write(
        home.path().join("params.json"),
        json!({"sleep_seconds": 0, "message": "cli-integration"}).to_string(),
    )
    .unwrap();
    let workflow_url = format!("https://github.com/ga4gh-sdk/wes-fixture/tree/{revision}");

    let args = [
        "wes",
        "run",
        workflow_url.as_str(),
        "--type",
        "NEXTFLOW",
        "--type_version",
        "21.04.0",
        "--params",
        "params.json",
    ];
    let run_id = stdout(&args, &cli(&home, &args)).trim().to_string();
    assert!(!run_id.is_empty());
    println!("submitted run {run_id}");

    // Starter Kit 0.2.0 returns a fixed list, so this checks only the format.
    let args = ["wes", "list"];
    assert!(stdout(&args, &cli(&home, &args)).starts_with("RUN ID"));

    let deadline = Instant::now() + Duration::from_secs(run_timeout);
    let mut state = String::new();
    while Instant::now() < deadline {
        // Starter Kit 0.2.0 answers 400 until the run's state is stored,
        // so a failed status request is retried like a pending state.
        let output = cli(&home, &["wes", "status", run_id.as_str()]);
        state = if output.status.success() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            String::from_utf8_lossy(&output.stderr).trim().to_string()
        };
        match state.as_str() {
            "COMPLETE" => break,
            "EXECUTOR_ERROR" | "SYSTEM_ERROR" | "CANCELED" | "PREEMPTED" => break,
            _ => std::thread::sleep(Duration::from_secs(2)),
        }
    }
    let args = ["wes", "get", run_id.as_str()];
    let log: Value = serde_json::from_str(&stdout(&args, &cli(&home, &args))).unwrap();
    assert_eq!(state, "COMPLETE", "run log: {log:#}");
    assert_eq!(log["run_id"], run_id.as_str());
    assert_eq!(log["state"], "COMPLETE");
    assert_eq!(log["request"]["workflow_url"], workflow_url.as_str());
    assert_eq!(log["run_log"]["exit_code"], 0);
    // The workflow writes its `message` parameter to its output.
    let result_url = log["outputs"]["result.txt"]
        .as_str()
        .expect("completed workflow exposes result.txt");
    let result_path = url::Url::parse(result_url)
        .expect("output is a URL")
        .to_file_path()
        .expect("output is a local file URL");
    assert_eq!(
        fs::read_to_string(result_path).unwrap().trim(),
        "cli-integration"
    );
    println!("completed run {run_id}");
}
