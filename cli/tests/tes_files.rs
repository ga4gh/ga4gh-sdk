//! `tes files`, `tes log` and `tes save` against a mock TES, mock HTTP and S3
//! storage, and servers that break off or stall a download.

// The CLI locates its config through HOME, which Windows does not use.
#![cfg(unix)]

use mockito::{mock, Matcher, Mock};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;

/// Bytes that aren't valid UTF-8, to check that files are saved unchanged.
const BINARY: &[u8] = b"\x00\xff\xfe binary \x80\n";

fn home_with_tes(prefix: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".ga4gh-cli");
    fs::create_dir(&directory).unwrap();
    let config = json!({"TES": {
        "base_path": format!("{}{prefix}", mockito::server_url()),
        "basic_auth": {"username": "user", "password": "password"},
        "oauth_access_token": "tes-token",
    }});
    fs::write(directory.join("config.json"), config.to_string()).unwrap();
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
        "NO_PROXY",
        "no_proxy",
    ] {
        command.env_remove(proxy);
    }
    // S3 requests go to the mock server, unsigned.
    for (key, _) in std::env::vars() {
        if key.starts_with("AWS_") {
            command.env_remove(key);
        }
    }
    command
        .env("AWS_ENDPOINT", mockito::server_url())
        .env("AWS_ALLOW_HTTP", "true")
        .env("AWS_SKIP_SIGNATURE", "true")
        .env("AWS_REGION", "us-east-1");
    command.output().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Mocks the service info and the `FULL` view of task `t1` below `prefix`.
fn tes_mocks(prefix: &str, task: &Value) -> Vec<Mock> {
    let service_info = json!({
        "id": "org.example.tes",
        "name": "Mock",
        "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0",
    });
    vec![
        mock("GET", format!("{prefix}/service-info").as_str())
            .with_status(200)
            .with_body(service_info.to_string())
            .create(),
        mock("GET", format!("{prefix}/tasks/t1?view=FULL").as_str())
            .with_status(200)
            .with_body(task.to_string())
            .create(),
    ]
}

/// A complete task with inline logs and HTTP and S3 output files. The size of
/// `file:0:2` is wrong on purpose.
fn complete_task(prefix: &str) -> Value {
    let storage = format!("{}{prefix}/storage", mockito::server_url());
    json!({
        "id": "t1",
        "state": "COMPLETE",
        "outputs": [
            {"url": format!("{storage}/out.bin"), "path": "/outputs/out.bin", "type": "FILE"},
            {"url": "s3://bucket/dir", "path": "/outputs/dir", "type": "DIRECTORY"},
        ],
        "logs": [{
            "logs": [{"exit_code": 0, "stdout": "line 1\nline 2 \u{1b}[31mred\n", "stderr": "warning\n"}],
            "system_logs": ["image pulled"],
            "outputs": [
                {"url": format!("{storage}/out.bin"), "path": "/outputs/out.bin", "size_bytes": BINARY.len().to_string()},
                {"url": format!("s3://bucket{prefix}/result.bin"), "path": "/outputs/result.bin", "size_bytes": "6"},
                {"url": format!("{storage}/short.txt"), "path": "/outputs/short.txt", "size_bytes": "100"},
                {"url": format!("{storage}/missing.txt?X-Amz-Signature=SECRET"), "path": "/outputs/missing.txt", "size_bytes": "1"},
                {"url": "gs://bucket/object.txt", "path": "/outputs/object.txt", "size_bytes": "1"},
                {"url": "https://exa mple.org/bad", "path": "/outputs/bad.txt", "size_bytes": "1"},
                {"url": format!("s3://bucket{prefix}/absent.bin"), "path": "/outputs/absent.bin", "size_bytes": "1"},
                {"url": format!("{storage}/x"), "path": "/outputs/..", "size_bytes": "1"},
            ]
        }]
    })
}

fn storage_mocks(prefix: &str) -> Vec<Mock> {
    vec![
        // No TES credentials may reach the storage host.
        mock("GET", format!("{prefix}/storage/out.bin").as_str())
            .match_header("authorization", Matcher::Missing)
            .with_status(200)
            .with_body(BINARY)
            .create(),
        mock("GET", format!("{prefix}/storage/short.txt").as_str())
            .with_status(200)
            .with_body("short")
            .create(),
        mock(
            "GET",
            Matcher::Regex(format!("^{prefix}/storage/missing.txt")),
        )
        .with_status(403)
        .with_body("denied")
        .create(),
        mock("GET", format!("/bucket{prefix}/result.bin").as_str())
            .with_status(200)
            .with_header("ETag", "\"abc\"")
            .with_header("Last-Modified", "Wed, 11 Nov 2026 10:00:00 GMT")
            .with_body(b"s3\x00\xffok")
            .create(),
        mock("GET", format!("/bucket{prefix}/absent.bin").as_str())
            .with_status(404)
            .with_body("<Error><Code>NoSuchKey</Code></Error>")
            .create(),
    ]
}

fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn files_lists_selectors_and_hides_url_secrets() {
    let prefix = "/files-list";
    let _tes = tes_mocks(prefix, &complete_task(prefix));
    let home = home_with_tes(prefix);
    let output = run(&home, &["tes", "files", "t1"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for selector in [
        "output:0",
        "output:1",
        "stdout:0:0",
        "stderr:0:0",
        "system:0",
        "file:0:0",
        "file:0:7",
    ] {
        assert!(
            text.contains(&format!("\n{selector} ")),
            "{selector}: {text}"
        );
    }
    assert!(
        text.contains("declared output directory: s3://bucket/dir"),
        "{text}"
    );
    assert!(
        text.contains("missing.txt?... (path /outputs/missing.txt)"),
        "{text}"
    );
    assert!(!text.contains("SECRET"), "{text}");
}

#[test]
fn error_logs_are_shown_without_rust_log() {
    // Bind a free port and release it, so that connections to it are refused.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".ga4gh-cli")).unwrap();
    let config = json!({"TES": {"base_path": format!("http://127.0.0.1:{port}")}});
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        config.to_string(),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(["tes", "files", "t1"])
        .env("HOME", home.path())
        .env_remove("RUST_LOG")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains(" ERROR ga4gh_sdk"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn debug_logs_omit_redirected_signed_urls() {
    let prefix = "/save-redirect";
    let storage = format!("{}{prefix}/storage", mockito::server_url());
    let task = json!({"id": "t1", "state": "COMPLETE", "logs": [{"logs": [], "outputs": [
        {"url": format!("{storage}/signed?X-Amz-Signature=SECRET1"), "path": "/o/r.bin", "size_bytes": "2"}
    ]}]});
    let _tes = tes_mocks(prefix, &task);
    let _storage = [
        mock("GET", Matcher::Regex(format!("^{prefix}/storage/signed")))
            .with_status(302)
            .with_header(
                "location",
                &format!("{storage}/target?X-Amz-Signature=SECRET2"),
            )
            .create(),
        mock("GET", Matcher::Regex(format!("^{prefix}/storage/target")))
            .with_status(200)
            .with_body("ok")
            .create(),
    ];
    let home = home_with_tes(prefix);
    let dir = home.path().join("results");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
    command
        .args([
            "tes",
            "save",
            "t1",
            "file:0:0",
            "--dir",
            dir.to_str().unwrap(),
        ])
        .env("HOME", home.path())
        .env("RUST_LOG", "trace");
    for proxy in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        command.env_remove(proxy);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read(dir.join("r.bin")).unwrap(), b"ok");
    for stream in [stdout(&output), stderr(&output)] {
        assert!(!stream.contains("SECRET"), "{stream}");
    }
}

#[test]
fn files_reports_an_incomplete_task() {
    let prefix = "/files-incomplete";
    let _tes = tes_mocks(prefix, &json!({"id": "t1", "state": "RUNNING"}));
    let home = home_with_tes(prefix);
    let output = run(&home, &["tes", "files", "t1"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "SELECTOR       SIZE       DESCRIPTION\n");
    assert_eq!(
        stderr(&output),
        "Note: The task is in state RUNNING; its logs and outputs may be incomplete.\n\
         Note: The task has no logs yet.\n"
    );

    let output = run(&home, &["tes", "log", "t1", "stdout:0:0"]);
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(message.contains("The task has no stdout:0:0"), "{message}");
    assert!(message.contains("state RUNNING"), "{message}");
}

#[test]
fn log_shows_inline_logs_escaped() {
    let prefix = "/log-show";
    let _tes = tes_mocks(prefix, &complete_task(prefix));
    let home = home_with_tes(prefix);
    let output = run(&home, &["tes", "log", "t1", "stdout:0:0"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), "line 1\nline 2 \\u{1b}[31mred\n");
    let output = run(&home, &["tes", "log", "t1", "system:0"]);
    assert_eq!(stdout(&output), "image pulled\n");
    let output = run(&home, &["tes", "log", "t1", "file:0:0"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(stderr(&output).contains("tes save"), "{}", stderr(&output));
}

#[test]
fn save_writes_logs_and_files_byte_for_byte() {
    let prefix = "/save-ok";
    let _tes = tes_mocks(prefix, &complete_task(prefix));
    let _storage = storage_mocks(prefix);
    let home = home_with_tes(prefix);
    let dir = home.path().join("results");
    let dir_arg = dir.to_str().unwrap();
    let output = run(
        &home,
        &[
            "tes",
            "save",
            "t1",
            "stdout:0:0",
            "system:0",
            "file:0:0",
            "file:0:1",
            "--dir",
            dir_arg,
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        fs::read(dir.join("stdout-0-0.txt")).unwrap(),
        b"line 1\nline 2 \x1b[31mred\n"
    );
    assert_eq!(
        fs::read(dir.join("system-0.txt")).unwrap(),
        b"image pulled\n"
    );
    assert_eq!(fs::read(dir.join("out.bin")).unwrap(), BINARY);
    assert_eq!(fs::read(dir.join("result.bin")).unwrap(), b"s3\x00\xffok");
    assert_eq!(
        entries(&dir),
        ["out.bin", "result.bin", "stdout-0-0.txt", "system-0.txt"]
    );
    let text = stdout(&output);
    assert!(
        text.contains(&format!(
            "Saved file:0:0 to {dir_arg}/out.bin ({} bytes)",
            BINARY.len()
        )),
        "{text}"
    );

    // An existing file is kept unless --force is given.
    fs::write(dir.join("out.bin"), "old").unwrap();
    let output = run(&home, &["tes", "save", "t1", "file:0:0", "--dir", dir_arg]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("already exists; use --force"),
        "{}",
        stderr(&output)
    );
    assert_eq!(fs::read(dir.join("out.bin")).unwrap(), b"old");
    let output = run(
        &home,
        &["tes", "save", "t1", "file:0:0", "--dir", dir_arg, "--force"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read(dir.join("out.bin")).unwrap(), BINARY);

    let output = run(
        &home,
        &[
            "tes", "save", "t1", "output:0", "--dir", dir_arg, "--name", "copy.bin",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read(dir.join("copy.bin")).unwrap(), BINARY);
}

#[test]
fn failed_downloads_leave_no_file() {
    let prefix = "/save-errors";
    let _tes = tes_mocks(prefix, &complete_task(prefix));
    let _storage = storage_mocks(prefix);
    let home = home_with_tes(prefix);
    let dir = home.path().join("results");
    let dir_arg = dir.to_str().unwrap();
    let cases = [
        ("file:0:2", "Received 5 bytes, but the task reports 100"),
        ("file:0:3", "returned HTTP 403"),
        (
            "file:0:4",
            "Unsupported URL scheme 'gs'; supported: http, https and s3",
        ),
        ("file:0:5", "Malformed URL"),
        ("file:0:6", "/absent.bin was not found"),
        ("file:0:7", "not a plain file name; choose one with --name"),
        ("output:1", "directory or a pattern"),
    ];
    for (selector, expected) in cases {
        let output = run(&home, &["tes", "save", "t1", selector, "--dir", dir_arg]);
        let message = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{selector}: {message}");
        assert!(message.contains(expected), "{selector}: {message}");
        assert!(
            !message.contains("SECRET") && !message.contains("panicked"),
            "{message}"
        );
        assert!(entries(&dir).is_empty(), "{selector}: {:?}", entries(&dir));
    }

    // Selections are checked before any download: nothing is saved.
    let output = run(
        &home,
        &[
            "tes", "save", "t1", "file:0:0", "file:0:9", "--dir", dir_arg,
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("The task has no file:0:9"),
        "{}",
        stderr(&output)
    );
    assert!(entries(&dir).is_empty());

    // Saving stops at the first failure and reports what was saved before.
    let output = run(
        &home,
        &[
            "tes", "save", "t1", "file:0:0", "file:0:3", "file:0:1", "--dir", dir_arg,
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("Saved file:0:0"),
        "{}",
        stdout(&output)
    );
    assert_eq!(entries(&dir), ["out.bin"]);
}

#[test]
fn an_interrupted_download_leaves_no_file() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\npartial data")
            .unwrap();
    });
    let prefix = "/save-interrupted";
    let task = json!({"id": "t1", "state": "COMPLETE", "logs": [{
        "logs": [], "outputs": [{"url": format!("http://{address}/big.bin"), "path": "/o/big.bin", "size_bytes": "1000"}]
    }]});
    let _tes = tes_mocks(prefix, &task);
    let home = home_with_tes(prefix);
    let dir = home.path().join("results");
    let output = run(
        &home,
        &[
            "tes",
            "save",
            "t1",
            "file:0:0",
            "--dir",
            dir.to_str().unwrap(),
        ],
    );
    server.join().unwrap();
    let message = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{message}");
    assert!(message.contains("interrupted after 12 bytes"), "{message}");
    assert!(entries(&dir).is_empty(), "{:?}", entries(&dir));
}

#[test]
fn invalid_input_fails_before_the_service_is_contacted() {
    // No mocks below this prefix: a request would fail differently.
    let home = home_with_tes("/no-service");
    let dir = home.path().join("results");
    let dir_arg = dir.to_str().unwrap();
    let cases: [(&[&str], &str); 4] = [
        (
            &["tes", "log", "t1", "stdout:0"],
            "Invalid selector 'stdout:0'",
        ),
        (
            &["tes", "save", "t1", "file:x:0", "--dir", dir_arg],
            "Invalid selector",
        ),
        (
            &[
                "tes",
                "save",
                "t1",
                "file:0:0",
                "--dir",
                dir_arg,
                "--name",
                "/etc/passwd",
            ],
            "absolute path",
        ),
        (
            &[
                "tes", "save", "t1", "file:0:0", "--dir", dir_arg, "--name", "../x",
            ],
            "contains '..'",
        ),
    ];
    for (args, expected) in cases {
        let output = run(&home, args);
        let message = stderr(&output);
        assert_eq!(output.status.code(), Some(1), "{args:?}: {message}");
        assert!(message.contains(expected), "{args:?}: {message}");
    }
    let output = run(&home, &["tes", "save", "t1", "file:0:0"]);
    assert_eq!(output.status.code(), Some(2), "--dir is required");
    assert!(!dir.exists());
}

#[test]
fn a_stalled_download_times_out_and_leaves_no_file() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nslow")
            .unwrap();
        // Keep the connection open without sending more data.
        std::thread::sleep(Duration::from_secs(5));
    });
    let prefix = "/save-stalled";
    let task = json!({"id": "t1", "state": "COMPLETE", "logs": [{
        "logs": [], "outputs": [{"url": format!("http://{address}/slow.bin"), "path": "/o/slow.bin", "size_bytes": "1000"}]
    }]});
    let _tes = tes_mocks(prefix, &task);
    let home = home_with_tes(prefix);
    let config = json!({"TES": {
        "base_path": format!("{}{prefix}", mockito::server_url()),
        "timeouts": {"request": 1},
    }});
    fs::write(
        home.path().join(".ga4gh-cli/config.json"),
        config.to_string(),
    )
    .unwrap();
    let dir = home.path().join("results");
    let started = Instant::now();
    let output = run(
        &home,
        &[
            "tes",
            "save",
            "t1",
            "file:0:0",
            "--dir",
            dir.to_str().unwrap(),
        ],
    );
    let message = stderr(&output);
    assert_eq!(output.status.code(), Some(1), "{message}");
    assert!(
        message.contains("stalled: no data for 1 seconds after 4 bytes"),
        "{message}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(entries(&dir).is_empty(), "{:?}", entries(&dir));
    server.join().unwrap();
}

#[test]
fn force_replaces_a_symlink_without_writing_through_it() {
    let prefix = "/save-symlink";
    let _tes = tes_mocks(prefix, &complete_task(prefix));
    let home = home_with_tes(prefix);
    let dir = home.path().join("results");
    fs::create_dir(&dir).unwrap();
    let outside = home.path().join("outside.txt");
    fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("stdout-0-0.txt")).unwrap();
    let dir_arg = dir.to_str().unwrap();
    let output = run(
        &home,
        &["tes", "save", "t1", "stdout:0:0", "--dir", dir_arg],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("already exists"),
        "{}",
        stderr(&output)
    );
    let output = run(
        &home,
        &[
            "tes",
            "save",
            "t1",
            "stdout:0:0",
            "--dir",
            dir_arg,
            "--force",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    assert!(!dir
        .join("stdout-0-0.txt")
        .symlink_metadata()
        .unwrap()
        .is_symlink());
}
