use crate::CommandOutput;
use clap::{arg, ArgMatches, Command};
use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain, FileWrapper};
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::HttpError;
use std::error::Error;

/// Build the TRS commands for browsing tools and fetching their descriptors.
pub fn command() -> Command {
    Command::new("trs")
        .about("Tool Registry Service commands")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(
            Command::new("tools")
                .about("List tools")
                .arg(arg!(--"tool-class" <TOOL_CLASS> "Tool class, e.g. Workflow or CommandLineTool"))
                .arg(arg!(--"descriptor-type" <TYPE> "Descriptor type: CWL, WDL, NFL, GALAXY or SMK"))
                .arg(arg!(--organization <ORGANIZATION> "Organization that published the tool"))
                .arg(arg!(--name <NAME> "Name of the tool's image"))
                .arg(arg!(--offset <OFFSET> "Start of the page, as returned by the registry"))
                .arg(arg!(--limit <LIMIT> "Number of tools per page")),
        )
        .subcommand(
            Command::new("tool")
                .about("Get a tool, including its versions")
                .arg(arg!(<TOOL_ID> "Tool ID")),
        )
        .subcommand(
            Command::new("versions")
                .about("List the versions of a tool")
                .arg(arg!(<TOOL_ID> "Tool ID")),
        )
        .subcommand(
            Command::new("version")
                .about("Get one version of a tool")
                .arg(arg!(<TOOL_ID> "Tool ID"))
                .arg(arg!(<VERSION_ID> "Version ID")),
        )
        .subcommand(
            Command::new("descriptor")
                .about("Get the descriptor of a tool version")
                .arg(arg!(<TOOL_ID> "Tool ID"))
                .arg(arg!(<VERSION_ID> "Version ID"))
                .arg(type_arg())
                .arg(arg!(--plain "Print the descriptor file itself instead of JSON"))
                .arg(arg!(--path <RELATIVE_PATH> "Path of a file relative to the primary descriptor")),
        )
        .subcommand(
            Command::new("files")
                .about("List the files of a tool version")
                .arg(arg!(<TOOL_ID> "Tool ID"))
                .arg(arg!(<VERSION_ID> "Version ID"))
                .arg(type_arg()),
        )
        .subcommand(
            Command::new("tests")
                .about("Get the test parameter files of a tool version")
                .arg(arg!(<TOOL_ID> "Tool ID"))
                .arg(arg!(<VERSION_ID> "Version ID"))
                .arg(type_arg())
                .arg(arg!(--plain "Print the test files themselves instead of JSON")),
        )
}

fn type_arg() -> clap::Arg {
    arg!(--type <TYPE> "Descriptor type: CWL, WDL, NFL, GALAXY or SMK").required(true)
}

/// A TRS request, checked before any request is sent.
enum Request {
    Tools(ToolFilter),
    Tool(String),
    Versions(String),
    Version {
        tool: String,
        version: String,
    },
    Descriptor {
        tool: String,
        version: String,
        descriptor_type: DescriptorTypeWithPlain,
        plain: bool,
        path: Option<String>,
    },
    Files {
        tool: String,
        version: String,
        descriptor_type: DescriptorType,
    },
    Tests {
        tool: String,
        version: String,
        descriptor_type: DescriptorTypeWithPlain,
        plain: bool,
    },
}

/// Run a parsed TRS command using the supplied service configuration.
pub async fn run(sub: &ArgMatches, config: &Configuration) -> anyhow::Result<CommandOutput> {
    let request = parse(sub)?;
    let trs = match TRS::new(config).await {
        Ok(trs) => trs,
        Err(error) => return Ok(service_failure(error.as_ref())),
    };
    Ok(match execute(&trs, request).await {
        Ok(stdout) => CommandOutput::success(stdout),
        Err(error) => CommandOutput::failure(error),
    })
}

fn parse(sub: &ArgMatches) -> anyhow::Result<Request> {
    let (name, args) = sub
        .subcommand()
        .ok_or_else(|| anyhow::anyhow!("TRS subcommand is required"))?;
    let string = |id: &str| args.get_one::<String>(id).cloned();
    let required =
        |id: &str| string(id).ok_or_else(|| anyhow::anyhow!("{id} argument is required"));

    Ok(match name {
        "tools" => Request::Tools(ToolFilter {
            tool_class: string("tool-class"),
            descriptor_type: string("descriptor-type")
                .map(|value| parse_descriptor_type(&value))
                .transpose()?,
            organization: string("organization"),
            name: string("name"),
            offset: string("offset"),
            limit: string("limit")
                .map(|value| {
                    value.parse::<i32>().map_err(|_| {
                        anyhow::anyhow!("Invalid limit '{value}': expected an integer")
                    })
                })
                .transpose()?,
            ..Default::default()
        }),
        "tool" => Request::Tool(required("TOOL_ID")?),
        "versions" => Request::Versions(required("TOOL_ID")?),
        "version" => Request::Version {
            tool: required("TOOL_ID")?,
            version: required("VERSION_ID")?,
        },
        "descriptor" | "files" | "tests" => {
            let tool = required("TOOL_ID")?;
            let version = required("VERSION_ID")?;
            let descriptor_type = parse_descriptor_type(&required("type")?)?;
            let plain = matches!(args.try_get_one::<bool>("plain"), Ok(Some(true)));
            match name {
                "descriptor" => Request::Descriptor {
                    tool,
                    version,
                    descriptor_type: with_plain(descriptor_type, plain),
                    plain,
                    path: string("path"),
                },
                "files" => Request::Files {
                    tool,
                    version,
                    descriptor_type,
                },
                _ => Request::Tests {
                    tool,
                    version,
                    descriptor_type: with_plain(descriptor_type, plain),
                    plain,
                },
            }
        }
        _ => anyhow::bail!("Unrecognized TRS command: {name}"),
    })
}

async fn execute(trs: &TRS, request: Request) -> Result<String, Box<dyn Error>> {
    match request {
        Request::Tools(filter) => json(serde_json::to_value(trs.list_tools(&filter).await?)),
        Request::Tool(id) => json(serde_json::to_value(trs.get_tool(&id).await?)),
        Request::Versions(id) => json(serde_json::to_value(trs.list_versions(&id).await?)),
        Request::Version { tool, version } => json(serde_json::to_value(
            trs.get_version(&tool, &version).await?,
        )),
        Request::Descriptor {
            tool,
            version,
            descriptor_type,
            plain,
            path,
        } => {
            let file = match path {
                Some(path) => {
                    trs.get_descriptor_file(&tool, &version, descriptor_type, &path)
                        .await?
                }
                None => trs.get_descriptor(&tool, &version, descriptor_type).await?,
            };
            if plain {
                Ok(file.content.unwrap_or_default())
            } else {
                json(serde_json::to_value(file))
            }
        }
        Request::Files {
            tool,
            version,
            descriptor_type,
        } => json(serde_json::to_value(
            trs.list_files(&tool, &version, descriptor_type).await?,
        )),
        Request::Tests {
            tool,
            version,
            descriptor_type,
            plain,
        } => {
            let files = trs.list_tests(&tool, &version, descriptor_type).await?;
            if plain {
                Ok(plain_files(&files))
            } else {
                json(serde_json::to_value(files))
            }
        }
    }
}

fn json(value: Result<serde_json::Value, serde_json::Error>) -> Result<String, Box<dyn Error>> {
    Ok(format!("{}\n", serde_json::to_string_pretty(&value?)?))
}

// print file's content
fn plain_files(files: &[FileWrapper]) -> String {
    files
        .iter()
        .filter_map(|file| file.content.clone().or_else(|| file.url.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn with_plain(descriptor_type: DescriptorType, plain: bool) -> DescriptorTypeWithPlain {
    match (descriptor_type, plain) {
        (DescriptorType::Cwl, false) => DescriptorTypeWithPlain::Cwl,
        (DescriptorType::Wdl, false) => DescriptorTypeWithPlain::Wdl,
        (DescriptorType::Nfl, false) => DescriptorTypeWithPlain::Nfl,
        (DescriptorType::Galaxy, false) => DescriptorTypeWithPlain::Galaxy,
        (DescriptorType::Smk, false) => DescriptorTypeWithPlain::Smk,
        (DescriptorType::Cwl, true) => DescriptorTypeWithPlain::PlainCwl,
        (DescriptorType::Wdl, true) => DescriptorTypeWithPlain::PlainWdl,
        (DescriptorType::Nfl, true) => DescriptorTypeWithPlain::PlainNfl,
        (DescriptorType::Galaxy, true) => DescriptorTypeWithPlain::PlainGalaxy,
        (DescriptorType::Smk, true) => DescriptorTypeWithPlain::PlainSmk,
    }
}

fn parse_descriptor_type(value: &str) -> anyhow::Result<DescriptorType> {
    let normalized = value.trim().to_ascii_uppercase();
    serde_json::from_value(serde_json::Value::String(normalized)).map_err(|_| {
        anyhow::anyhow!("Invalid descriptor type '{value}': expected CWL, WDL, NFL, GALAXY, or SMK")
    })
}

fn service_failure(error: &(dyn Error + 'static)) -> CommandOutput {
    let message = if error
        .downcast_ref::<HttpError>()
        .is_some_and(|http| http.status == 404)
    {
        format!("{error}. Check that base_path is the TRS API root")
    } else {
        error.to_string()
    };
    CommandOutput::failure(message)
}

#[cfg(test)]
mod tests {
    use super::{parse_descriptor_type, plain_files, with_plain};
    use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain, FileWrapper};

    #[test]
    fn descriptor_types_ignore_case_and_name_the_valid_values() {
        assert_eq!(parse_descriptor_type(" cwl ").unwrap(), DescriptorType::Cwl);
        assert_eq!(
            parse_descriptor_type("Galaxy").unwrap(),
            DescriptorType::Galaxy
        );
        let error = parse_descriptor_type("PLAIN_CWL").unwrap_err().to_string();
        assert!(error.contains("CWL, WDL, NFL, GALAXY, or SMK"), "{error}");
    }

    #[test]
    fn plain_switches_to_the_plain_variant() {
        assert_eq!(
            with_plain(DescriptorType::Wdl, false),
            DescriptorTypeWithPlain::Wdl
        );
        assert_eq!(
            with_plain(DescriptorType::Wdl, true),
            DescriptorTypeWithPlain::PlainWdl
        );
    }

    #[test]
    fn plain_files_print_content_or_url() {
        let files = [
            FileWrapper {
                content: Some("a: 1".into()),
                ..Default::default()
            },
            FileWrapper {
                url: Some("https://example.org/b.json".into()),
                ..Default::default()
            },
            FileWrapper::default(),
        ];
        assert_eq!(plain_files(&files), "a: 1\nhttps://example.org/b.json");
    }
}
