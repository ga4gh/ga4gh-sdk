//! Checks the `service-info` command against a mock service.
//!
//! Each test uses its own API root on the shared mock server, so that tests
//! running in parallel don't see each other's mocks. The binary tests set
//! `HOME` only in their child processes.

// The CLI finds its config file through `HOME`, which Windows doesn't use.
#![cfg(unix)]

use ga4gh_cli::{command, dispatch_with_config, CliContext};
use ga4gh_sdk::utils::configuration::Configuration;
use mockito::mock;
use serde_json::{json, Value};
use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::process::{Command, ExitCode, Output};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use url::Url;

/// A credential value that must never appear in any output.
const MARKER: &str = "SENTINEL";

/// Service info with extension fields at the top level and in nested metadata.
fn service_info(artifact: &str) -> Value {
    json!({
        "id": format!("org.example.{artifact}"),
        "name": format!("Example {artifact}"),
        "type": {
            "group": "org.ga4gh",
            "artifact": artifact,
            "version": "1.1.0",
            "x-type-extension": "kept"
        },
        "organization": {
            "name": "Example",
            "url": "https://example.org",
            "x-organization-extension": {"nested": [1, 2]}
        },
        "version": "1.0.0",
        "storage": ["s3://bucket"],
        "x-vendor": {"deep": {"deeper": true}}
    })
}

/// Runs the CLI binary with `config` as `~/.ga4gh-cli/config.json`.
fn run(config: &Value, args: &[&str]) -> Output {
    let home = TempDir::new().unwrap();
    let config_dir = home.path().join(".ga4gh-cli");
    fs::create_dir(&config_dir).unwrap();
    fs::write(config_dir.join("config.json"), config.to_string()).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"));
    command
        .args(args)
        .env("HOME", home.path())
        .env_remove("GA4GH_CLI_PROFILE")
        .env_remove("RUST_LOG");
    for proxy in ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] {
        command.env_remove(proxy);
    }
    let output = command.output().unwrap();
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        assert!(
            !text.contains(MARKER),
            "credential in the output of {args:?}:\n{text}"
        );
    }
    output
}

fn root(prefix: &str) -> String {
    format!("{}{prefix}", mockito::server_url())
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Asserts a failure with exit code 1, nothing on stdout, and `expected` on
/// stderr.
fn assert_failure(output: &Output, expected: &str) {
    let message = stderr(output);
    assert_eq!(output.status.code(), Some(1), "{message}");
    assert!(output.stdout.is_empty(), "{}", stdout(output));
    assert!(message.contains(expected), "{message}");
    // One diagnostic, without a log line that repeats it.
    assert_eq!(message.lines().count(), 1, "{message}");
}

#[test]
fn prints_the_service_info_of_a_built_in_section_with_its_credential() {
    let body = service_info("tes");
    let info = mock("GET", "/si-built-in/ga4gh/tes/v1/service-info")
        .match_header("authorization", "Bearer SENTINEL_TOKEN")
        .with_body(body.to_string())
        .expect(2)
        .create();
    for suffix in ["", "/"] {
        let config = json!({"TES": {
            "base_path": format!("{}{suffix}", root("/si-built-in/ga4gh/tes/v1")),
            "oauth_access_token": "SENTINEL_TOKEN",
        }});
        let output = run(&config, &["service-info", "--service", "TES"]);
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
        // The body as received, with its layout.
        assert_eq!(stdout(&output), body.to_string());
    }
    info.assert();
}

#[test]
fn values_keep_their_spelling_and_order() {
    let body = r#"{"zz":1,"id":"org.example.tes","name":"Example","type":{"group":"org.ga4gh","artifact":"tes","version":"1.1.0"},"organization":{"name":"Example","url":"https://example.org"},"version":"1.0.0","description":null,"big":123456789012345678901234567890,"f":1.00}"#;
    let _info = mock("GET", "/si-spelling/service-info")
        .with_body(body)
        .create();
    let config = json!({"TES": {"base_path": root("/si-spelling")}});
    let output = run(&config, &["service-info", "--service", "TES"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(stdout(&output), body);
}

#[test]
fn help_describes_the_command() {
    let output = run(&json!({}), &["service-info", "--help"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for expected in ["--service <SECTION>", "base_path", "discovery timeout"] {
        assert!(text.contains(expected), "{text}");
    }
}

#[test]
fn reads_a_custom_section_of_the_selected_profile() {
    let info = mock("GET", "/si-profile/staging/service-info")
        .match_header("authorization", "Basic Y2xpOlNFTlRJTkVMX1BBU1M=")
        .with_body(service_info("my-service").to_string())
        .create();
    let config = json!({
        "default_profile": "local",
        "profiles": {
            "local": {"MY_SERVICE": {"base_path": root("/si-profile/local")}},
            "staging": {"MY_SERVICE": {
                "base_path": root("/si-profile/staging"),
                "basic_auth": {"username": "cli", "password": "SENTINEL_PASS"},
            }}
        }
    });
    let output = run(
        &config,
        &[
            "--profile",
            "staging",
            "service-info",
            "--service",
            "MY_SERVICE",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let printed: Value = serde_json::from_str(&stdout(&output)).unwrap();
    assert_eq!(printed["type"]["artifact"], "my-service");
    info.assert();
}

#[test]
fn server_text_is_escaped_and_still_round_trips() {
    let mut body = service_info("tes");
    body["name"] = json!("a\u{1b}[2K\rb\u{9b}");
    body["organization"]["name"] = json!("right\u{202e}left\u{2028}next");
    let _info = mock("GET", "/si-escape/service-info")
        .with_body(body.to_string())
        .create();
    let config = json!({"TES": {"base_path": root("/si-escape")}});
    let output = run(&config, &["service-info", "--service", "TES"]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for raw in ['\u{1b}', '\r', '\u{9b}', '\u{202e}', '\u{2028}'] {
        assert!(!text.contains(raw), "{text}");
    }
    assert!(text.contains(r"a\u001b[2K\rb\u009b"), "{text}");
    assert!(text.contains(r"right\u202eleft\u2028next"), "{text}");
    let printed: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(printed, body);
}

#[test]
fn a_missing_section_or_service_option_fails_before_a_request() {
    let config = json!({"TES": {"base_path": "http://127.0.0.1:9/"}});
    let output = run(&config, &["service-info", "--service", "WES"]);
    assert_failure(&output, "'WES'");

    // A user-supplied section name can reach a configuration error before
    // service_info::run has a chance to format it.
    let output = run(
        &config,
        &["service-info", "--service", "BAD\u{1b}[2K\rforged"],
    );
    assert_failure(&output, "BAD");
    assert!(!stderr(&output).contains(['\u{1b}', '\r']));

    let output = run(
        &config,
        &["--profile", "nope", "service-info", "--service", "TES"],
    );
    assert_failure(&output, "nope");

    // No configuration file at all.
    let home = TempDir::new().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(["service-info", "--service", "TES"])
        .env("HOME", home.path())
        .env_remove("GA4GH_CLI_PROFILE")
        .output()
        .unwrap();
    assert_failure(&output, "Cannot read configuration file");

    // A token variable that the section names but that isn't set; the port
    // is closed, so a request would give another error.
    let config = json!({"TES": {
        "base_path": "http://127.0.0.1:9/",
        "oauth_access_token_env": "GA4GH_CLI_TEST_SERVICE_INFO_UNSET",
    }});
    let output = run(&config, &["service-info", "--service", "TES"]);
    assert_failure(&output, "'GA4GH_CLI_TEST_SERVICE_INFO_UNSET' is not set");

    let output = run(&config, &["service-info"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("--service"), "{}", stderr(&output));

    let output = run(&config, &["service-info", "--service", ""]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(output.stdout.is_empty());
}

#[test]
fn http_errors_report_the_status_without_the_body() {
    for status in [401, 404, 500] {
        let prefix = format!("/si-error-{status}");
        let _info = mock("GET", format!("{prefix}/service-info").as_str())
            .with_status(status)
            .with_body("\u{1b}[2K\rforged line")
            .create();
        let config = json!({"DRS": {
            "base_path": root(&prefix),
            "bearer_access_token": "SENTINEL_BEARER",
        }});
        let output = run(&config, &["service-info", "--service", "DRS"]);
        assert_failure(&output, &status.to_string());
        let message = stderr(&output);
        assert!(!message.contains("forged line"), "{message}");
        assert!(!message.contains('\u{1b}'), "{message}");
        assert_eq!(
            message.contains("Check that base_path is the API root of section 'DRS'"),
            status == 404,
            "{message}"
        );
    }
}

#[test]
fn malformed_json_or_a_non_object_is_an_error() {
    for (name, body) in [
        ("not-json", "<html>not service info</html>".to_owned()),
        ("array", "[]".to_owned()),
        ("null", "null".to_owned()),
        ("trailing", "{} trailing".to_owned()),
    ] {
        let prefix = format!("/si-malformed-{name}");
        let _info = mock("GET", format!("{prefix}/service-info").as_str())
            .with_body(body)
            .create();
        let config = json!({"TRS": {"base_path": root(&prefix)}});
        let output = run(&config, &["service-info", "--service", "TRS"]);
        assert_failure(&output, "Invalid service info from section 'TRS'");
    }
}

#[test]
fn the_raw_command_keeps_numbers_that_the_typed_model_cannot_hold() {
    // The command reads a JSON object; `ServiceInfo::get` uses serde_json::Value
    // for extensions, which cannot represent this number.
    let body = r#"{"id":"i","name":"n","type":{"group":"g","artifact":"a","version":"1"},"organization":{"name":"o","url":"u"},"version":"1","x":1e400}"#;
    let _info = mock("GET", "/si-out-of-range/service-info")
        .with_body(body)
        .create();
    let config = json!({"TES": {"base_path": root("/si-out-of-range")}});
    let output = run(&config, &["service-info", "--service", "TES"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty());
    assert_eq!(stdout(&output), body);
}

#[test]
fn prints_native_wes_service_info_without_generic_discovery_fields() {
    // WES::new accepts this WES 1.0 shape, even though ServiceInfo::get cannot
    // decode it into the generic Service model.
    let body = json!({
        "version": "9.3.0",
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {"toil": "9.3.0"},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let response = mock("GET", "/si-native-wes/service-info")
        .with_body(body.to_string())
        .expect(1)
        .create();
    let config = json!({"WES": {"base_path": root("/si-native-wes")}});
    let output = run(&config, &["service-info", "--service", "WES"]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(output.stderr.is_empty());
    assert_eq!(stdout(&output), body.to_string());
    response.assert();
}

#[test]
fn a_refused_connection_is_an_error() {
    // Bind a free port and release it, so that connections to it are refused.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let config = json!({"TES": {"base_path": format!("http://127.0.0.1:{port}/")}});
    let output = run(&config, &["service-info", "--service", "TES"]);
    assert_failure(&output, "Error requesting service-info of section 'TES'");
}

#[test]
fn the_discovery_timeout_applies() {
    // A server that accepts the connection but never answers.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0; 1024];
        while matches!(stream.read(&mut buffer), Ok(n) if n > 0) {}
    });
    let config = json!({"TES": {
        "base_path": format!("http://{address}/ga4gh/tes/v1"),
        "timeouts": {"discovery": 0.5, "request": 30},
    }});
    let started = Instant::now();
    let output = run(&config, &["service-info", "--service", "TES"]);
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_failure(&output, "deadline exceeded");
    server.join().unwrap();
}

#[tokio::test]
async fn dispatch_with_config_uses_the_supplied_configuration() {
    let info = mock("GET", "/si-dispatch/ga4gh/wes/v1/service-info")
        .with_body(service_info("wes").to_string())
        .create();
    let config = Configuration::new(Url::parse(&root("/si-dispatch/ga4gh/wes/v1")).unwrap());
    let matches = command()
        .try_get_matches_from(["ga4gh-cli", "service-info", "--service", "WES"])
        .unwrap();
    let output = dispatch_with_config(&matches, &config)
        .await
        .unwrap()
        .expect("service-info is a built-in command");
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    let printed: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(printed, service_info("wes"));
    info.assert();
}

#[tokio::test]
async fn context_dispatch_reads_the_section_of_the_selected_profile() {
    let info = mock("GET", "/si-context-dispatch/service-info")
        .with_body(service_info("my-service").to_string())
        .create();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    fs::write(
        &path,
        json!({"profiles": {
            "default": {"MY_SERVICE": {"base_path": "http://127.0.0.1:9/"}},
            "other": {"MY_SERVICE": {"base_path": root("/si-context-dispatch")}}
        }})
        .to_string(),
    )
    .unwrap();
    let context = CliContext::new("other-cli", "1.0.0").with_config_path(&path);
    let matches = context
        .command()
        .try_get_matches_from([
            "other-cli",
            "--profile",
            "other",
            "service-info",
            "--service",
            "MY_SERVICE",
        ])
        .unwrap();
    let output = context
        .dispatch(&matches)
        .await
        .unwrap()
        .expect("service-info is a built-in command");
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert_eq!(output.stdout, service_info("my-service").to_string());
    info.assert();
}

#[tokio::test]
async fn context_dispatch_to_reads_the_section_of_its_own_file() {
    let info = mock("GET", "/si-context/service-info")
        .with_body(service_info("my-service").to_string())
        .create();
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    fs::write(
        &path,
        json!({"MY_SERVICE": {"base_path": root("/si-context")}}).to_string(),
    )
    .unwrap();
    let context = CliContext::new("other-cli", "1.0.0").with_config_path(&path);
    let matches = context
        .command()
        .try_get_matches_from(["other-cli", "service-info", "--service", "MY_SERVICE"])
        .unwrap();
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let exit_code = context
        .dispatch_to(&matches, &mut out, &mut err)
        .await
        .unwrap();
    assert_eq!(exit_code, Some(ExitCode::SUCCESS));
    assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
    let printed: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(printed["id"], "org.example.my-service");
    info.assert();
}
