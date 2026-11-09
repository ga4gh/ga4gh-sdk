//! Reusable command definitions and dispatch for the GA4GH CLI.

pub mod tes;

use clap::{ArgMatches, Command};
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use std::error::Error;
use std::process::ExitCode;

/// Build the root command. Downstream CLIs can append their own subcommands.
pub fn command() -> Command<'static> {
    Command::new("ga4gh-cli")
        .bin_name("ga4gh-cli")
        .version("0.1.0")
        .about("CLI to manage tasks")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(tes::command())
}

/// Run a built-in command. `None` leaves an added command to its caller.
pub async fn dispatch(matches: &ArgMatches) -> Result<Option<ExitCode>, Box<dyn Error>> {
    match matches.subcommand() {
        Some(("tes", sub)) => {
            let config = Configuration::from_file(ServiceType::TES)?;
            tes::run(sub, &config).await.map(Some)
        }
        _ => Ok(None),
    }
}
