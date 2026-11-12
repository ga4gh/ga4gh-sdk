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
//!
//! A binary with its own name, version, configuration file, profile
//! variable or token variable describes them in a [`CliContext`] and uses its methods in place
//! of [`command`], [`dispatch`] and [`dispatch_to`]. [`CommandOutput`] and
//! [`printable`] give its own commands the output and exit codes of the
//! built-in ones.

pub mod drs;
pub mod registry;
mod render;
pub mod tes;
pub mod trs;
pub mod wes;
pub mod workflow;

pub use render::printable;

use clap::{Arg, ArgMatches, Command};
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use std::path::PathBuf;
use std::process::ExitCode;

/// Output of a command. Callers decide where to write each stream.
///
/// Downstream commands can build it with [`success`](Self::success),
/// [`failure`](Self::failure) and [`json`](Self::json), so that their output
/// is escaped and their exit codes are set as for the built-in commands.
pub struct CommandOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: ExitCode,
}

impl CommandOutput {
    /// Successful output with `stdout` as given and an empty `stderr`.
    ///
    /// `stdout` is not escaped: pass values from a service through
    /// [`printable`] first.
    pub fn success(stdout: String) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            exit_code: ExitCode::SUCCESS,
        }
    }

    /// A failure with `message` and a newline on `stderr`.
    ///
    /// Error messages can hold a service's response body, so they are always
    /// shown with [`printable`].
    pub fn failure(message: impl std::fmt::Display) -> Self {
        Self {
            stdout: String::new(),
            stderr: format!("{}\n", render::printable(&message.to_string())),
            exit_code: ExitCode::FAILURE,
        }
    }

    /// Writes `stdout` and `stderr` to the given writers and returns the exit code.
    ///
    /// # Errors
    ///
    /// Fails if a writer fails.
    pub fn write_to(
        self,
        stdout: &mut dyn std::io::Write,
        stderr: &mut dyn std::io::Write,
    ) -> std::io::Result<ExitCode> {
        stdout.write_all(self.stdout.as_bytes())?;
        stderr.write_all(self.stderr.as_bytes())?;
        Ok(self.exit_code)
    }

    /// Successful output of a JSON response body, such as one from a service.
    ///
    /// Preserves the JSON value while escaping terminal controls inside its
    /// strings. A body that is not JSON is reported as a failure.
    pub fn json(body: String) -> Self {
        // Validate syntax without materialising a Value: a valid extension
        // may contain deeply nested fields or numbers outside f64's range.
        match serde_json::from_str::<serde::de::IgnoredAny>(&body) {
            Ok(_) => Self::success(render::json_printable(&body)),
            Err(error) => Self::failure(format!("The response is not valid JSON: {error}")),
        }
    }
}

#[cfg(test)]
mod output_tests {
    use super::CommandOutput;

    #[test]
    fn valid_large_numbers_and_deep_extensions_are_preserved() {
        let nested = format!("{}0{}", "[".repeat(150), "]".repeat(150));
        let body = format!("{{\"large\":1e400,\"nested\":{nested}}}");
        let output = CommandOutput::json(body.clone());
        assert!(output.stderr.is_empty(), "{}", output.stderr);
        assert_eq!(output.stdout, body);

        let invalid = CommandOutput::json(format!("{body} forged"));
        assert!(invalid.stdout.is_empty());
        assert!(invalid.stderr.contains("not valid JSON"));
    }
}

/// The settings of a binary built on this CLI: its name, version, help text,
/// configuration file, profile and token variable.
///
/// [`CliContext::default`] describes `ga4gh-cli`. A downstream binary creates
/// its own and uses its [`command`](Self::command),
/// [`dispatch`](Self::dispatch) and [`dispatch_to`](Self::dispatch_to) in
/// place of the free functions of the same names, which use the default:
///
/// ```no_run
/// use clap::Command;
/// use ga4gh_cli::CliContext;
/// use ga4gh_sdk::clients::ServiceType;
/// # async fn example() -> anyhow::Result<std::process::ExitCode> {
/// let context = CliContext::new("my-cli", env!("CARGO_PKG_VERSION"))
///     .with_about("CLI for my services")
///     .with_token_env("MY_CLI_TOKEN", [ServiceType::Custom("MY_SERVICE".into())]);
/// let matches = context.command().subcommand(Command::new("custom")).get_matches();
/// let mut stdout = std::io::stdout();
/// match context.dispatch_to(&matches, &mut stdout, &mut std::io::stderr()).await? {
///     Some(exit_code) => Ok(exit_code),
///     // Handle the custom subcommand, for example with the section
///     // `context.load_config(ServiceType::Custom("MY_SERVICE".into()))`.
///     None => Ok(std::process::ExitCode::SUCCESS),
/// }
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct CliContext {
    name: &'static str,
    version: &'static str,
    about: Option<&'static str>,
    config_path: Option<PathBuf>,
    profile: Option<String>,
    profile_env: Option<String>,
    token_env: Option<String>,
    token_sections: Vec<String>,
}

/// The default profile variable of a binary: its name in upper case, with
/// `_` for characters other than ASCII letters and digits, and `_PROFILE`.
fn default_profile_env(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("{stem}_PROFILE")
}

impl Default for CliContext {
    /// The context of `ga4gh-cli`: configuration in `~/.ga4gh-cli/config.json`,
    /// profile variable `GA4GH_CLI_PROFILE`, no token variable.
    fn default() -> Self {
        Self::new("ga4gh-cli", env!("CARGO_PKG_VERSION")).with_about("CLI for GA4GH services")
    }
}

impl CliContext {
    /// A context for the binary `name` at `version`, such as
    /// `env!("CARGO_PKG_VERSION")`.
    ///
    /// The configuration file defaults to `~/.<name>/config.json`, the
    /// profile variable to `<NAME>_PROFILE` (`name` in upper case, with `_`
    /// for other characters than ASCII letters and digits), and no token
    /// variable is read. The texts are `&'static str`, as Clap's
    /// command names are without its `string` feature; they are usually
    /// literals or `env!` values.
    pub fn new(name: &'static str, version: &'static str) -> Self {
        Self {
            name,
            version,
            about: None,
            config_path: None,
            profile: None,
            profile_env: Some(default_profile_env(name)),
            token_env: None,
            token_sections: Vec::new(),
        }
    }

    /// Sets the description that `--help` shows.
    pub fn with_about(mut self, about: &'static str) -> Self {
        self.about = Some(about);
        self
    }

    /// Reads the configuration from `path` instead of `~/.<name>/config.json`.
    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }

    /// Selects the profile `name` of the configuration file, in place of the
    /// profile variable and the file's `default_profile`.
    ///
    /// [`dispatch`](Self::dispatch) and [`dispatch_to`](Self::dispatch_to)
    /// call it with the value of `--profile`, if it is given.
    pub fn with_profile(mut self, name: impl Into<String>) -> Self {
        self.profile = Some(name.into());
        self
    }

    /// Reads the profile from the environment variable `name` when no
    /// profile is selected with `--profile` or
    /// [`with_profile`](Self::with_profile).
    ///
    /// An unset or empty variable selects no profile, so the file's
    /// `default_profile` applies.
    pub fn with_profile_env(mut self, name: impl Into<String>) -> Self {
        self.profile_env = Some(name.into());
        self
    }

    /// Never reads a profile variable, not even the default one.
    pub fn without_profile_env(mut self) -> Self {
        self.profile_env = None;
        self
    }

    /// Reads an OAuth access token from the environment variable `name` for
    /// the given sections, such as the binary's own services, when they set
    /// no credentials.
    ///
    /// The token applies only to the listed sections, and only if they set
    /// none of `basic_auth`, `oauth_access_token` and `oauth_access_token_env`;
    /// other sections never receive it. An unset or empty variable leaves the
    /// sections without a token. To send a token to another service, name a
    /// variable in its section with `oauth_access_token_env`.
    ///
    /// Sections match by their exact name (`service_type.as_str()`, case
    /// sensitive). Another call replaces the variable and its sections; an
    /// empty list means that the variable is never read.
    pub fn with_token_env(
        mut self,
        name: impl Into<String>,
        sections: impl IntoIterator<Item = ServiceType>,
    ) -> Self {
        self.token_env = Some(name.into());
        self.token_sections = sections
            .into_iter()
            .map(|section| section.as_str().to_owned())
            .collect();
        self
    }

    /// The binary's name.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// The binary's version.
    pub fn version(&self) -> &'static str {
        self.version
    }

    /// The description that `--help` shows, if set.
    pub fn about(&self) -> Option<&'static str> {
        self.about
    }

    /// The profile selected with [`with_profile`](Self::with_profile).
    pub fn profile(&self) -> Option<&str> {
        self.profile.as_deref()
    }

    /// The profile variable; see [`with_profile_env`](Self::with_profile_env).
    pub fn profile_env(&self) -> Option<&str> {
        self.profile_env.as_deref()
    }

    /// The profile to read: the one selected with
    /// [`with_profile`](Self::with_profile), else the value of the profile
    /// variable, else `None` for the file's default profile.
    ///
    /// # Errors
    ///
    /// Fails if the profile variable is set but not valid Unicode.
    pub fn selected_profile(&self) -> anyhow::Result<Option<String>> {
        if let Some(profile) = &self.profile {
            return Ok(Some(profile.clone()));
        }
        let Some(name) = &self.profile_env else {
            return Ok(None);
        };
        match std::env::var(name) {
            Ok(profile) if !profile.is_empty() => Ok(Some(profile)),
            Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => {
                anyhow::bail!("Environment variable '{name}' is not valid Unicode")
            }
        }
    }

    /// The token variable set with [`with_token_env`](Self::with_token_env).
    pub fn token_env(&self) -> Option<&str> {
        self.token_env.as_deref()
    }

    /// The sections that the token variable applies to.
    pub fn token_sections(&self) -> &[String] {
        &self.token_sections
    }

    /// The path of the configuration file.
    ///
    /// # Errors
    ///
    /// Fails if no path was set and the home directory is unknown, or the
    /// name can't be used as a directory name.
    pub fn config_path(&self) -> anyhow::Result<PathBuf> {
        if let Some(path) = &self.config_path {
            return Ok(path.clone());
        }
        if matches!(self.name, "" | "." | "..") || self.name.contains(['/', '\\']) {
            anyhow::bail!(
                "The binary name '{}' can't be used as a directory name; set a configuration path",
                self.name
            );
        }
        // For `ga4gh-cli`, the same file as `Configuration::from_file` in the
        // library reads; keep the two in step.
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("Home directory not found"))?;
        Ok(home.join(format!(".{}", self.name)).join("config.json"))
    }

    /// Reads the section for `service_type` of the
    /// [selected profile](Self::selected_profile) from the configuration file,
    /// as [`Configuration::from_path_with_profile`] does, and applies the token
    /// variable.
    ///
    /// The section name is `service_type.as_str()`, such as `TES`, or the name
    /// of a `ServiceType::Custom` section.
    ///
    /// # Errors
    ///
    /// Fails if the file, profile or section is missing or malformed, or if
    /// the profile or token variable is set but not valid Unicode.
    pub fn load_config(&self, service_type: ServiceType) -> anyhow::Result<Configuration> {
        let path = self.config_path()?;
        let profile = self.selected_profile()?;
        let section = service_type.as_str();
        let mut config = Configuration::from_path_with_profile(&path, profile.as_deref(), section)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        if let Some(name) = &self.token_env {
            if self.token_sections.iter().any(|listed| listed == section)
                && config.basic_auth.is_none()
                && config.oauth_access_token.is_none()
            {
                match std::env::var(name) {
                    Ok(token) if !token.is_empty() => config.oauth_access_token = Some(token),
                    Ok(_) | Err(std::env::VarError::NotPresent) => {}
                    Err(std::env::VarError::NotUnicode(_)) => {
                        anyhow::bail!("Environment variable '{name}' is not valid Unicode")
                    }
                }
            }
        }
        Ok(config)
    }

    /// Builds the root command with the built-in subcommands and the global
    /// option `--profile`. Downstream CLIs can append their own subcommands or
    /// change built-in ones.
    pub fn command(&self) -> Command {
        let mut profile_help =
            "Profile of the configuration file to use, in place of its default_profile".to_owned();
        if let Some(name) = &self.profile_env {
            profile_help.push_str(&format!(" [env: {name}]"));
        }
        Command::new(self.name)
            .bin_name(self.name)
            .version(self.version)
            .about(self.about)
            .subcommand_required(true)
            .arg_required_else_help(true)
            .arg(
                Arg::new(PROFILE_ARG)
                    .long(PROFILE_ARG)
                    .value_name("NAME")
                    .global(true)
                    .help(profile_help),
            )
            .subcommand(drs::command())
            .subcommand(tes::command())
            .subcommand(trs::command())
            .subcommand(wes::command())
            .subcommand(workflow::command())
            .subcommand(registry::command())
    }

    /// This context with the profile of `--profile`, if `matches` has one;
    /// otherwise an unchanged copy.
    ///
    /// [`dispatch`](Self::dispatch) and [`dispatch_to`](Self::dispatch_to)
    /// call it themselves. A downstream binary calls it before
    /// [`load_config`](Self::load_config) for its own commands, so that
    /// `--profile` applies to them too.
    pub fn with_profile_from(&self, matches: &ArgMatches) -> Self {
        // `try_get_one`, because a downstream command may lack the option.
        match matches.try_get_one::<String>(PROFILE_ARG) {
            Ok(Some(profile)) => self.clone().with_profile(profile.clone()),
            _ => self.clone(),
        }
    }

    /// Runs a built-in command with its service's section of this context's
    /// configuration file, in the profile of `--profile` if it is given; see
    /// [`crate::dispatch`].
    pub async fn dispatch(&self, matches: &ArgMatches) -> anyhow::Result<Option<CommandOutput>> {
        let context = self.with_profile_from(matches);
        if let Some(("workflow", sub)) = matches.subcommand() {
            return workflow::run_with_context(sub, None, &context)
                .await
                .map(Some);
        }
        let Some(service_type) = built_in_service(matches) else {
            return Ok(None);
        };
        let config = context.load_config(service_type)?;
        dispatch_with_config(matches, &config).await
    }

    /// Runs a built-in command with its service's section of this context's
    /// configuration file, in the profile of `--profile` if it is given,
    /// writing to caller-owned sinks; see [`crate::dispatch_to`].
    pub async fn dispatch_to(
        &self,
        matches: &ArgMatches,
        stdout: &mut dyn std::io::Write,
        stderr: &mut dyn std::io::Write,
    ) -> anyhow::Result<Option<ExitCode>> {
        let context = self.with_profile_from(matches);
        if let Some(("workflow", sub)) = matches.subcommand() {
            let output = workflow::run_with_context(sub, None, &context).await?;
            return Ok(Some(output.write_to(stdout, stderr)?));
        }
        let Some(service_type) = built_in_service(matches) else {
            return Ok(None);
        };
        let config = context.load_config(service_type)?;
        dispatch_with_config_to(matches, &config, stdout, stderr).await
    }
}

/// The id and long name of the global option that selects a profile.
const PROFILE_ARG: &str = "profile";

/// The service whose configuration section a built-in command reads, or
/// `None` for `workflow` and commands that this crate doesn't own.
fn built_in_service(matches: &ArgMatches) -> Option<ServiceType> {
    match matches.subcommand() {
        Some(("drs", _)) => Some(ServiceType::DRS),
        Some(("tes", _)) => Some(ServiceType::TES),
        Some(("trs", _)) => Some(ServiceType::TRS),
        Some(("wes", _)) => Some(ServiceType::WES),
        Some(("registry", _)) => Some(ServiceType::REGISTRY),
        _ => None,
    }
}

/// Build the root command of `ga4gh-cli`. Downstream CLIs can append their own
/// subcommands, or use [`CliContext::command`] for their own name and version.
pub fn command() -> Command {
    CliContext::default().command()
}

/// Run a built-in command using its service section in the config file.
///
/// `None` leaves an added command to its caller without reading the config file.
/// Config and input errors are returned as `Err`. Service request failures return
/// a `CommandOutput` with an error message and a failure exit code. Reads
/// `~/.ga4gh-cli/config.json`; [`CliContext::dispatch`] reads another file.
pub async fn dispatch(matches: &ArgMatches) -> anyhow::Result<Option<CommandOutput>> {
    CliContext::default().dispatch(matches).await
}

/// Run a built-in command with a caller-supplied configuration for its service.
///
/// Returns `None` for commands not owned by this crate. The caller can inspect
/// or reformat both output streams without changing process-global state.
pub async fn dispatch_with_config(
    matches: &ArgMatches,
    config: &Configuration,
) -> anyhow::Result<Option<CommandOutput>> {
    match matches.subcommand() {
        Some(("drs", sub)) => drs::run(sub, config).await.map(Some),
        Some(("workflow", sub)) => workflow::run(sub, Some(config)).await.map(Some),
        Some(("tes", sub)) => tes::run(sub, config).await.map(Some),
        Some(("trs", sub)) => trs::run(sub, config).await.map(Some),
        Some(("wes", sub)) => wes::run(sub, config).await.map(Some),
        Some(("registry", sub)) => registry::run(sub, config).await.map(Some),
        _ => Ok(None),
    }
}

/// Built-in streaming dispatcher. Writes incrementally to caller-owned sinks.
/// Added downstream commands return None before reading configuration.
/// Reads `~/.ga4gh-cli/config.json`; [`CliContext::dispatch_to`] reads
/// another file.
pub async fn dispatch_to(
    matches: &ArgMatches,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> anyhow::Result<Option<ExitCode>> {
    CliContext::default()
        .dispatch_to(matches, stdout, stderr)
        .await
}

/// Streaming counterpart of dispatch_with_config, including bounded JSONL export.
pub async fn dispatch_with_config_to(
    matches: &ArgMatches,
    config: &Configuration,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> anyhow::Result<Option<ExitCode>> {
    if let Some((service @ ("tes" | "wes"), sub)) = matches.subcommand() {
        if let Some(("list", list)) = sub.subcommand() {
            if list.get_flag("all") {
                let exit_code = if service == "tes" {
                    tes::export_all(list, config, stdout, stderr).await
                } else {
                    wes::export_all(list, config, stdout, stderr).await
                };
                return exit_code.map(Some);
            }
        }
    }
    match dispatch_with_config(matches, config).await? {
        Some(output) => Ok(Some(output.write_to(stdout, stderr)?)),
        None => Ok(None),
    }
}
