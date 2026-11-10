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
                .arg(
                    arg!(<id> "The ID of the service")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new()),
                )
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
                    let json = serde_json::to_string_pretty(&service)?;
                    CommandOutput::success(format!("{}\n", json_printable(&json)))
                }
                // A 404 means an unknown ID here, so no base_path hint.
                Err(error) => CommandOutput::failure(printable(&error.to_string())),
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

/// Error messages can hold the server's response body, so they are escaped too.
fn service_failure(error: &(dyn Error + 'static)) -> CommandOutput {
    let message = printable(&error.to_string());
    if error
        .downcast_ref::<HttpError>()
        .is_some_and(|http| http.status == 404)
    {
        CommandOutput::failure(format!(
            "{message}. Check that base_path is the Service Registry API root"
        ))
    } else {
        CommandOutput::failure(message)
    }
}

/// Escapes control and bidirectional formatting characters in a value from
/// the registry, so that it can't add lines, terminal sequences or reordered
/// text to the output.
pub(crate) fn printable(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if needs_escape(c) {
                c.escape_default().collect()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// Escapes the same characters as [`printable`] in serialised JSON, as JSON
/// `\uXXXX` escapes, so that the output stays valid JSON. Serde escapes C0
/// controls in strings already; the remaining ones are all in the Basic
/// Multilingual Plane and can only occur inside strings.
fn json_printable(json: &str) -> String {
    json.chars()
        .map(|c| {
            // Raw newlines are the layout of pretty JSON; serde escapes the rest.
            if c != '\n' && needs_escape(c) {
                format!("\\u{:04x}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn needs_escape(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn type_label(service_type: &ServiceType) -> String {
    printable(&format!(
        "{}:{}:{}",
        service_type.group, service_type.artifact, service_type.version
    ))
}

fn format_services(services: &[&ExternalService]) -> String {
    let rows: Vec<[String; 3]> = services
        .iter()
        .map(|service| {
            [
                printable(&service.id),
                type_label(&service.r#type),
                printable(&service.url),
            ]
        })
        .collect();
    let header = ["ID", "TYPE", "URL"].map(String::from);
    let width = |column: usize| {
        rows.iter()
            .chain([&header])
            .map(|row| row[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let (id_width, type_width) = (width(0), width(1));
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

    #[test]
    fn non_ascii_values_are_aligned_by_characters() {
        let wide = service("org.exämple", "tes");
        let short = service("a", "wes");
        assert_eq!(
            format_services(&[&wide, &short]),
            "ID           TYPE                 URL\n\
             org.exämple  org.ga4gh:tes:1.1.0  https://example.org/tes\n\
             a            org.ga4gh:wes:1.1.0  https://example.org/wes\n"
        );
    }

    #[test]
    fn control_characters_are_escaped() {
        let mut forged = service("a\nfake  org.ga4gh:tes:1.1.0  https://evil.example", "tes");
        forged.url = "\u{1b}[2Khttps://example.org/tes\r".to_string();
        forged.r#type.version = "1\n".to_string();
        forged.r#type.group = "org.\u{202E}ga4gh".to_string();
        let output = format_services(&[&forged]);
        assert_eq!(output.lines().count(), 2, "{output}");
        assert!(
            !output.contains('\u{1b}') && !output.contains('\r'),
            "{output}"
        );
        assert!(output.contains(r"a\nfake"), "{output}");
        assert!(output.contains(r"\u{1b}[2K"), "{output}");
        assert!(output.contains(r"org.\u{202e}ga4gh:tes:1\n"), "{output}");
    }
}
