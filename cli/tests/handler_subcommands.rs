//! The TES, WES and registry handlers reject matches without a known
//! subcommand, as the DRS and TRS handlers do.
//!
//! The built-in commands require a subcommand, so only matches of a
//! downstream binary's own commands reach the handlers without one.

use clap::{ArgMatches, Command};
use ga4gh_cli::{registry, tes, wes, CommandOutput};
use ga4gh_sdk::utils::configuration::Configuration;
use url::Url;

/// Matches of the command `name` with an optional subcommand `other`, parsed
/// from `args`.
fn matches(name: &'static str, args: &[&str]) -> ArgMatches {
    Command::new(name)
        .subcommand(Command::new("other"))
        .try_get_matches_from(args)
        .unwrap()
}

/// A configuration whose service is never asked: the handlers fail first.
fn config() -> Configuration {
    Configuration::new(Url::parse("http://127.0.0.1:9").unwrap())
}

/// Checks that a handler returned an error with `expected` as its message.
fn assert_error(result: anyhow::Result<CommandOutput>, expected: &str) {
    match result {
        Ok(output) => panic!(
            "expected {expected:?}, got exit code {:?} with stdout {:?} and stderr {:?}",
            output.exit_code, output.stdout, output.stderr
        ),
        Err(error) => assert_eq!(error.to_string(), expected),
    }
}

#[tokio::test]
async fn tes_needs_a_known_subcommand() {
    let config = config();
    assert_error(
        tes::run(&matches("tes", &["tes"]), &config).await,
        "TES subcommand is required",
    );
    assert_error(
        tes::run(&matches("tes", &["tes", "other"]), &config).await,
        "Unrecognized TES command: other",
    );
}

#[tokio::test]
async fn wes_needs_a_known_subcommand() {
    let config = config();
    assert_error(
        wes::run(&matches("wes", &["wes"]), &config).await,
        "WES subcommand is required",
    );
    assert_error(
        wes::run(&matches("wes", &["wes", "other"]), &config).await,
        "Unrecognized WES command: other",
    );
}

#[tokio::test]
async fn registry_needs_a_known_subcommand() {
    let config = config();
    assert_error(
        registry::run(&matches("registry", &["registry"]), &config).await,
        "Registry subcommand is required",
    );
    assert_error(
        registry::run(&matches("registry", &["registry", "other"]), &config).await,
        "Unrecognized registry command: other",
    );
}
