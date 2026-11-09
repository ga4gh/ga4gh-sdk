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

pub mod tes;

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
}

/// Build the root command. Downstream CLIs can append their own subcommands.
pub fn command() -> Command {
    Command::new("ga4gh-cli")
        .bin_name("ga4gh-cli")
        .version(env!("CARGO_PKG_VERSION"))
        .about("CLI to manage tasks")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(tes::command())
}

/// Run a built-in command using the usual TES config file.
///
/// `None` leaves an added command to its caller without reading the config file.
/// Errors include config, input, and service setup failures. Some TES request
/// failures still return a successful exit code; issue #83 will change that.
pub async fn dispatch(matches: &ArgMatches) -> anyhow::Result<Option<CommandOutput>> {
    match matches.subcommand() {
        Some(("tes", _)) => {
            let config = Configuration::from_file(ServiceType::TES)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            dispatch_with_config(matches, &config).await
        }
        _ => Ok(None),
    }
}

/// Run a built-in command with a caller-supplied TES configuration.
///
/// Returns `None` for commands not owned by this crate. The caller can inspect
/// or reformat both output streams without changing process-global state.
pub async fn dispatch_with_config(
    matches: &ArgMatches,
    config: &Configuration,
) -> anyhow::Result<Option<CommandOutput>> {
    match matches.subcommand() {
        Some(("tes", sub)) => tes::run(sub, config).await.map(Some),
        _ => Ok(None),
    }
}
