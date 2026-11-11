//! A CLI that extends the SDK's CLI with a `tes usage` subcommand.
//!
//! The command tree is the SDK's ([`ga4gh_cli::command`]) with one added
//! subcommand; all built-in commands keep working unchanged, because
//! commands that this crate doesn't own go to the SDK's dispatcher.

use crate::usage::UsageClient;
use clap::{Arg, ArgMatches, Command};
use ga4gh_cli::printable;
use ga4gh_sdk::clients::ServiceType;
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

/// Runs any command, as the `downstream-cli` binary does.
///
/// Each command reads its service's section of `~/.ga4gh-cli/config.json`:
/// `tes usage` the `TES` section, built-in commands the section that the
/// SDK's [`ga4gh_cli::dispatch_to`] picks for them. A command that neither
/// this crate nor the SDK owns writes an error and fails.
///
/// # Errors
///
/// Fails if the configuration can't be read, or for the errors of
/// [`ga4gh_cli::dispatch_to`]. Service errors are written to `stderr`
/// and give a failure exit code instead.
pub async fn run(
    matches: &ArgMatches,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> anyhow::Result<ExitCode> {
    let exit_code = if usage_task_id(matches).is_some() {
        // Only this crate's own commands read the section here.
        let config = Configuration::from_file(ServiceType::TES)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        run_extension(matches, &config, stdout, stderr).await?
    } else {
        ga4gh_cli::dispatch_to(matches, stdout, stderr).await?
    };
    match exit_code {
        Some(exit_code) => Ok(exit_code),
        None => {
            writeln!(stderr, "Error: Unrecognized command")?;
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Runs this crate's own commands with `config`; returns `None` for all
/// other commands.
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
