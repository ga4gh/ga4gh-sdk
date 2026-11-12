//! A CLI that extends the SDK's CLI with a `tes usage` subcommand and a
//! top-level `quota` command, under its own name, version, configuration
//! file and token variable.
//!
//! The command tree is the SDK's ([`ga4gh_cli::CliContext::command`]) with
//! two added subcommands; all built-in commands keep working unchanged,
//! because commands that this crate doesn't own go to the SDK's dispatcher.

use crate::usage::UsageClient;
use clap::{Arg, ArgMatches, Command};
use ga4gh_cli::{printable, CliContext, CommandOutput};
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::Transport;
use std::io::Write;
use std::process::ExitCode;

/// The name of the binary.
pub const BIN_NAME: &str = "downstream-cli";

/// The environment variable with an access token for sections that set no
/// credentials.
pub const TOKEN_ENV: &str = "DOWNSTREAM_CLI_TOKEN";

/// The configuration section of the made-up quota service.
pub const QUOTA_SECTION: &str = "QUOTA";

/// The binary's settings: its name and version, the configuration file
/// `~/.downstream-cli/config.json` (the default for the name) and the token
/// variable [`TOKEN_ENV`] for the sections of its own services, `TES` and
/// `QUOTA`. Other sections never receive that token.
pub fn context() -> CliContext {
    CliContext::new(BIN_NAME, env!("CARGO_PKG_VERSION"))
        .with_about("CLI for GA4GH services, with service extensions")
        .with_token_env(
            TOKEN_ENV,
            [ServiceType::TES, ServiceType::Custom(QUOTA_SECTION.into())],
        )
}

/// Builds the SDK's command tree for [`context`], plus `tes usage <ID>` and
/// `quota`.
pub fn command() -> Command {
    context()
        .command()
        .mut_subcommand("tes", |tes| {
            tes.subcommand(
                Command::new("usage")
                    .about("Show the resource usage of a task (service extension)")
                    .arg(Arg::new("ID").required(true).help("Task ID")),
            )
        })
        .subcommand(Command::new("quota").about("Show the caller's quota (service extension)"))
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
/// Each command reads its service's section of the configuration file of
/// [`context`]: `tes usage` the `TES` section, `quota` the `QUOTA` section,
/// built-in commands the section that [`CliContext::dispatch_to`] picks for
/// them. A command that neither this crate nor the SDK owns writes an error
/// and fails.
///
/// # Errors
///
/// Fails if the configuration can't be read, or for the errors of
/// [`CliContext::dispatch_to`]. Service errors are written to `stderr` and
/// give a failure exit code instead.
pub async fn run(
    matches: &ArgMatches,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
) -> anyhow::Result<ExitCode> {
    // `--profile` selects the profile for this crate's own commands too.
    let context = context().with_profile_from(matches);
    // Only this crate's own commands read their sections here.
    let exit_code = if usage_task_id(matches).is_some() {
        let config = context.load_config(ServiceType::TES)?;
        run_extension(matches, &config, stdout, stderr).await?
    } else if let Some(("quota", _)) = matches.subcommand() {
        let config = context.load_config(ServiceType::Custom(QUOTA_SECTION.into()))?;
        Some(quota(&config).await.write_to(stdout, stderr)?)
    } else {
        context.dispatch_to(matches, stdout, stderr).await?
    };
    match exit_code {
        Some(exit_code) => Ok(exit_code),
        None => {
            writeln!(stderr, "Error: Unrecognized command")?;
            Ok(ExitCode::FAILURE)
        }
    }
}

/// Runs `tes usage` with `config`; returns `None` for all other commands.
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

/// Fetches `GET /quota` from the quota service and shows the JSON response.
///
/// The SDK's [`CommandOutput`] helpers escape the output and set the exit
/// code as for the built-in commands: [`CommandOutput::json`] for the
/// response, [`CommandOutput::failure`] for an error.
///
/// `Transport::new` doesn't send configured credentials yet
/// ([#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)); a service that
/// needs them takes a client with an `Authorization` header through
/// `Transport::with_client`.
pub async fn quota(config: &Configuration) -> CommandOutput {
    match Transport::new(config).get("quota", None).await {
        Ok(body) => CommandOutput::json(body),
        Err(error) => CommandOutput::failure(error),
    }
}
