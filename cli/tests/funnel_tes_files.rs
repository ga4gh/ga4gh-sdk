//! Lists, shows and saves the stdout of a task run by Funnel, through the
//! `tes files`, `tes log` and `tes save` commands of the CLI binary.
//!
//! Run through `run-integration-tests.sh`, which starts Funnel on
//! `localhost:8000`.
#![cfg(all(unix, feature = "integration_tests"))]

use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn cli(home: &TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn saves_the_stdout_of_a_funnel_task() {
    let home = TempDir::new().unwrap();
    let config_dir = home.path().join(".ga4gh-cli");
    fs::create_dir(&config_dir).unwrap();
    let config = json!({"TES": {"base_path": "http://localhost:8000/ga4gh/tes/v1"}});
    fs::write(config_dir.join("config.json"), config.to_string()).unwrap();
    let task = json!({
        "name": "tes files integration test",
        "executors": [{"image": "alpine", "command": ["sh", "-c", "echo saved by ga4gh-cli"]}]
    });

    let created = cli(
        &home,
        &["tes", "create", "--format", "json", &task.to_string()],
    );
    assert!(created.status.success(), "{}", text(&created.stderr));
    let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Funnel may add the executor log shortly after the final state.
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let output = cli(&home, &["tes", "files", &id]);
        assert!(output.status.success(), "{}", text(&output.stderr));
        let notes = text(&output.stderr);
        if text(&output.stdout).contains("\nstdout:0:0 ") && !notes.contains("may be incomplete") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no stdout before the deadline: {notes}"
        );
        std::thread::sleep(Duration::from_secs(2));
    }

    let shown = cli(&home, &["tes", "log", &id, "stdout:0:0"]);
    assert!(shown.status.success(), "{}", text(&shown.stderr));
    assert_eq!(text(&shown.stdout), "saved by ga4gh-cli\n");

    let dir = home.path().join("results");
    let saved = cli(
        &home,
        &[
            "tes",
            "save",
            &id,
            "stdout:0:0",
            "--dir",
            dir.to_str().unwrap(),
        ],
    );
    assert!(saved.status.success(), "{}", text(&saved.stderr));
    assert_eq!(
        fs::read_to_string(dir.join("stdout-0-0.txt")).unwrap(),
        "saved by ga4gh-cli\n"
    );
}
