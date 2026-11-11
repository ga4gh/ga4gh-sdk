//! A CLI that extends the SDK's CLI with a `tes usage` subcommand.
//!
//! The command tree is the SDK's ([`ga4gh_cli::command`]) with one added
//! subcommand; all built-in commands keep working unchanged, because
//! commands that this crate doesn't own go to the SDK's dispatcher.

use crate::usage::UsageClient;
use clap::{Arg, ArgMatches, Command};
use ga4gh_cli::printable;
use ga4gh_sdk::utils::configuration::Configuration;
use std::io::Write;
use std::process::ExitCode;

/// The name of the binary.
pub const BIN_NAME: &str = "downstream-cli";

/// Builds the SDK's command tree, plus `tes usage <ID>`.
pub fn command() -> Command {
    ga4gh_cli::command()
        .name(BIN_NAME)
        .bin_name(BIN_NAME)
        .version(env!("CARGO_PKG_VERSION"))
        .about("CLI for GA4GH services, with TES extensions")
        .mut_subcommand("tes", |tes| {
            tes.subcommand(
                Command::new("usage")
                    .about("Show the resource usage of a task (service extension)")
                    .arg(Arg::new("ID").required(true).help("Task ID")),
            )
        })
}

/// Returns the task ID if `matches` holds this crate's `tes usage` command.
pub fn usage_task_id(matches: &ArgMatches) -> Option<&str> {
    match matches.subcommand() {
        Some(("tes", tes)) => match tes.subcommand() {
            Some(("usage", usage)) => usage.get_one::<String>("ID").map(String::as_str),
            _ => None,
        },
        _ => None,
    }
}

/// Runs this crate's own commands; returns `None` for all other commands.
///
/// Output and errors from the service go to `stdout` and `stderr`, with
/// control characters escaped by [`printable`], as in the SDK's CLI.
pub async fn run_extension(
    matches: &ArgMatches,
    config: &Configuration,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> anyhow::Result<Option<ExitCode>> {
    let Some(id) = usage_task_id(matches) else {
        return Ok(None);
    };
    match UsageClient::new(config).task_usage(id).await {
        Ok(usage) => {
            let cpu = usage
                .cpu_seconds
                .map_or("-".into(), |value| value.to_string());
            let memory = usage
                .peak_memory_bytes
                .map_or("-".into(), |value| value.to_string());
            writeln!(stdout, "{}\t{cpu}\t{memory}", printable(&usage.id))?;
            Ok(Some(ExitCode::SUCCESS))
        }
        Err(error) => {
            writeln!(stderr, "{}", printable(&error.to_string()))?;
            Ok(Some(ExitCode::FAILURE))
        }
    }
}

/// Runs any command, built-in or added, with `config` for its service.
///
/// Returns `None` for a command that neither this crate nor the SDK owns.
pub async fn run_with_config(
    matches: &ArgMatches,
    config: &Configuration,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> anyhow::Result<Option<ExitCode>> {
    if let Some(exit_code) = run_extension(matches, config, stdout, stderr).await? {
        return Ok(Some(exit_code));
    }
    ga4gh_cli::dispatch_with_config_to(matches, config, stdout, stderr).await
}
