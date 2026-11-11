use crate::render::printable;
use crate::CommandOutput;
use clap::{arg, ArgAction, ArgMatches, Command};
use ga4gh_sdk::clients::pagination::{Pager, PagingOptions};
use ga4gh_sdk::clients::wait::ServiceKind;
use ga4gh_sdk::clients::wes::attachments::{
    validate_filename, AttachmentSource, StreamingAttachment,
};
use ga4gh_sdk::clients::wes::models::{RunListResponse, RunRequest};
use ga4gh_sdk::clients::wes::{urlencode, Run, WES};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::operation::Operation;
use ga4gh_sdk::utils::transport::Transport;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// Time allowed for preparing, uploading and acknowledging one submission.
const SUBMISSION_BUDGET: Duration = Duration::from_secs(300);

/// Build the WES command and its subcommands.
pub fn command() -> Command {
    Command::new("wes")
        .about("WES subcommands")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("run")
                .about("Submit a workflow run")
                .arg(arg!(<WORKFLOW_URL> "The workflow URL, or the path of an attached workflow file"))
                .arg(arg!(--type <TYPE> "The workflow type, such as CWL, WDL or NFL").required(true))
                .arg(arg!(--type_version <VERSION> "The version of the workflow type, such as v1.2").required(true))
                .arg(arg!(--params [PARAMS] "Workflow parameters: JSON file, inline JSON object, or - for stdin"))
                .arg(
                    arg!(-a --attachment [PATH] "A file to upload, at the relative path the run sees")
                        .action(ArgAction::Append)
                        .num_args(1),
                )
                .arg(
                    arg!(--tag [KEY_VALUE] "A run tag as KEY=VALUE")
                        .action(ArgAction::Append)
                        .num_args(1),
                )
                .arg(arg!(-e --engine [ENGINE] "The workflow engine to use"))
                .arg(arg!(--engine_version [VERSION] "The version of the workflow engine"))
                .arg(
                    arg!(--engine_parameter [KEY_VALUE] "A workflow engine parameter as KEY=VALUE")
                        .action(ArgAction::Append)
                        .num_args(1)
                        .allow_hyphen_values(true),
                )
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("list")
                .about("List workflow runs")
                .arg(arg!(-p --page_size [PAGE_SIZE] "The page size for pagination"))
                .arg(arg!(-t --page_token [PAGE_TOKEN] "The page token for pagination"))
                .arg(arg!(--all "Fetch pages lazily and stream JSONL").action(ArgAction::SetTrue))
                .arg(
                    clap::Arg::new("format")
                        .long("format")
                        .value_parser(["table", "jsonl"])
                        .default_value("table"),
                )
                .arg(
                    clap::Arg::new("max-pages")
                        .long("max-pages")
                        .value_parser(clap::value_parser!(usize))
                        .default_value("1000"),
                )
                .arg(
                    clap::Arg::new("max-items")
                        .long("max-items")
                        .value_parser(clap::value_parser!(usize)),
                ),
        )
        .subcommand(
            Command::new("get")
                .about("Get the details and logs of a run")
                .arg(
                    arg!(<id> "The ID of the run")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new()),
                )
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("status")
                .about("Get the state of a run")
                .arg(
                    arg!(<id> "The ID of the run")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new()),
                )
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("cancel")
                .about("Cancel a run")
                .arg(
                    arg!(<id> "The ID of the run")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new()),
                )
                .arg_required_else_help(true),
        )
}

/// Execute a parsed WES subcommand using the supplied configuration.
///
/// Output is returned to the caller for writing or reformatting. Input errors
/// are returned as `Err`; service failures return a failure `CommandOutput`.
pub async fn run(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    match sub.subcommand() {
        Some(("run", sub)) => submit(sub, config).await,
        Some(("list", sub)) => list(sub, config).await,
        Some(("get", sub)) => {
            let id = id_argument(sub)?;
            let wes = match WES::new(config).await {
                Ok(wes) => wes,
                Err(error) => return Ok(CommandOutput::failure(error)),
            };
            // Not Run::log: the server's JSON is printed unchanged, with
            // fields that the SDK models lack.
            let path = format!("runs/{}", urlencode(id));
            Ok(match wes.transport.get(&path, None).await {
                Ok(log) => CommandOutput::json(log),
                Err(error) => CommandOutput::failure(error),
            })
        }
        Some(("status", sub)) => {
            let run = Run::new(id_argument(sub)?.to_string(), Transport::new(config));
            Ok(match run.status().await {
                Ok(state) => CommandOutput::success(format!("{state}\n")),
                Err(error) => CommandOutput::failure(error),
            })
        }
        Some(("cancel", sub)) => {
            let run = Run::new(id_argument(sub)?.to_string(), Transport::new(config));
            Ok(match run.cancel().await {
                Ok(_) => CommandOutput::success(format!("{}\n", printable(&run.id))),
                Err(error) => CommandOutput::failure(error),
            })
        }
        _ => Ok(CommandOutput::success(String::new())),
    }
}

fn id_argument(sub: &ArgMatches) -> anyhow::Result<&String> {
    sub.get_one::<String>("id")
        .ok_or_else(|| anyhow::anyhow!("id argument is required"))
}

async fn submit(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    let required = |name: &str| {
        sub.get_one::<String>(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("{name} argument is required"))
    };
    let mut request = RunRequest::new(
        required("type")?,
        required("type_version")?,
        required("WORKFLOW_URL")?,
    );
    request.workflow_params = Some(match sub.get_one::<String>("params") {
        Some(params) => match read_params(params).await? {
            Ok(params) => params,
            Err(output) => return Ok(output),
        },
        None => serde_json::json!({}),
    });
    request.tags = key_values(sub, "tag")?;
    request.workflow_engine_parameters = key_values(sub, "engine_parameter")?;
    request.workflow_engine = sub.get_one::<String>("engine").cloned();
    request.workflow_engine_version = sub.get_one::<String>("engine_version").cloned();
    let attachments = attachments(sub)?;

    let wes = match WES::new(config).await {
        Ok(wes) => wes,
        Err(error) => return Ok(CommandOutput::failure(error)),
    };
    let operation = Operation::new(SUBMISSION_BUDGET);
    Ok(
        match wes
            .run_workflow_streaming(request, attachments, &operation)
            .await
        {
            Ok(run) => CommandOutput::success(format!("{}\n", printable(&run.id))),
            Err(error) => CommandOutput::failure(error),
        },
    )
}

/// Reads workflow parameters like `tes create` reads tasks: from a file,
/// inline JSON or standard input. A missing file is a failure output.
async fn read_params(params: &str) -> anyhow::Result<Result<serde_json::Value, CommandOutput>> {
    let json = if params == "-" {
        let mut input = String::new();
        tokio::io::stdin().read_to_string(&mut input).await?;
        input
    } else if Path::new(params).exists() {
        std::fs::read_to_string(params)
            .map_err(|error| anyhow::anyhow!("Failed to read file: {error}"))?
    } else if params.trim_start().starts_with('{') {
        params.to_string()
    } else {
        return Ok(Err(CommandOutput::failure(format!(
            "File does not exist: {}",
            printable(params)
        ))));
    };
    let value: serde_json::Value = serde_json::from_str(&json)
        .map_err(|error| anyhow::anyhow!("Failed to parse JSON: {error}"))?;
    if !value.is_object() {
        anyhow::bail!("Workflow parameters must be a JSON object");
    }
    Ok(Ok(value))
}

/// Parses repeated `KEY=VALUE` options; the value may contain further `=`.
fn key_values(sub: &ArgMatches, name: &str) -> anyhow::Result<Option<HashMap<String, String>>> {
    let Some(values) = sub.get_many::<String>(name) else {
        return Ok(None);
    };
    values
        .map(|value| match value.split_once('=') {
            Some((key, value)) if !key.is_empty() => Ok((key.to_string(), value.to_string())),
            _ => anyhow::bail!("Invalid {name} '{}': expected KEY=VALUE", printable(value)),
        })
        .collect::<anyhow::Result<_>>()
        .map(Some)
}

/// Checks attachment paths before any request. Each file is uploaded under
/// the relative path given, so that workflows can refer to it.
fn attachments(sub: &ArgMatches) -> anyhow::Result<Vec<StreamingAttachment>> {
    let mut names = HashSet::new();
    sub.get_many::<String>("attachment")
        .into_iter()
        .flatten()
        .map(|path| {
            let name = printable(path);
            validate_filename(path)
                .map_err(|error| anyhow::anyhow!("Attachment '{name}': {error}"))?;
            if !names.insert(path) {
                anyhow::bail!("Duplicate attachment '{name}'");
            }
            match std::fs::metadata(path) {
                Ok(metadata) if metadata.is_file() => Ok(StreamingAttachment {
                    filename: path.clone(),
                    source: AttachmentSource::File(PathBuf::from(path)),
                }),
                Ok(_) => anyhow::bail!("Cannot read attachment '{name}': not a file"),
                Err(error) => anyhow::bail!("Cannot read attachment '{name}': {error}"),
            }
        })
        .collect()
}

fn page_size(sub: &ArgMatches) -> anyhow::Result<Option<i64>> {
    sub.get_one::<String>("page_size")
        .map(|value| {
            value.parse::<i64>().map_err(|_| {
                anyhow::anyhow!(
                    "Invalid page_size '{}': expected an integer",
                    printable(value)
                )
            })
        })
        .transpose()
}

async fn list(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    if sub.get_flag("all") {
        anyhow::bail!("--all needs the streaming dispatch_to/dispatch_with_config_to API");
    }
    if sub.get_one::<String>("format").map(String::as_str) == Some("jsonl") {
        anyhow::bail!("--format jsonl requires --all");
    }
    let page_size = page_size(sub)?;
    let page_token = sub.get_one::<String>("page_token").cloned();
    let wes = match WES::new(config).await {
        Ok(wes) => wes,
        Err(error) => return Ok(CommandOutput::failure(error)),
    };
    Ok(match wes.list_runs(page_token, page_size).await {
        Ok(response) => CommandOutput::success(format_runs(&response)),
        Err(error) => CommandOutput::failure(error),
    })
}

fn format_runs(response: &RunListResponse) -> String {
    let rows: Vec<[String; 2]> = response
        .runs
        .iter()
        .flatten()
        .map(|run| {
            [
                printable(&run.run_id),
                run.state.unwrap_or_default().to_string(),
            ]
        })
        .collect();
    let header = ["RUN ID", "STATE"].map(String::from);
    let width = rows
        .iter()
        .chain([&header])
        .map(|row| row[0].chars().count())
        .max()
        .unwrap_or(0);
    let mut table: String = [header]
        .iter()
        .chain(&rows)
        .map(|[id, state]| format!("{id:<width$}  {state}\n"))
        .collect();
    if let Some(token) = response
        .next_page_token
        .as_deref()
        .filter(|token| !token.is_empty())
    {
        table.push_str(&format!("Next page token: {}\n", printable(token)));
    }
    table
}

/// Stream runs as JSONL a page at a time. Diagnostics go only to stderr.
pub async fn export_all(
    sub: &ArgMatches,
    config: &Configuration,
    stdout: &mut dyn std::io::Write,
    stderr: &mut dyn std::io::Write,
) -> anyhow::Result<std::process::ExitCode> {
    if sub.get_one::<String>("format").map(String::as_str) != Some("jsonl") {
        anyhow::bail!("--all requires --format jsonl");
    }
    let mut query = serde_json::Map::new();
    if let Some(size) = page_size(sub)? {
        query.insert("page_size".into(), serde_json::json!(size));
    }
    if let Some(token) = sub.get_one::<String>("page_token") {
        query.insert("page_token".into(), serde_json::json!(token));
    }
    let mut options = PagingOptions::new(Duration::from_secs(300));
    options.max_requests = *sub.get_one::<usize>("max-pages").unwrap();
    options.max_items = sub.get_one::<usize>("max-items").copied();
    let wes = match WES::new(config).await {
        Ok(wes) => wes,
        Err(error) => {
            writeln!(stderr, "{}", printable(&error.to_string()))?;
            return Ok(std::process::ExitCode::FAILURE);
        }
    };
    let pager = Pager::new(&wes.transport, ServiceKind::Wes, query.into(), options)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    crate::tes::write_jsonl(pager, stdout, stderr).await
}

#[cfg(test)]
mod tests {
    use super::format_runs;
    use ga4gh_sdk::clients::wes::models::{RunListResponse, RunListResponseRunsInner, State};

    #[test]
    fn runs_are_aligned_and_missing_states_are_unknown() {
        let mut running = RunListResponseRunsInner::new("a".into());
        running.state = Some(State::Running);
        let response = RunListResponse {
            runs: Some(vec![
                running,
                RunListResponseRunsInner::new("long-run-id".into()),
            ]),
            next_page_token: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            format_runs(&response),
            "RUN ID       STATE\n\
             a            RUNNING\n\
             long-run-id  UNKNOWN\n"
        );
        assert_eq!(format_runs(&RunListResponse::default()), "RUN ID  STATE\n");
    }
}
