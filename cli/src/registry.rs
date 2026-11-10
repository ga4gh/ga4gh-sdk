use crate::CommandOutput;
use clap::{arg, ArgMatches, Command};
use ga4gh_sdk::clients::registry::models::{ExternalService, ServiceType};
use ga4gh_sdk::clients::registry::ServiceRegistry;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::HttpError;
use std::error::Error;

/// Build the Service Registry command and its subcommands.
pub fn command() -> Command {
    Command::new("registry")
        .about("Service Registry subcommands")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("list")
                .about("List the services in the registry")
                .arg(arg!(-t --type <ARTIFACT> "Only list services of this type artifact, such as tes")),
        )
        .subcommand(
            Command::new("get")
                .about("Get a service by its ID")
                .arg(arg!(<id> "The ID of the service"))
                .arg_required_else_help(true),
        )
        .subcommand(Command::new("types").about("List the types of the services in the registry"))
}

/// Execute a parsed Service Registry subcommand using the supplied configuration.
///
/// Output is returned to the caller for writing or reformatting. Request and
/// response failures return a failure `CommandOutput`.
pub async fn run(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    let registry = ServiceRegistry::new(config);
    let output = match sub.subcommand() {
        Some(("list", sub)) => {
            let artifact = sub.get_one::<String>("type");
            match registry.list_services().await {
                Ok(services) => {
                    let services: Vec<_> = services
                        .iter()
                        .filter(|service| {
                            artifact.is_none_or(|artifact| {
                                service.r#type.artifact.eq_ignore_ascii_case(artifact)
                            })
                        })
                        .collect();
                    CommandOutput::success(format_services(&services))
                }
                Err(error) => service_failure(error.as_ref()),
            }
        }
        Some(("get", sub)) => {
            let id = sub
                .get_one::<String>("id")
                .ok_or_else(|| anyhow::anyhow!("id argument is required"))?;
            match registry.get_service(id).await {
                Ok(service) => {
                    CommandOutput::success(format!("{}\n", serde_json::to_string_pretty(&service)?))
                }
                Err(error) => CommandOutput::failure(error),
            }
        }
        Some(("types", _)) => match registry.list_service_types().await {
            Ok(types) => CommandOutput::success(
                types
                    .iter()
                    .map(|service_type| format!("{}\n", type_label(service_type)))
                    .collect(),
            ),
            Err(error) => service_failure(error.as_ref()),
        },
        _ => CommandOutput::success(String::new()),
    };
    Ok(output)
}

fn service_failure(error: &(dyn Error + 'static)) -> CommandOutput {
    if error
        .downcast_ref::<HttpError>()
        .is_some_and(|http| http.status == 404)
    {
        CommandOutput::failure(format!(
            "{error}. Check that base_path is the Service Registry API root"
        ))
    } else {
        CommandOutput::failure(error)
    }
}

fn type_label(service_type: &ServiceType) -> String {
    format!(
        "{}:{}:{}",
        service_type.group, service_type.artifact, service_type.version
    )
}

fn format_services(services: &[&ExternalService]) -> String {
    let rows: Vec<[String; 3]> = services
        .iter()
        .map(|service| {
            [
                service.id.clone(),
                type_label(&service.r#type),
                service.url.clone(),
            ]
        })
        .collect();
    let header = ["ID", "TYPE", "URL"].map(String::from);
    let id_width = rows.iter().chain([&header]).map(|row| row[0].len()).max();
    let type_width = rows.iter().chain([&header]).map(|row| row[1].len()).max();
    let (id_width, type_width) = (id_width.unwrap_or(0), type_width.unwrap_or(0));
    [header]
        .iter()
        .chain(&rows)
        .map(|[id, service_type, url]| {
            format!("{id:<id_width$}  {service_type:<type_width$}  {url}\n")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::format_services;
    use ga4gh_sdk::clients::registry::models::{ExternalService, ServiceOrganization, ServiceType};

    fn service(id: &str, artifact: &str) -> ExternalService {
        ExternalService::new(
            id.to_string(),
            "Name".to_string(),
            ServiceType::new("org.ga4gh".into(), artifact.into(), "1.1.0".into()),
            ServiceOrganization::new("Example".into(), "https://example.org".into()),
            "1.0.0".to_string(),
            format!("https://example.org/{artifact}"),
        )
    }

    #[test]
    fn services_are_aligned_in_columns() {
        let short = service("a", "tes");
        let long = service("org.example.wes", "wes");
        assert_eq!(
            format_services(&[&short, &long]),
            "ID               TYPE                 URL\n\
             a                org.ga4gh:tes:1.1.0  https://example.org/tes\n\
             org.example.wes  org.ga4gh:wes:1.1.0  https://example.org/wes\n"
        );
        assert_eq!(format_services(&[]), "ID  TYPE  URL\n");
    }
}
