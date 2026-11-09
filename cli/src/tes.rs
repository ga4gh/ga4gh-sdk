use crate::CommandOutput;
use clap::{arg, ArgMatches, Command};
use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::models::TesListTasksResponse;
use ga4gh_sdk::clients::tes::models::TesState;
use ga4gh_sdk::clients::tes::models::TesTask;
use ga4gh_sdk::clients::tes::{Task, TES};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::{HttpError, Transport};
use log::debug;
use std::path::Path;

/// Build the TES command and its subcommands.
pub fn command() -> Command {
    Command::new("tes")
        .about("TES subcommands")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("create")
                .about("Create a task")
                .arg(arg!(<TASK_FILE> "The task file to create"))
                // .arg(arg!(--url <URL> "The URL for the task"))
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("list")
                .about("list all tasks")
                .arg(arg!(-n --name_prefix [NAME_PREFIX] "The name prefix to filter tasks"))
                .arg(arg!(-s --state [STATE] "The state to filter tasks"))
                .arg(arg!(-k --tag_key [TAG_KEY] "The tag key to filter tasks"))
                .arg(arg!(-v --tag_value [TAG_VALUE] "The tag value to filter tasks"))
                .arg(arg!(-p --page_size [PAGE_SIZE] "The page size for pagination"))
                .arg(arg!(-t --page_token [PAGE_TOKEN] "The page token for pagination"))
                .arg(arg!(-w --view [VIEW] "The view for the tasks")),
        )
        .subcommand(
            Command::new("get")
                .about("get task data")
                .arg(arg!(<id> "The id of the task which should be returned"))
                .arg(arg!(<view> "The view in which the task should be returned"))
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("status")
                .about("get status of the task")
                .arg(arg!(<id> "The id of the task which should be returned"))
                .arg_required_else_help(true),
        )
        .subcommand(
            Command::new("cancel")
                .about("cancel the task")
                .arg(arg!(<id> "The id of the task which should be cancel"))
                .arg_required_else_help(true),
        )
}

/// Execute a parsed TES subcommand using the supplied configuration.
///
/// Output is returned to the caller for writing or reformatting. Input errors
/// are returned as `Err`; service failures return a failure `CommandOutput`.
pub async fn run(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    if let Some(("create", sub)) = sub.subcommand() {
        let task_file = sub
            .get_one::<String>("TASK_FILE")
            .ok_or_else(|| anyhow::anyhow!("TASK_FILE argument is required"))?;
        let path = Path::new(task_file);
        if !path.exists() {
            return Ok(CommandOutput::failure(format!(
                "File does not exist: {}",
                path.display()
            )));
        }
        let task_json = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Failed to read file: {e}"))?;
        let testask: TesTask = serde_json::from_str(&task_json)
            .map_err(|e| anyhow::anyhow!("Failed to parse JSON: {e}"))?;

        let tes = match TES::new(config).await {
            Ok(tes) => tes,
            Err(error) => return Ok(service_failure(error.as_ref())),
        };
        match tes.create(testask).await {
            Ok(task) => return Ok(CommandOutput::success(format!("{}\n", task.id))),
            Err(error) => return Ok(CommandOutput::failure(error)),
        }
    }

    if let Some(("list", sub)) = sub.subcommand() {
        let name_prefix = sub.get_one::<String>("name_prefix").cloned();
        let state = sub
            .get_one::<String>("state")
            .map(|s| parse_state(s))
            .transpose()?;
        let _tag_key = sub.get_one::<String>("tag_key").cloned();
        let _tag_value = sub.get_one::<String>("tag_value").cloned();
        let page_size = sub
            .get_one::<String>("page_size")
            .map(|value| {
                value.parse::<i32>().map_err(|_| {
                    anyhow::anyhow!("Invalid page_size '{value}': expected an integer")
                })
            })
            .transpose()?;
        let page_token = sub.get_one::<String>("page_token").cloned();
        let view = sub.get_one::<String>("view").cloned();

        let parameters = ListTasksParams {
            name_prefix,
            state,
            tag_key: None,   // Example does not cover parsing Vec<String>
            tag_value: None, // Example does not cover parsing Vec<String>
            page_size,
            page_token,
            view,
        };
        debug!("parameters are: {:?}", parameters);

        let tes = match TES::new(config).await {
            Ok(tes) => tes,
            Err(error) => return Ok(service_failure(error.as_ref())),
        };
        match tes.list_tasks(Some(parameters)).await {
            Ok(task_response) => {
                return Ok(CommandOutput::success(format!(
                    "{}\n",
                    format_tasks_response(&task_response)
                )));
            }
            Err(error) => return Ok(CommandOutput::failure(error)),
        }
    }

    if let Some(("get", sub)) = sub.subcommand() {
        let id = sub
            .get_one::<String>("id")
            .ok_or_else(|| anyhow::anyhow!("id argument is required"))?;
        let view = sub
            .get_one::<String>("view")
            .ok_or_else(|| anyhow::anyhow!("view argument is required"))?;

        let tes = match TES::new(config).await {
            Ok(tes) => tes,
            Err(error) => return Ok(service_failure(error.as_ref())),
        };
        match tes.get_raw(view, id).await {
            Ok(task) => return Ok(CommandOutput::success(task)),
            Err(error) => return Ok(CommandOutput::failure(error)),
        }
    }

    if let Some(("status", sub)) = sub.subcommand() {
        let id = sub
            .get_one::<String>("id")
            .ok_or_else(|| anyhow::anyhow!("id argument is required"))?
            .to_string();
        let transport = Transport::new(config);
        let task = Task::new(id.clone(), transport);
        match task.status().await {
            Ok(status) => return Ok(CommandOutput::success(format!("{status}\n"))),
            Err(error) => return Ok(CommandOutput::failure(error)),
        }
    }

    if let Some(("cancel", sub)) = sub.subcommand() {
        let id = sub
            .get_one::<String>("id")
            .ok_or_else(|| anyhow::anyhow!("id argument is required"))?
            .to_string();
        let transport = Transport::new(config);
        let task = Task::new(id, transport);
        match task.cancel().await {
            Ok(output) => {
                return Ok(CommandOutput::success(format!("STATUS: {output:?}\n")));
            }
            Err(error) => return Ok(CommandOutput::failure(error)),
        }
    }
    Ok(CommandOutput::success(String::new()))
}

fn service_failure(error: &(dyn std::error::Error + 'static)) -> CommandOutput {
    let message = if error
        .downcast_ref::<HttpError>()
        .is_some_and(|http| http.status == 404)
    {
        format!("{error}. Check that base_path is the TES API root")
    } else {
        error.to_string()
    };
    CommandOutput::failure(message)
}

fn parse_state(value: &str) -> anyhow::Result<TesState> {
    let trimmed = value.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|state| state.strip_suffix('"'))
        .unwrap_or(trimmed);
    let normalized = unquoted.to_ascii_uppercase().replace(' ', "_");
    serde_json::from_value(serde_json::Value::String(normalized)).map_err(|_| {
        anyhow::anyhow!(
            "Invalid state '{value}': expected UNKNOWN, QUEUED, INITIALIZING, RUNNING, PAUSED, COMPLETE, EXECUTOR_ERROR, SYSTEM_ERROR, CANCELED, CANCELING, or PREEMPTED"
        )
    })
}

// Helper functions

fn tes_state_to_str(state: &Option<TesState>) -> &str {
    match state {
        Some(TesState::Unknown) => "Unknown",
        Some(TesState::Queued) => "Queued",
        Some(TesState::Initializing) => "Initializing",
        Some(TesState::Running) => "Running",
        Some(TesState::Paused) => "Paused",
        Some(TesState::Complete) => "Complete",
        Some(TesState::ExecutorError) => "Executor Error",
        Some(TesState::SystemError) => "System Error",
        Some(TesState::Canceled) => "Canceled",
        Some(TesState::Canceling) => "Canceling",
        Some(TesState::Preempted) => "Preempted",
        None => "None",
    }
}

fn format_task(task: &TesTask) -> String {
    format!(
        "{:<25} {:<15}\n",
        task.id.as_deref().unwrap_or("None"),
        tes_state_to_str(&task.state)
    )
}

fn format_tasks_response(response: &TesListTasksResponse) -> String {
    let mut table = String::new();
    let headers = format!("{:<25} {:<15}\n", "TASK ID", "State");
    table.push_str(&headers);
    for task in &response.tasks {
        table.push_str(&format_task(task));
    }
    table
}

#[cfg(test)]
mod tests {
    use super::{parse_state, tes_state_to_str};
    use ga4gh_sdk::clients::tes::models::TesState;

    #[test]
    fn displayed_error_states_can_be_used_as_list_filters() {
        for state in [TesState::ExecutorError, TesState::SystemError] {
            assert_eq!(parse_state(tes_state_to_str(&Some(state))).unwrap(), state);
        }
        assert!(parse_state("invalid")
            .unwrap_err()
            .to_string()
            .contains("PREEMPTED"));
    }
}
