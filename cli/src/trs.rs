use crate::render::{json_printable, printable};
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
        Err(error) => CommandOutput::failure(printable(&error.to_string())),
    })
}

fn parse(sub: &ArgMatches) -> anyhow::Result<Request> {
    let (name, args) = sub
        .subcommand()
        .ok_or_else(|| anyhow::anyhow!("TRS subcommand is required"))?;
    let string = |id: &str| args.get_one::<String>(id).cloned();
    let required =
        |id: &str| string(id).ok_or_else(|| anyhow::anyhow!("{id} argument is required"));
    let segment = |id: &str| segment_arg(id, required(id)?);

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
                        anyhow::anyhow!(
                            "Invalid limit '{}': expected an integer",
                            printable(&value)
                        )
                    })
                })
                .transpose()?,
            ..Default::default()
        }),
        "tool" => Request::Tool(segment("TOOL_ID")?),
        "versions" => Request::Versions(segment("TOOL_ID")?),
        "version" => Request::Version {
            tool: segment("TOOL_ID")?,
            version: segment("VERSION_ID")?,
        },
        "descriptor" | "files" | "tests" => {
            let tool = segment("TOOL_ID")?;
            let version = segment("VERSION_ID")?;
            let descriptor_type = parse_descriptor_type(&required("type")?)?;
            let plain = matches!(args.try_get_one::<bool>("plain"), Ok(Some(true)));
            match name {
                "descriptor" => Request::Descriptor {
                    tool,
                    version,
                    descriptor_type: with_plain(descriptor_type, plain),
                    plain,
                    path: string("path")
                        .map(|value| segment_arg("path", value))
                        .transpose()?,
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

fn segment_arg(name: &str, value: String) -> anyhow::Result<String> {
    if value.is_empty() || matches!(value.as_str(), "." | "..") {
        anyhow::bail!("{name} must not be empty, '.' or '..'");
    }
    Ok(value)
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
                file.content.ok_or_else(|| {
                    "TRS descriptor has no inline content; omit --plain to inspect its URL".into()
                })
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
                plain_files(&files)
            } else {
                json(serde_json::to_value(files))
            }
        }
    }
}

fn json(value: Result<serde_json::Value, serde_json::Error>) -> Result<String, Box<dyn Error>> {
    Ok(format!(
        "{}\n",
        json_printable(&serde_json::to_string_pretty(&value?)?)
    ))
}

// A plain response is one file's bytes. Multiple wrappers have no unambiguous
// plain representation; JSON output retains their names and URLs.
fn plain_files(files: &[FileWrapper]) -> Result<String, Box<dyn Error>> {
    match files {
        [file] => file.content.clone().ok_or_else(|| {
            "TRS test file has no inline content; omit --plain to inspect its URL".into()
        }),
        [] => Err("TRS returned no test file for --plain".into()),
        _ => Err("TRS returned multiple test files; omit --plain to inspect them".into()),
    }
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
        (DescriptorType::Unrecognized(value), false) => {
            DescriptorTypeWithPlain::Unrecognized(value)
        }
        (DescriptorType::Unrecognized(value), true) => {
            DescriptorTypeWithPlain::Unrecognized(format!("PLAIN_{value}"))
        }
    }
}

fn parse_descriptor_type(value: &str) -> anyhow::Result<DescriptorType> {
    let normalized = value.trim().to_ascii_uppercase();
    serde_json::from_value::<DescriptorType>(serde_json::Value::String(normalized))
        .ok()
        .filter(|descriptor_type| !matches!(descriptor_type, DescriptorType::Unrecognized(_)))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Invalid descriptor type '{}': expected CWL, WDL, NFL, GALAXY, or SMK",
                printable(value)
            )
        })
}

fn service_failure(error: &(dyn Error + 'static)) -> CommandOutput {
    let message = if error
        .downcast_ref::<HttpError>()
        .is_some_and(|http| http.status == 404)
    {
        format!(
            "{}. Check that base_path is the TRS API root",
            printable(&error.to_string())
        )
    } else {
        printable(&error.to_string())
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
    fn plain_files_require_one_inline_file() {
        let inline = FileWrapper {
            content: Some("a: 1".into()),
            ..Default::default()
        };
        let url_only = FileWrapper {
            url: Some("https://example.org/b.json".into()),
            ..Default::default()
        };
        assert_eq!(plain_files(std::slice::from_ref(&inline)).unwrap(), "a: 1");
        assert!(plain_files(&[url_only])
            .unwrap_err()
            .to_string()
            .contains("no inline content"));
        assert!(plain_files(&[inline.clone(), inline])
            .unwrap_err()
            .to_string()
            .contains("multiple"));
        assert!(plain_files(&[])
            .unwrap_err()
            .to_string()
            .contains("no test file"));
    }
}
