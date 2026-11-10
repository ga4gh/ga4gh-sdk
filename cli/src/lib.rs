//! Reusable command definitions and dispatch for the GA4GH CLI.
//!
//! Downstream binaries can extend the Clap 4 command and handle their own
//! subcommands after the built-in dispatcher returns `None`:
//!
//! ```no_run
//! use clap::Command;
//! use ga4gh_cli::{command, dispatch_with_config};
//! use ga4gh_sdk::utils::configuration::Configuration;
//! use url::Url;
//! # async fn example() -> anyhow::Result<()> {
//! let matches = command().subcommand(Command::new("custom"))
//!     .try_get_matches_from(["ga4gh-cli", "custom"])?;
//! let config = Configuration::new(Url::parse("http://localhost:8000/ga4gh/tes/v1")?);
//! if let Some(output) = dispatch_with_config(&matches, &config).await? {
//!     print!("{}", output.stdout);
//!     eprint!("{}", output.stderr);
//! } else {
//!     // Handle the custom subcommand.
//! }
//! # Ok(())
//! # }
//! ```

pub mod drs;
pub mod tes;
pub mod trs;

use clap::{ArgMatches, Command};
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use std::process::ExitCode;

/// Output of a built-in command. Callers decide where to write each stream.
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: ExitCode,
}

impl CommandOutput {
    fn success(stdout: String) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            exit_code: ExitCode::SUCCESS,
        }
    }

    fn failure(message: impl std::fmt::Display) -> Self {
        Self {
            stdout: String::new(),
            stderr: format!("{message}\n"),
            exit_code: ExitCode::FAILURE,
        }
    }
}

/// Build the root command. Downstream CLIs can append their own subcommands.
pub fn command() -> Command {
    Command::new("ga4gh-cli")
        .bin_name("ga4gh-cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("CLI for GA4GH services")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(drs::command())
        .subcommand(tes::command())
        .subcommand(trs::command())
}

/// Run a built-in command using its service section in the config file.
///
/// `None` leaves an added command to its caller without reading the config file.
/// Config and input errors are returned as `Err`. Service request failures return
/// a `CommandOutput` with an error message and a failure exit code.
pub async fn dispatch(matches: &ArgMatches) -> anyhow::Result<Option<CommandOutput>> {
    let service_type = match matches.subcommand() {
        Some(("drs", _)) => ServiceType::DRS,
        Some(("tes", _)) => ServiceType::TES,
        Some(("trs", _)) => ServiceType::TRS,
        _ => return Ok(None),
    };
    let config = Configuration::from_file(service_type)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    dispatch_with_config(matches, &config).await
}

/// Run a built-in command with a caller-supplied service configuration.
///
/// Returns `None` for commands not owned by this crate. The caller can inspect
/// or reformat both output streams without changing process-global state.
pub async fn dispatch_with_config(
    matches: &ArgMatches,
    config: &Configuration,
) -> anyhow::Result<Option<CommandOutput>> {
    match matches.subcommand() {
        Some(("drs", sub)) => drs::run(sub, config).await.map(Some),
        Some(("tes", sub)) => tes::run(sub, config).await.map(Some),
        Some(("trs", sub)) => trs::run(sub, config).await.map(Some),
        _ => Ok(None),
    }
}

/// Built-in streaming dispatcher. Writes incrementally to caller-owned sinks.
/// Added downstream commands return None before reading configuration.
pub async fn dispatch_to(
    matches: &ArgMatches,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> anyhow::Result<Option<ExitCode>> {
    let service_type = match matches.subcommand() {
        Some(("drs", _)) => ServiceType::DRS,
        Some(("tes", _)) => ServiceType::TES,
        Some(("trs", _)) => ServiceType::TRS,
        _ => return Ok(None),
    };
    let config = Configuration::from_file(service_type)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    dispatch_with_config_to(matches, &config, stdout, stderr).await
}

/// Streaming counterpart of dispatch_with_config, including bounded JSONL export.
pub async fn dispatch_with_config_to(
    matches: &ArgMatches,
    config: &Configuration,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> anyhow::Result<Option<ExitCode>> {
    if let Some(("tes", sub)) = matches.subcommand() {
        if let Some(("list", list)) = sub.subcommand() {
            if list.get_flag("all") {
                return tes::export_all(list, config, stdout, stderr)
                    .await
                    .map(Some);
            }
        }
    }
    match dispatch_with_config(matches, config).await? {
        Some(output) => {
            stdout.write_all(output.stdout.as_bytes())?;
            stderr.write_all(output.stderr.as_bytes())?;
            Ok(Some(output.exit_code))
        }
        None => Ok(None),
    }
}
