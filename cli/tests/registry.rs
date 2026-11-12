//! Checks the `registry` commands of the CLI binary against a mock registry.

// The CLI finds its config file through `HOME`, which Windows doesn't use.
#![cfg(unix)]

use ga4gh_cli::{command, dispatch_with_config};
use ga4gh_sdk::utils::configuration::Configuration;
use mockito::mock;
use serde_json::{json, Value};
use std::fs;
use std::net::TcpListener;
use std::process::{Command, Output};
use tempfile::TempDir;
use url::Url;

const MARKER: &str = "SENTINEL";

fn service(id: &str, artifact: &str) -> Value {
    json!({
        "id": id,
        "name": format!("Service {id}"),
        "type": {"group": "org.ga4gh", "artifact": artifact, "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0",
        "url": format!("https://example.org/ga4gh/{artifact}/v1"),
    })
}

/// Runs the CLI with a config file whose `REGISTRY` section is `section`.
fn run_with_section(section: Value, args: &[&str]) -> Output {
    let home = TempDir::new().unwrap();
    let config_dir = home.path().join(".ga4gh-cli");
    fs::create_dir(&config_dir).unwrap();
    fs::write(config_dir.join("config.json"), section.to_string()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
    command
        .args(args)
        .env("HOME", home.path())
        .env("RUST_LOG", "debug");
    for proxy in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        command.env_remove(proxy);
    }
    let output = command.output().unwrap();
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        assert!(
            !text.contains(MARKER),
            "marker in the output of {args:?}:\n{text}"
        );
    }
    output
}

/// Runs the CLI against `base_path`, with a marker value in the credential.
fn run(base_path: &str, args: &[&str]) -> Output {
    run_with_section(
        json!({"REGISTRY": {
            "base_path": base_path,
            "oauth_access_token": "SENTINEL_OAUTH",
        }}),
        args,
    )
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn list_prints_services_and_filters_by_type() {
    let mut extra = service("org.example.wes", "wes");
    extra["vendor_field"] = json!({"kept": false});
    let _m = mock("GET", "/list/ga4gh/registry/v1/services")
        .with_status(200)
        .with_body(json!([service("org.example.tes", "tes"), extra]).to_string())
        .create();
    for suffix in ["", "/"] {
        let base_path = format!("{}/list/ga4gh/registry/v1{suffix}", mockito::server_url());

        let output = run(&base_path, &["registry", "list"]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(
            stdout(&output),
            "ID               TYPE                 URL\n\
             org.example.tes  org.ga4gh:tes:1.1.0  https://example.org/ga4gh/tes/v1\n\
             org.example.wes  org.ga4gh:wes:1.1.0  https://example.org/ga4gh/wes/v1\n"
        );

        for artifact in ["tes", "TES"] {
            let output = run(&base_path, &["registry", "list", "--type", artifact]);
            assert!(output.status.success(), "{}", stderr(&output));
            let text = stdout(&output);
            assert!(text.contains("org.example.tes"), "{text}");
            assert!(!text.contains("org.example.wes"), "{text}");
        }

        let output = run(&base_path, &["registry", "list", "-t", "drs"]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(stdout(&output), "ID  TYPE  URL\n");
    }
}

#[test]
fn get_prints_one_service_with_the_fields_that_the_registry_adds() {
    let mut body = service("org.example/tes 1", "tes");
    body["vendor_field"] = json!(true);
    let _m = mock("GET", "/get/services/org.example%2Ftes%201")
        .with_status(200)
        .with_body(body.to_string())
        .create();
    let base_path = format!("{}/get", mockito::server_url());
    let output = run(&base_path, &["registry", "get", "org.example/tes 1"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let printed: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(printed, body);
    _m.assert();
}

#[test]
fn types_prints_one_type_per_line() {
    let _m = mock("GET", "/types/services/types")
        .with_status(200)
        .with_body(
            json!([
                {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
                {"group": "org.ga4gh", "artifact": "wes", "version": "1.1.0"},
            ])
            .to_string(),
        )
        .create();
    let output = run(
        &format!("{}/types", mockito::server_url()),
        &["registry", "types"],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "org.ga4gh:tes:1.1.0\norg.ga4gh:wes:1.1.0\n"
    );
}

#[test]
fn error_responses_exit_with_status_and_body() {
    for status in [401, 404, 500] {
        let prefix = format!("/error{status}");
        let body = format!("mock failure {status}");
        let _m = mock(
            "GET",
            mockito::Matcher::Regex(format!("^{prefix}/services")),
        )
        .with_status(status)
        .with_body(&body)
        .create();
        let base_path = format!("{}{prefix}", mockito::server_url());
        for args in [
            &["registry", "list"][..],
            &["registry", "get", "x"],
            &["registry", "types"],
        ] {
            let output = run(&base_path, args);
            assert_eq!(output.status.code(), Some(1), "{args:?}");
            assert!(output.stdout.is_empty(), "{args:?}");
            let message = stderr(&output);
            assert!(message.contains(&status.to_string()), "{message}");
            assert!(message.contains(&body), "{message}");
            let hint = message.contains("base_path is the Service Registry API root");
            // An unknown service ID is a 404 too, but not a wrong base_path.
            assert_eq!(
                hint,
                status == 404 && args[1] != "get",
                "{args:?}: {message}"
            );
        }
    }
}

#[test]
fn invalid_responses_and_unreachable_registries_fail() {
    let _m = mock("GET", "/invalid/services")
        .with_status(200)
        .with_body("<html>not a registry</html>")
        .create();
    let output = run(
        &format!("{}/invalid", mockito::server_url()),
        &["registry", "list"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("Invalid response from the service registry"),
        "{}",
        stderr(&output)
    );

    // Bind a free port and release it, so that connections to it are refused.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let output = run(&format!("http://127.0.0.1:{port}"), &["registry", "types"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn server_text_is_escaped_in_get_and_errors() {
    let mut body = service("org.example.tes", "tes");
    body["name"] = json!("a\u{9b}2K\u{1b}b");
    body["description"] = json!("right\u{202e}left\u{2028}next");
    let _get = mock("GET", "/escape/services/ok")
        .with_status(200)
        .with_body(body.to_string())
        .create();
    let _error = mock("GET", "/escape/services")
        .with_status(500)
        .with_body("\u{1b}[2K\rOK: 3 services\nfake line")
        .create();
    let base_path = format!("{}/escape", mockito::server_url());

    let output = run(&base_path, &["registry", "get", "ok"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        !text.contains('\u{9b}')
            && !text.contains('\u{1b}')
            && !text.contains('\u{202e}')
            && !text.contains('\u{2028}'),
        "{text}"
    );
    assert!(text.contains(r"a\u009b2K\u001bb"), "{text}");
    assert!(text.contains(r"right\u202eleft\u2028next"), "{text}");
    let parsed: Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(parsed, body);

    let output = run(&base_path, &["registry", "list"]);
    assert_eq!(output.status.code(), Some(1));
    let message = stderr(&output);
    assert!(
        !message.contains('\u{1b}') && !message.contains('\r'),
        "{message}"
    );
    assert!(
        message.contains(r"\u{1b}[2K\rOK: 3 services\nfake line"),
        "{message}"
    );
}

#[test]
fn empty_service_id_is_a_usage_error() {
    let output = run("http://127.0.0.1:9", &["registry", "get", ""]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(!output.stderr.is_empty());
}

#[test]
fn missing_registry_section_is_an_error() {
    let output = run_with_section(
        json!({"TES": {"base_path": "http://localhost/ga4gh/tes/v1"}}),
        &["registry", "list"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr(&output).contains("'REGISTRY' object"),
        "{}",
        stderr(&output)
    );
}

#[tokio::test]
async fn downstream_can_inject_a_registry_config() {
    let _m = mock("GET", "/downstream/services")
        .with_status(200)
        .with_body(json!([service("org.example.tes", "tes")]).to_string())
        .create();
    let config =
        Configuration::new(Url::parse(&format!("{}/downstream", mockito::server_url())).unwrap());
    let matches = command()
        .try_get_matches_from(["ga4gh-cli", "registry", "list"])
        .unwrap();
    let output = dispatch_with_config(&matches, &config)
        .await
        .unwrap()
        .expect("registry is a built-in command");
    assert!(output.stdout.contains("org.example.tes"));
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_code, std::process::ExitCode::SUCCESS);
}
