//! Discover commands from installed manifests; load only the artifact used to parse a command.
mod catalog;
use crate::{printable, CliContext, CommandOutput};
use catalog::{Binding, Catalog};
use clap::{
    error::{ContextKind, ContextValue, ErrorKind},
    ArgMatches, Command,
};
use ga4gh_cli_protocol::{Attachment, Output, Reply, SubmissionOutcome, CAPABILITY, MAX_MESSAGE};
use ga4gh_sdk::{
    clients::{
        wes::{models::RunRequest, WES},
        ServiceType,
    },
    utils::{
        acknowledgement::AcknowledgementError, configuration::Configuration,
        extension::ExtensionInstance, extension_manager::ExtensionManager,
        operation::AmbiguousSubmissionError,
    },
};
use serde::{de::DeserializeOwned, Serialize};
use std::{ffi::OsString, io::Write, process::ExitCode};

/// Matches and the exact provider selected when parsing them. No native code runs during parsing.
pub struct ParsedCli {
    matches: ArgMatches,
    context: CliContext,
    provider: Option<(ExtensionManager, Binding)>,
}

impl CliContext {
    /// Parse the final (possibly downstream-extended) root tree with installed commands.
    /// Use `clap::Error::exit` in a binary to preserve normal help and usage exit codes.
    pub fn parse_from(
        &self,
        root: Command,
        args: impl IntoIterator<Item = impl Into<OsString>>,
    ) -> Result<ParsedCli, clap::Error> {
        let args: Vec<_> = args.into_iter().map(Into::into).collect();
        let (matches, provider) = match Catalog::discover(self, &root) {
            Ok(Catalog {
                command,
                manager,
                mut bindings,
                hint,
            }) => {
                let matches = command.try_get_matches_from(args).map_err(|mut error| {
                    if error.kind() == ErrorKind::InvalidSubcommand {
                        error.insert(
                            ContextKind::Suggested,
                            ContextValue::StyledStrs(vec![hint.into()]),
                        );
                    }
                    error
                })?;
                let binding = matches
                    .subcommand_name()
                    .and_then(|name| bindings.remove(name));
                (matches, binding.map(|binding| (manager, binding)))
            }
            Err(error) => match root.try_get_matches_from(args) {
                Ok(matches) => (matches, None),
                Err(parse) if parse.kind() != ErrorKind::InvalidSubcommand => return Err(parse),
                Err(_) => {
                    return Err(clap::Error::raw(
                        ErrorKind::InvalidValue,
                        format!("{error}\n"),
                    ))
                }
            },
        };
        Ok(ParsedCli {
            context: self.with_profile_from(&matches),
            matches,
            provider,
        })
    }

    pub(crate) fn extension_manager(&self) -> anyhow::Result<ExtensionManager> {
        Ok(ExtensionManager::open(
            self.config_path()?.with_file_name("extensions.json"),
        )?)
    }
}

impl ParsedCli {
    /// Inspect matches for a command owned by the downstream binary.
    pub fn matches(&self) -> &ArgMatches {
        &self.matches
    }

    /// Dispatch an installed command only. Built-ins and downstream commands return `None`.
    /// A supplied WES configuration takes precedence over this invocation's context.
    /// WES submission failures retain their SDK error, even if the provider returns success.
    /// Enabling a provider authorizes native execution; only install trusted libraries.
    pub async fn dispatch_extension(
        &self,
        config: Option<&Configuration>,
    ) -> anyhow::Result<Option<CommandOutput>> {
        self.execute(config.map_or(WesSource::Context, WesSource::Configuration))
            .await
    }

    /// Dispatch with the caller's WES client, retaining its transport, limits and credentials.
    pub async fn dispatch_extension_with_wes(
        &self,
        wes: &WES,
    ) -> anyhow::Result<Option<CommandOutput>> {
        self.execute(WesSource::Client(wes)).await
    }

    async fn execute(&self, source: WesSource<'_>) -> anyhow::Result<Option<CommandOutput>> {
        let Some((manager, binding)) = &self.provider else {
            return Ok(None);
        };
        let profile = self.context.selected_profile()?;
        let context = match &profile {
            Some(name) => self.context.clone().with_profile(name.clone()),
            None => self.context.clone().without_profile_env(),
        };
        let (resolved, invocation, wes_submit) = binding.invocation(&self.matches, profile)?;
        let instance = unsafe { manager.create_instance(resolved, b"{}") }?;
        let result = match exchange(&instance, "execute", &invocation)? {
            Reply::Complete(output) => render(output)?,
            Reply::WesSubmit {
                request,
                attachments,
            } => {
                anyhow::ensure!(wes_submit, "Undeclared WES submission action");
                let run_id = match source.submit(&context, *request, attachments).await {
                    Ok(run_id) => run_id,
                    Err(error) => {
                        let uncertain =
                            error
                                .downcast_ref::<ga4gh_sdk::Error>()
                                .is_some_and(|source| {
                                    source.downcast_ref::<AmbiguousSubmissionError>().is_some()
                                        || source.downcast_ref::<AcknowledgementError>().is_some()
                                });
                        let outcome = SubmissionOutcome::Failed {
                            message: if uncertain {
                                "WES may have accepted the run; reconcile before retrying"
                            } else {
                                "WES submission failed"
                            }
                            .into(),
                        };
                        let _ = exchange::<_, Output>(&instance, "wes_outcome", &outcome);
                        return Err(error);
                    }
                };
                let outcome = SubmissionOutcome::Submitted {
                    run_id: run_id.clone(),
                };
                let warning = || {
                    format!("WES accepted run '{}', but the extension could not report its result; do not resubmit", printable(&run_id))
                };
                let mut output = exchange(&instance, "wes_outcome", &outcome)
                    .and_then(render)
                    .map_err(|_| anyhow::anyhow!(warning()))?;
                if output.exit_code != ExitCode::SUCCESS {
                    if !output.stderr.is_empty() && !output.stderr.ends_with('\n') {
                        output.stderr.push('\n');
                    }
                    output.stderr.push_str(&warning());
                    output.stderr.push('\n');
                }
                output
            }
        };
        Ok(Some(result))
    }

    /// Dispatch an installed command or stream a built-in command to caller-owned sinks.
    pub async fn dispatch_to(
        &self,
        stdout: &mut dyn Write,
        stderr: &mut dyn Write,
    ) -> anyhow::Result<Option<ExitCode>> {
        match self.dispatch_extension(None).await {
            Err(error) => Ok(Some(
                CommandOutput::failure(error).write_to(stdout, stderr)?,
            )),
            Ok(Some(output)) => Ok(Some(output.write_to(stdout, stderr)?)),
            Ok(None) => {
                self.context
                    .dispatch_to(&self.matches, stdout, stderr)
                    .await
            }
        }
    }
}

enum WesSource<'a> {
    Context,
    Configuration(&'a Configuration),
    Client(&'a WES),
}

impl WesSource<'_> {
    async fn submit(
        self,
        context: &CliContext,
        request: RunRequest,
        attachments: Vec<Attachment>,
    ) -> anyhow::Result<String> {
        let attachments = attachments
            .into_iter()
            .map(TryInto::try_into)
            .collect::<Result<Vec<_>, _>>()?;
        let owned;
        let client = match self {
            Self::Client(client) => client,
            Self::Configuration(config) => {
                owned = WES::new(config).await?;
                &owned
            }
            Self::Context => {
                let config = context.load_config_fresh(ServiceType::WES).await?;
                owned = WES::new(&config).await?;
                &owned
            }
        };
        Ok(client
            .run_workflow_with_attachments(request, attachments)
            .await?
            .id)
    }
}

fn render(result: Output) -> anyhow::Result<CommandOutput> {
    let stdout = if result.json_stdout {
        let output = CommandOutput::json(result.stdout);
        anyhow::ensure!(output.stderr.is_empty(), "Invalid JSON from extension");
        output.stdout.replace('\r', " ")
    } else {
        text(&result.stdout)
    };
    Ok(CommandOutput {
        stdout,
        stderr: text(&result.stderr),
        exit_code: ExitCode::from(result.exit_status),
    })
}

fn text(value: &str) -> String {
    match value.strip_suffix('\n') {
        Some(line) => format!("{}\n", printable(line)),
        None => printable(value),
    }
}

fn exchange<I: Serialize, O: DeserializeOwned>(
    instance: &ExtensionInstance,
    operation: &str,
    input: &I,
) -> anyhow::Result<O> {
    let bytes = ga4gh_cli_protocol::encode(input)?;
    let bytes = instance.invoke_bounded(CAPABILITY, operation, &bytes, MAX_MESSAGE)?;
    serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("Invalid CLI extension reply"))
}
