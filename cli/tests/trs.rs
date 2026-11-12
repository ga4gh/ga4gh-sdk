#![cfg(unix)]

use mockito::{mock, Matcher, Mock};
use serde_json::{json, Value};
use std::fs;
use std::process::{Command, Output};
use tempfile::TempDir;

const ROOT: &str = "/ga4gh/trs/v2";

const TOOL_ID: &str = "abc-123";

fn home_with_trs_config() -> TempDir {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".ga4gh-cli");
    fs::create_dir(&directory).unwrap();
    let base_path = format!("{}{ROOT}", mockito::server_url());
    fs::write(
        directory.join("config.json"),
        json!({"TRS": {"base_path": base_path}}).to_string(),
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

fn service_info(expected_calls: usize) -> Mock {
    mock("GET", format!("{ROOT}/service-info").as_str())
        .with_status(200)
        .with_body(
            json!({
                "id": "org.example.trs",
                "name": "Example TRS",
                "type": {"group": "org.ga4gh", "artifact": "trs", "version": "2.0.2"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0",
            })
            .to_string(),
        )
        .expect(expected_calls)
        .create()
}

fn get(path: &str, body: &str) -> Mock {
    mock("GET", format!("{ROOT}/{path}").as_str())
        .with_status(200)
        .with_body(body)
        .create()
}

fn stdout(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_str(&stdout(output)).unwrap()
}

fn tool() -> Value {
    json!({
        "url": "https://example.org/tools/1",
        "id": TOOL_ID,
        "organization": "org",
        "name": "repo",
        "toolclass": {"id": "1", "name": "Workflow"},
        "versions": [{"url": "https://example.org/tools/1/versions/v1", "id": "v1"}],
    })
}

#[test]
fn tools_sends_filters_and_prints_json() {
    let service = service_info(1);
    let tools = mock("GET", format!("{ROOT}/tools").as_str())
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("toolClass".into(), "Workflow".into()),
            Matcher::UrlEncoded("descriptorType".into(), "CWL".into()),
            Matcher::UrlEncoded("organization".into(), "org".into()),
            Matcher::UrlEncoded("name".into(), "repo".into()),
            Matcher::UrlEncoded("offset".into(), "page-2".into()),
            Matcher::UrlEncoded("limit".into(), "5".into()),
            Matcher::UrlEncoded("tags".into(), "a b,c".into()),
        ]))
        .with_status(200)
        .with_body(json!([tool()]).to_string())
        .create();
    let home = home_with_trs_config();

    let output = run(
        &home,
        &[
            "trs",
            "tools",
            "--tool-class",
            "Workflow",
            "--descriptor-type",
            "cwl",
            "--organization",
            "org",
            "--name",
            "repo",
            "--offset",
            "page-2",
            "--limit",
            "5",
            "--tag",
            "a b",
            "--tag",
            "c",
        ],
    );
    let listed = stdout_json(&output);
    assert_eq!(listed[0]["id"], TOOL_ID);
    assert_eq!(listed[0]["toolclass"]["name"], "Workflow");
    service.assert();
    tools.assert();
}

#[test]
fn tool_versions_and_version_encode_ids() {
    let service = service_info(3);
    let tool_mock = get(&format!("tools/{TOOL_ID}"), &tool().to_string());
    let versions = get(
        &format!("tools/{TOOL_ID}/versions"),
        &json!([{"url": "https://example.org/v1", "id": "v1", "name": "first"}]).to_string(),
    );
    let version = get(
        &format!("tools/{TOOL_ID}/versions/v%201%2Fbeta"),
        &json!({"url": "https://example.org/v1", "id": "v 1/beta", "name": "beta", "descriptor_type": ["CWL"], "description": "Beta release"})
            .to_string(),
    );
    let home = home_with_trs_config();

    assert_eq!(
        stdout_json(&run(&home, &["trs", "tool", TOOL_ID]))["versions"][0]["id"],
        "v1"
    );
    let one = stdout_json(&run(&home, &["trs", "version", TOOL_ID, "v 1/beta"]));
    assert_eq!(one["name"], "beta");
    assert_eq!(one["descriptor_type"][0], "CWL");
    assert_eq!(one["description"], "Beta release");
    assert_eq!(
        stdout_json(&run(&home, &["trs", "versions", TOOL_ID]))[0]["name"],
        "first"
    );
    service.assert();
    tool_mock.assert();
    versions.assert();
    version.assert();
}

#[test]
fn descriptor_prints_json_plain_and_relative_files() {
    let service = service_info(3);
    let version = format!("tools/{TOOL_ID}/versions/v%201");
    let descriptor = "cwlVersion: v1.2\nclass: Workflow\n";
    let wrapped = get(
        &format!("{version}/CWL/descriptor"),
        &json!({"content": descriptor, "url": "https://example.org/main.cwl"}).to_string(),
    );
    let plain = get(&format!("{version}/PLAIN_CWL/descriptor"), descriptor);
    let relative = get(
        &format!("{version}/PLAIN_CWL/descriptor/tools%2Falign.cwl"),
        "class: CommandLineTool\n",
    );
    let home = home_with_trs_config();

    let json = stdout_json(&run(
        &home,
        &["trs", "descriptor", TOOL_ID, "v 1", "--type", "CWL"],
    ));
    assert_eq!(json["content"], descriptor);
    assert_eq!(json["url"], "https://example.org/main.cwl");
    assert_eq!(
        stdout(&run(
            &home,
            &[
                "trs",
                "descriptor",
                TOOL_ID,
                "v 1",
                "--type",
                "cwl",
                "--plain"
            ],
        )),
        descriptor
    );
    assert_eq!(
        stdout(&run(
            &home,
            &[
                "trs",
                "descriptor",
                TOOL_ID,
                "v 1",
                "--type",
                "CWL",
                "--plain",
                "--path",
                "tools/align.cwl",
            ],
        )),
        "class: CommandLineTool\n"
    );
    service.assert();
    wrapped.assert();
    plain.assert();
    relative.assert();
}

#[test]
fn files_and_tests_print_json_or_plain_content() {
    let service = service_info(3);
    let version = format!("tools/{TOOL_ID}/versions/v1");
    let files = get(
        &format!("{version}/CWL/files"),
        &json!([
            {"path": "main.cwl", "file_type": "PRIMARY_DESCRIPTOR"},
            {"path": "tools/align.cwl", "file_type": "SECONDARY_DESCRIPTOR"},
        ])
        .to_string(),
    );
    let tests = get(
        &format!("{version}/CWL/tests"),
        &json!([{"content": "reads: a.fq", "url": "https://example.org/test.yml"}]).to_string(),
    );
    let plain_tests = get(&format!("{version}/PLAIN_CWL/tests"), "reads: a.fq\n");
    let home = home_with_trs_config();

    let listed = stdout_json(&run(
        &home,
        &["trs", "files", TOOL_ID, "v1", "--type", "CWL"],
    ));
    assert_eq!(listed[1]["path"], "tools/align.cwl");
    assert_eq!(listed[1]["file_type"], "SECONDARY_DESCRIPTOR");
    assert_eq!(
        stdout_json(&run(
            &home,
            &["trs", "tests", TOOL_ID, "v1", "--type", "CWL"]
        ))[0]["content"],
        "reads: a.fq"
    );
    assert_eq!(
        stdout(&run(
            &home,
            &["trs", "tests", TOOL_ID, "v1", "--type", "CWL", "--plain"],
        )),
        "reads: a.fq\n"
    );
    service.assert();
    files.assert();
    tests.assert();
    plain_tests.assert();
}

#[test]
fn files_zip_writes_the_archive_without_overwriting_files() {
    let service = service_info(1);
    let bytes = include_bytes!("../../tests/fixtures/trs/workflow.zip");
    let zip = mock(
        "GET",
        format!("{ROOT}/tools/{TOOL_ID}/versions/v1/CWL/files").as_str(),
    )
    .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
    .with_status(200)
    .with_header("content-type", "application/zip")
    .with_body(bytes.as_slice())
    .expect(1)
    .create();
    let home = home_with_trs_config();
    let archive = home.path().join("workflow.zip");
    let archive_arg = archive.to_str().unwrap();
    let args = [
        "trs",
        "files",
        TOOL_ID,
        "v1",
        "--type",
        "CWL",
        "--zip",
        archive_arg,
    ];

    let written = stdout(&run(&home, &args));
    assert!(
        written.starts_with(&format!("Wrote {} bytes to ", bytes.len())),
        "{written}"
    );
    assert_eq!(fs::read(&archive).unwrap(), bytes);

    // An existing file is kept, and the registry isn't contacted again.
    fs::write(&archive, "keep").unwrap();
    let output = run(&home, &args);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
    assert_eq!(fs::read_to_string(&archive).unwrap(), "keep");
    service.assert();
    zip.assert();
}

#[test]
fn files_zip_writes_no_file_if_the_download_fails() {
    let service = service_info(2);
    let version = format!("{ROOT}/tools/{TOOL_ID}/versions/v2");
    let not_zip = mock("GET", format!("{version}/CWL/files").as_str())
        .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
        .with_status(200)
        .with_body(json!([{"path": "main.cwl"}]).to_string())
        .expect(1)
        .create();
    let missing = mock("GET", format!("{version}/WDL/files").as_str())
        .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
        .with_status(404)
        .expect(1)
        .create();
    let home = home_with_trs_config();
    let archive = home.path().join("workflow.zip");

    for (descriptor_type, expected) in [("CWL", "not a ZIP archive"), ("WDL", "404")] {
        let output = run(
            &home,
            &[
                "trs",
                "files",
                TOOL_ID,
                "v2",
                "--type",
                descriptor_type,
                "--zip",
                archive.to_str().unwrap(),
            ],
        );
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "{stderr}");
        assert!(!archive.exists());
        // No temporary file is left either; only the config directory.
        assert_eq!(fs::read_dir(home.path()).unwrap().count(), 1);
    }
    service.assert();
    not_zip.assert();
    missing.assert();
}

#[test]
fn plain_tests_reject_url_only_wrappers_without_printing_a_false_file() {
    let service = service_info(1);
    let test_files = get(
        &format!("tools/{TOOL_ID}/versions/v1/PLAIN_CWL/tests"),
        &json!([{"url": "https://example.org/test.yml"}]).to_string(),
    );
    let home = home_with_trs_config();

    let output = run(
        &home,
        &["trs", "tests", TOOL_ID, "v1", "--type", "CWL", "--plain"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("no inline content"));
    service.assert();
    test_files.assert();
}

#[test]
fn json_output_and_server_errors_escape_terminal_controls() {
    let service = service_info(2);
    let mut body = tool();
    body["name"] = json!("workflow\u{202e}name");
    let tool_mock = get(&format!("tools/{TOOL_ID}"), &body.to_string());
    let failed = mock("GET", format!("{ROOT}/tools/missing").as_str())
        .with_status(400)
        .with_body("bad\u{1b}[31mresponse")
        .create();
    let home = home_with_trs_config();

    let output = run(&home, &["trs", "tool", TOOL_ID]);
    assert!(!stdout(&output).contains('\u{202e}'));
    assert_eq!(stdout_json(&output)["name"], "workflow\u{202e}name");
    let output = run(&home, &["trs", "tool", "missing"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains('\u{1b}'));
    assert!(stderr.contains("\\u{1b}"), "{stderr}");
    service.assert();
    tool_mock.assert();
    failed.assert();
}

#[test]
fn invalid_arguments_fail_before_contacting_the_registry() {
    let service = service_info(0);
    let home = home_with_trs_config();

    for (args, expected) in [
        (
            vec!["trs", "descriptor", "t", "v", "--type", "PYTHON"],
            "Invalid descriptor type 'PYTHON'",
        ),
        (
            vec!["trs", "tools", "--descriptor-type", "nope"],
            "CWL, WDL, NFL, GALAXY, or SMK",
        ),
        (
            vec!["trs", "tools", "--limit", "ten"],
            "Invalid limit 'ten'",
        ),
        (
            vec!["trs", "tools", "--tag", "a,b"],
            "TRS tags must not be empty or contain a comma",
        ),
        (
            vec!["trs", "tools", "--tag", ""],
            "TRS tags must not be empty or contain a comma",
        ),
        (vec!["trs", "tool", ".."], "TOOL_ID must not be empty"),
        (
            vec!["trs", "version", "t", "."],
            "VERSION_ID must not be empty",
        ),
        (
            vec![
                "trs",
                "descriptor",
                "t",
                "v",
                "--type",
                "CWL",
                "--path",
                "..",
            ],
            "path must not be empty",
        ),
    ] {
        let output = run(&home, &args);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{args:?}: {stderr}");
        assert!(stderr.contains(expected), "{args:?}: {stderr}");
        assert!(!stderr.contains("panicked"), "{stderr}");
    }

    // A missing --type is a usage error.
    let usage = run(&home, &["trs", "files", "t", "v"]);
    assert_eq!(usage.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&usage.stderr).contains("--type"));
    service.assert();
}

#[test]
fn service_failures_have_nonzero_exit_codes() {
    let service = service_info(1);
    let missing = mock("GET", format!("{ROOT}/tools/missing").as_str())
        .with_status(404)
        .with_body("tool not found")
        .create();
    let home = home_with_trs_config();

    let output = run(&home, &["trs", "tool", "missing"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("tool not found"));
    service.assert();
    missing.assert();
}

#[test]
fn wrong_api_root_suggests_checking_base_path() {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".ga4gh-cli");
    fs::create_dir(&directory).unwrap();
    // The host without the TRS prefix: service-info is not found there.
    fs::write(
        directory.join("config.json"),
        json!({"TRS": {"base_path": format!("{}/wrong", mockito::server_url())}}).to_string(),
    )
    .unwrap();
    let not_found = mock("GET", "/wrong/service-info").with_status(404).create();

    let output = run(&home, &["trs", "tool", "t"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("TRS API root"));
    not_found.assert();
}

#[test]
fn missing_trs_config_names_the_section() {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".ga4gh-cli");
    fs::create_dir(&directory).unwrap();
    fs::write(
        directory.join("config.json"),
        json!({"TES": {"base_path": "https://example.org"}}).to_string(),
    )
    .unwrap();

    let output = run(&home, &["trs", "tool", "t"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("'TRS' object"));
}
