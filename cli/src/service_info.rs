//! The `service-info` command: reads the service info of one configured
//! service.

use crate::render::{json_printable, printable};
use crate::CommandOutput;
use clap::{Arg, ArgMatches, Command};
use ga4gh_sdk::clients::serviceinfo::ServiceInfo;
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::HttpError;

/// The id of the option `--service`.
const SERVICE_ARG: &str = "service";

/// Builds the `service-info` command.
pub fn command() -> Command {
    Command::new("service-info")
        .about("Get the service info of a configured service as JSON")
        .long_about(
            "Get the service info of a configured service as JSON.\n\n\
             Requests service-info below the base_path of the section named by \
             --service, in the selected profile of the configuration file, with \
             that section's credentials and discovery timeout.",
        )
        .arg(
            Arg::new(SERVICE_ARG)
                .long("service")
                .value_name("SECTION")
                .required(true)
                .value_parser(clap::builder::NonEmptyStringValueParser::new())
                .help(
                    "Configuration section of the service, such as TES, WES, DRS, TRS, \
                     REGISTRY or a custom section name (case sensitive)",
                ),
        )
}

/// The configuration section that `matches` of the `service-info` command
/// names: a built-in variant for a built-in section name, otherwise
/// [`ServiceType::Custom`].
pub(crate) fn section(matches: &ArgMatches) -> Option<ServiceType> {
    let name = matches.get_one::<String>(SERVICE_ARG)?;
    Some(match name.as_str() {
        "TES" => ServiceType::TES,
        "DRS" => ServiceType::DRS,
        "TRS" => ServiceType::TRS,
        "AAI" => ServiceType::AAI,
        "WES" => ServiceType::WES,
        "REGISTRY" => ServiceType::REGISTRY,
        _ => ServiceType::Custom(name.clone()),
    })
}

/// Runs a parsed `service-info` command with the supplied configuration.
///
/// The configuration is used as given; `--service` then only names the
/// section in error messages. On success, `stdout` holds the service info as
/// one pretty-printed JSON document, including fields that the specification
/// doesn't define. A failed request or invalid service info returns a failure
/// `CommandOutput`.
///
/// # Errors
///
/// Fails if `--service` is missing from `sub`.
pub async fn run(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    let section = section(sub).ok_or_else(|| anyhow::anyhow!("--service is required"))?;
    let section = printable(section.as_str());
    let client = match ServiceInfo::new(config) {
        Ok(client) => client,
        Err(error) => return Ok(CommandOutput::failure(error)),
    };
    let output = match client.get().await {
        Ok(service) => {
            let json = serde_json::to_string_pretty(&service)?;
            CommandOutput::success(format!("{}\n", json_printable(&json)))
        }
        Err(error) => failure(&error, &section),
    };
    Ok(output)
}

/// A failure message for `error` of the service-info request of `section`,
/// with a hint to check `base_path` for a 404.
fn failure(error: &ga4gh_sdk::Error, section: &str) -> CommandOutput {
    if let ga4gh_sdk::Error::Parse(_) = error {
        return CommandOutput::failure(format!(
            "Invalid service info from section '{section}': {error}"
        ));
    }
    let mut message = format!("Error requesting service-info of section '{section}': {error}");
    if error
        .downcast_ref::<HttpError>()
        .is_some_and(|http| http.status == 404)
    {
        message.push_str(&format!(
            ". Check that base_path is the API root of section '{section}'"
        ));
    }
    CommandOutput::failure(message)
}
