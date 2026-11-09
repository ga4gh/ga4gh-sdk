//! Compile and exercise the public CLI extension point as a downstream crate.

use clap::{Arg, Command};
use ga4gh_cli::{command, dispatch};

#[tokio::test]
async fn downstream_command_can_be_parsed_and_dispatched() {
    let matches = command()
        .subcommand(Command::new("custom").arg(Arg::new("value").required(true)))
        .try_get_matches_from(["ga4gh-cli", "custom", "hello"])
        .expect("custom command should parse");

    // The SDK CLI leaves commands it does not own to the downstream caller.
    assert!(dispatch(&matches).await.unwrap().is_none());
    let output = match matches.subcommand() {
        Some(("custom", args)) => args.value_of("value").unwrap().to_owned(),
        _ => panic!("custom command was not dispatched"),
    };
    assert_eq!(output, "hello");
}
