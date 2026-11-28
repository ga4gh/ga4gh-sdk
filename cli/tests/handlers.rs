//! Calls the service command handlers directly with `ArgMatches`.
//!
//! The handlers are public, so downstream binaries can pass them matches of
//! their own commands. Some of these tests build commands without arguments
//! that the built-in commands require, to check the errors the handlers
//! return for such matches. Others reach failures of the built-in commands
//! that the binary tests don't: service-info failures, invalid bodies, and
//! output streams that fail while a listing is written.

use clap::{Arg, ArgMatches, Command};
use ga4gh_cli::{drs, registry, tes, trs, wes, CommandOutput};
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::json;
use std::io::Write;
use std::process::ExitCode;
use url::Url;

/// Parses `args`, whose first item is the command name, with `command`.
fn parse(command: Command, args: &[&str]) -> ArgMatches {
    command.try_get_matches_from(args).unwrap()
}

/// The matches of the subcommand that `args` names, parsed with `command`.
fn parse_sub(command: Command, args: &[&str]) -> ArgMatches {
    parse(command, args)
        .subcommand()
        .map(|(_, sub)| sub.clone())
        .unwrap()
}

/// A command `name` with `subcommands`, each with optional string arguments.
///
/// Matches of these commands lack the arguments that the built-in commands
/// require, and their subcommand is optional.
fn lenient(name: &'static str, subcommands: &[(&'static str, &[&'static str])]) -> Command {
    subcommands
        .iter()
        .fold(Command::new(name), |command, (subcommand, args)| {
            command.subcommand(args.iter().fold(Command::new(*subcommand), |sub, arg| {
                sub.arg(Arg::new(*arg))
            }))
        })
}

/// A configuration for the API root `path` of `server`.
fn config(server: &mockito::ServerGuard, path: &str) -> Configuration {
    Configuration::new(Url::parse(&format!("{}{path}", server.url())).unwrap())
}

/// Mocks the service info of an `artifact` service at the API root `path`.
async fn service_info(
    server: &mut mockito::ServerGuard,
    path: &str,
    artifact: &str,
) -> mockito::Mock {
    server
        .mock("GET", format!("{path}/service-info").as_str())
        .with_body(
            json!({
                "id": format!("org.example.{artifact}"),
                "name": "Example",
                "type": {"group": "org.ga4gh", "artifact": artifact, "version": "1.1.0"},
                "organization": {"name": "Example", "url": "https://example.org"},
                "version": "1.0.0",
            })
            .to_string(),
        )
        .create_async()
        .await
}

/// Asserts that `output` is a failure whose message contains `expected`.
fn assert_failure(output: &CommandOutput, expected: &str) {
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(output.stderr.contains(expected), "{}", output.stderr);
}

/// Asserts that `result` is an error whose message contains `expected`.
fn assert_error<T>(result: anyhow::Result<T>, expected: &str) {
    let Err(error) = result else {
        panic!("expected an error containing {expected:?}");
    };
    assert!(error.to_string().contains(expected), "{error}");
}

/// A writer that fails on writes that contain `needle` and accepts the rest.
struct FailingWriter {
    needle: &'static str,
}

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if String::from_utf8_lossy(buf).contains(self.needle) {
            return Err(std::io::Error::other("closed"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn tes_handlers_reject_matches_without_required_arguments() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let command = || {
        lenient(
            "tes",
            &[
                ("create", &["TASK_FILE"]),
                ("get", &["id", "view"]),
                ("status", &["id"]),
                ("cancel", &["id"]),
                ("files", &["id"]),
            ],
        )
    };
    for (args, expected) in [
        (&["tes", "create"][..], "TASK_FILE argument is required"),
        (&["tes", "get"], "id argument is required"),
        (&["tes", "get", "t1"], "view argument is required"),
        (&["tes", "status"], "id argument is required"),
        (&["tes", "cancel"], "id argument is required"),
        (&["tes", "files"], "id argument is required"),
    ] {
        assert_error(tes::run(&parse(command(), args), &config).await, expected);
    }
    // Matches without a subcommand that the handler knows produce no output.
    let output = tes::run(&parse(command(), &["tes"]), &config)
        .await
        .unwrap();
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[tokio::test]
async fn tes_create_and_list_fail_before_any_request() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("task.json");
    let missing = missing.to_str().unwrap();
    let output = tes::run(&parse(tes::command(), &["tes", "create", missing]), &config)
        .await
        .unwrap();
    assert_failure(&output, "File does not exist");
    // The streaming listing needs the writers of `export_all`.
    assert_error(
        tes::run(&parse(tes::command(), &["tes", "list", "--all"]), &config).await,
        "--all needs the streaming",
    );
    assert_error(
        tes::run(
            &parse(tes::command(), &["tes", "list", "--format", "jsonl"]),
            &config,
        )
        .await,
        "--format jsonl requires --all",
    );
}

#[tokio::test]
async fn tes_files_reports_task_bodies_that_are_not_json() {
    let mut server = mockito::Server::new_async().await;
    let _info = service_info(&mut server, "", "tes").await;
    let _task = server
        .mock("GET", "/tasks/t1?view=FULL")
        .with_body("not json")
        .create_async()
        .await;
    let output = tes::run(
        &parse(tes::command(), &["tes", "files", "t1"]),
        &config(&server, ""),
    )
    .await
    .unwrap();
    assert_failure(&output, "The response is not valid JSON");
}

#[tokio::test]
async fn tes_save_without_a_directory_is_an_error() {
    let mut server = mockito::Server::new_async().await;
    let _info = service_info(&mut server, "", "tes").await;
    let _task = server
        .mock("GET", "/tasks/t1?view=FULL")
        .with_body(json!({"id": "t1"}).to_string())
        .create_async()
        .await;
    let command = Command::new("tes").subcommand(
        Command::new("save")
            .arg(Arg::new("id"))
            .arg(Arg::new("SELECTOR").num_args(1..))
            .arg(Arg::new("dir").long("dir")),
    );
    assert_error(
        tes::run(
            &parse(command, &["tes", "save", "t1", "stdout:0:0"]),
            &config(&server, ""),
        )
        .await,
        "--dir is required",
    );
}

#[tokio::test]
async fn tes_export_checks_its_filters_before_any_request() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    for (args, expected) in [
        (
            &["tes", "list", "--all"][..],
            "--all requires --format jsonl",
        ),
        (
            &["tes", "list", "--all", "--format", "jsonl", "-p", "ten"],
            "page_size must be an integer",
        ),
        (
            &["tes", "list", "--all", "--format", "jsonl", "-p", "2048"],
            "page_size must be 1..2047",
        ),
        (
            &[
                "tes", "list", "--all", "--format", "jsonl", "-k", "a", "-v", "1", "-v", "2",
            ],
            "Each tag_value needs a corresponding tag_key",
        ),
    ] {
        let sub = parse_sub(tes::command(), args);
        assert_error(
            tes::export_all(&sub, &config, &mut stdout, &mut stderr).await,
            expected,
        );
    }
    assert!(stdout.is_empty() && stderr.is_empty());
}

#[tokio::test]
async fn tes_export_reports_service_and_pager_failures() {
    let mut server = mockito::Server::new_async().await;
    // Without a service-info mock, the server answers 501.
    let sub = parse_sub(
        tes::command(),
        &[
            "tes", "list", "--all", "--format", "jsonl", "-n", "run", "-t", "token", "-s",
            "running", "-p", "5", "-k", "a", "-v", "1",
        ],
    );
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = tes::export_all(&sub, &config(&server, "/none"), &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(exit_code, ExitCode::FAILURE);
    assert!(stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&stderr).contains("501"),
        "{stderr:?}"
    );

    let _info = service_info(&mut server, "", "tes").await;
    let sub = parse_sub(
        tes::command(),
        &[
            "tes",
            "list",
            "--all",
            "--format",
            "jsonl",
            "--max-pages",
            "0",
        ],
    );
    assert_error(
        tes::export_all(&sub, &config(&server, ""), &mut stdout, &mut stderr).await,
        "max_requests must be positive",
    );
}

#[tokio::test]
async fn tes_export_returns_errors_of_the_diagnostics_stream() {
    let mut server = mockito::Server::new_async().await;
    let _info = service_info(&mut server, "", "tes").await;
    let _first = server
        .mock("GET", "/tasks")
        .match_query(mockito::Matcher::Missing)
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"next"}"#)
        .create_async()
        .await;
    let _second = server
        .mock("GET", "/tasks")
        .match_query(mockito::Matcher::UrlEncoded(
            "page_token".into(),
            "next".into(),
        ))
        .with_status(503)
        .create_async()
        .await;
    let config = config(&server, "");
    for args in [
        // Stops at the limit, before the second page
        &[
            "tes",
            "list",
            "--all",
            "--format",
            "jsonl",
            "--max-pages",
            "1",
        ][..],
        // Fails at the second page
        &["tes", "list", "--all", "--format", "jsonl"],
    ] {
        let sub = parse_sub(tes::command(), args);
        let mut stdout = Vec::new();
        // Matches the checkpoint lines of `write_jsonl` ("Listing stopped
        // at configured limit; resume checkpoint: " and "Resume checkpoint: "),
        // but not the failure message before the latter.
        let mut stderr = FailingWriter {
            needle: "esume checkpoint",
        };
        assert_error(
            tes::export_all(&sub, &config, &mut stdout, &mut stderr).await,
            "closed",
        );
        assert_eq!(String::from_utf8(stdout).unwrap(), "{\"id\":\"one\"}\n");
    }
}

#[tokio::test]
async fn wes_handlers_reject_matches_without_required_arguments() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let command = || {
        lenient(
            "wes",
            &[
                ("run", &["type", "type_version", "WORKFLOW_URL"]),
                ("get", &["id"]),
            ],
        )
    };
    assert_error(
        wes::run(&parse(command(), &["wes", "run"]), &config).await,
        "type argument is required",
    );
    assert_error(
        wes::run(&parse(command(), &["wes", "get"]), &config).await,
        "id argument is required",
    );
    let output = wes::run(&parse(command(), &["wes"]), &config)
        .await
        .unwrap();
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[tokio::test]
async fn wes_run_and_list_fail_before_any_request() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    // A directory exists, but can't be read as a file.
    let directory = tempfile::tempdir().unwrap();
    let args = [
        "wes",
        "run",
        "main.cwl",
        "--type",
        "CWL",
        "--type_version",
        "v1.2",
        "--params",
        directory.path().to_str().unwrap(),
    ];
    assert_error(
        wes::run(&parse(wes::command(), &args), &config).await,
        "Failed to read file",
    );
    assert_error(
        wes::run(&parse(wes::command(), &["wes", "list", "--all"]), &config).await,
        "--all needs the streaming",
    );
}

#[tokio::test]
async fn wes_get_and_list_report_service_info_failures() {
    // Without a service-info mock, the server answers 501.
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    for args in [&["wes", "get", "r1"][..], &["wes", "list"]] {
        let output = wes::run(&parse(wes::command(), args), &config)
            .await
            .unwrap();
        assert_failure(&output, "501");
    }
}

#[tokio::test]
async fn wes_export_reports_service_and_pager_failures() {
    let mut server = mockito::Server::new_async().await;
    let sub = parse_sub(
        wes::command(),
        &[
            "wes", "list", "--all", "--format", "jsonl", "-p", "5", "-t", "token",
        ],
    );
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = wes::export_all(&sub, &config(&server, "/none"), &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(exit_code, ExitCode::FAILURE);
    assert!(stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&stderr).contains("501"),
        "{stderr:?}"
    );

    let _info = service_info(&mut server, "", "wes").await;
    let sub = parse_sub(
        wes::command(),
        &[
            "wes",
            "list",
            "--all",
            "--format",
            "jsonl",
            "--max-pages",
            "0",
        ],
    );
    assert_error(
        wes::export_all(&sub, &config(&server, ""), &mut stdout, &mut stderr).await,
        "max_requests must be positive",
    );
}

#[tokio::test]
async fn drs_handler_rejects_matches_without_required_arguments() {
    let mut server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let command = || {
        lenient(
            "drs",
            &[
                ("get", &["OBJECT_ID"]),
                ("access", &["OBJECT_ID", "ACCESS_ID"]),
                ("other", &["OBJECT_ID"]),
            ],
        )
    };
    for (args, expected) in [
        (&["drs"][..], "DRS subcommand is required"),
        (&["drs", "get"], "OBJECT_ID argument is required"),
        (&["drs", "access", "o1"], "ACCESS_ID argument is required"),
    ] {
        assert_error(drs::run(&parse(command(), args), &config).await, expected);
    }
    // The subcommand is checked once the service is known.
    let _info = service_info(&mut server, "", "drs").await;
    assert_error(
        drs::run(&parse(command(), &["drs", "other", "o1"]), &config).await,
        "Unrecognized DRS command: other",
    );
}

#[tokio::test]
async fn drs_get_reports_service_info_failures() {
    // Without a service-info mock, the server answers 501.
    let server = mockito::Server::new_async().await;
    let output = drs::run(
        &parse(drs::command(), &["drs", "get", "o1"]),
        &config(&server, ""),
    )
    .await
    .unwrap();
    assert_failure(&output, "501");
}

#[tokio::test]
async fn registry_handler_rejects_matches_without_required_arguments() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let command = || lenient("registry", &[("get", &["id"])]);
    assert_error(
        registry::run(&parse(command(), &["registry", "get"]), &config).await,
        "id argument is required",
    );
    let output = registry::run(&parse(command(), &["registry"]), &config)
        .await
        .unwrap();
    assert_eq!(output.exit_code, ExitCode::SUCCESS);
    assert!(output.stdout.is_empty() && output.stderr.is_empty());
}

#[tokio::test]
async fn trs_handler_rejects_invalid_matches_before_any_request() {
    let server = mockito::Server::new_async().await;
    let config = config(&server, "");
    let command = || {
        lenient("trs", &[("tool", &["TOOL_ID"]), ("other", &[])])
            .arg(Arg::new("api-version").long("api-version"))
            .arg(Arg::new("tag-filter").long("tag-filter"))
    };
    for (args, expected) in [
        (&["trs"][..], "TRS subcommand is required"),
        (&["trs", "other"], "Unrecognized TRS command: other"),
        (
            &["trs", "--api-version", "1.0", "tool", "t1"],
            "Invalid TRS API version",
        ),
        (
            &["trs", "--tag-filter", "sometimes", "tool", "t1"],
            "Invalid TRS tag-filter policy",
        ),
    ] {
        assert_error(trs::run(&parse(command(), args), &config).await, expected);
    }
    // A disabled tag filter rejects tags before the request.
    let args = [
        "trs",
        "--no-discovery",
        "--api-version",
        "2.0.2",
        "--tag-filter",
        "disabled",
        "tools",
        "--tag",
        "x",
    ];
    let output = trs::run(&parse(trs::command(), &args), &config)
        .await
        .unwrap();
    assert_failure(&output, "tags");
}
