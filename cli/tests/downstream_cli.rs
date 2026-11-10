//! Compile and exercise the public CLI extension point as a downstream crate.

use clap::{Arg, Command};
use ga4gh_cli::{command, dispatch, dispatch_with_config};
use ga4gh_sdk::utils::configuration::Configuration;
use mockito::mock;
use url::Url;

#[tokio::test]
async fn downstream_command_can_be_parsed_and_dispatched() {
    let matches = command()
        .subcommand(Command::new("custom").arg(Arg::new("value").required(true)))
        .try_get_matches_from(["ga4gh-cli", "custom", "hello"])
        .expect("custom command should parse");

    // The SDK CLI leaves commands it does not own to the downstream caller.
    assert!(dispatch(&matches).await.unwrap().is_none());
    let output = match matches.subcommand() {
        Some(("custom", args)) => args.get_one::<String>("value").unwrap().to_owned(),
        _ => panic!("custom command was not dispatched"),
    };
    assert_eq!(output, "hello");
}

#[tokio::test]
async fn downstream_can_inject_config_and_capture_tes_output() {
    let _service_info = mock("GET", "/ga4gh/tes/v1/service-info")
        .with_status(200)
        .with_body(r#"{"id":"test","name":"Test","type":{"group":"org.ga4gh","artifact":"tes","version":"1.1.0"},"organization":{"name":"Test","url":"https://example.org"},"version":"1.0.0"}"#)
        .create();
    let _tasks = mock("GET", "/ga4gh/tes/v1/tasks")
        .with_status(200)
        .with_body(r#"{"tasks":[{"id":"task-from-test","state":"COMPLETE"}]}"#)
        .create();
    let config =
        Configuration::new(Url::parse(&format!("{}/ga4gh/tes/v1", mockito::server_url())).unwrap());
    let matches = command()
        .try_get_matches_from(["ga4gh-cli", "tes", "list"])
        .unwrap();

    let output = dispatch_with_config(&matches, &config)
        .await
        .unwrap()
        .expect("TES is a built-in command");
    assert!(output.stdout.contains("task-from-test"));
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_code, std::process::ExitCode::SUCCESS);
}

#[tokio::test]
async fn unreadable_task_file_reports_read_error() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().to_str().unwrap();
    let matches = command()
        .try_get_matches_from(["ga4gh-cli", "tes", "create", path])
        .unwrap();
    let config = Configuration::new(Url::parse("http://localhost:8000/ga4gh/tes/v1").unwrap());

    let error = dispatch_with_config(&matches, &config)
        .await
        .err()
        .expect("reading a directory should fail");
    assert!(error.to_string().contains("Failed to read file:"));
}

#[test]
fn tes_list_options_keep_their_names_after_clap_upgrade() {
    let matches = command()
        .try_get_matches_from([
            "ga4gh-cli",
            "tes",
            "list",
            "--name_prefix",
            "sample",
            "--state",
            "QUEUED",
            "--page_size",
            "5",
        ])
        .unwrap();
    let (_, tes) = matches.subcommand().unwrap();
    let (_, list) = tes.subcommand().unwrap();
    assert_eq!(
        list.get_one::<String>("name_prefix").map(String::as_str),
        Some("sample")
    );
    assert_eq!(
        list.get_one::<String>("state").map(String::as_str),
        Some("QUEUED")
    );
    assert_eq!(
        list.get_one::<String>("page_size").map(String::as_str),
        Some("5")
    );
}
