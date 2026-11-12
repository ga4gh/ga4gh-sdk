use crate::render::{json_printable, printable};
use crate::CommandOutput;
use clap::{arg, ArgAction, ArgMatches, Command};
use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain, FileWrapper};
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::HttpError;
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The largest ZIP archive that `trs files --zip` downloads.
const ZIP_LIMIT: usize = 64 * 1024 * 1024;

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
                .arg(
                    arg!(--tag <TAG> "Registry-specific tag; repeat for several tags")
                        .action(ArgAction::Append),
                )
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
                .about("List the files of a tool version, or download them as a ZIP archive")
                .arg(arg!(<TOOL_ID> "Tool ID"))
                .arg(arg!(<VERSION_ID> "Version ID"))
                .arg(type_arg())
                .arg(
                    arg!(--zip <FILE> "Write all files as a ZIP archive to FILE (must not exist)")
                        .value_parser(clap::value_parser!(PathBuf)),
                ),
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
    // Boxed: the filter is much larger than the other variants.
    Tools(Box<ToolFilter>),
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
        zip: Option<PathBuf>,
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
        "tools" => Request::Tools(tool_filter(ToolFilter {
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
            tags: args
                .get_many::<String>("tag")
                .map(|tags| tags.cloned().collect())
                .unwrap_or_default(),
            ..Default::default()
        })?),
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
                    zip: args
                        .get_one::<PathBuf>("zip")
                        .map(|path| new_file_arg(path.clone()))
                        .transpose()?,
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

// Checks the filter as the library would, so that invalid tags fail before
// the registry is contacted.
fn tool_filter(filter: ToolFilter) -> anyhow::Result<Box<ToolFilter>> {
    filter
        .query()
        .map_err(|error| anyhow::anyhow!("{}", printable(&error.to_string())))?;
    Ok(Box::new(filter))
}

// The archive never replaces an existing file; checked here so that the
// registry isn't contacted, and again when the archive is linked into place.
fn new_file_arg(path: PathBuf) -> anyhow::Result<PathBuf> {
    if path.exists() {
        anyhow::bail!(
            "{} already exists; choose another --zip file",
            printable(&path.display().to_string())
        );
    }
    Ok(path)
}

/// A new file that appears under its final name only when it is complete.
///
/// The content goes to a hidden temporary file next to the target, created
/// before the download so that an unwritable location fails early. `finish`
/// hard-links it to the target, which fails if the target exists, so nothing
/// is replaced. The temporary file is removed when this value is dropped, also
/// after errors; only an interrupted process can leave it behind.
struct NewFile {
    target: PathBuf,
    temp: PathBuf,
    file: std::fs::File,
}

impl NewFile {
    fn create(target: &Path) -> Result<Self, Box<dyn Error>> {
        let name = target
            .file_name()
            .ok_or_else(|| format!("{} is not a file path", target.display()))?;
        let mut temp_name = std::ffi::OsString::from(".");
        temp_name.push(name);
        temp_name.push(format!(".partial-{}", std::process::id()));
        let temp = target.with_file_name(temp_name);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|error| format!("Can't create {}: {error}", temp.display()))?;
        Ok(Self {
            target: target.to_path_buf(),
            temp,
            file,
        })
    }

    fn finish(mut self, bytes: &[u8]) -> Result<String, Box<dyn Error>> {
        self.file
            .write_all(bytes)
            .and_then(|()| self.file.sync_all())
            .map_err(|error| format!("Can't write {}: {error}", self.temp.display()))?;
        std::fs::hard_link(&self.temp, &self.target)
            .map_err(|error| format!("Can't create {}: {error}", self.target.display()))?;
        Ok(format!(
            "Wrote {} bytes to {}\n",
            bytes.len(),
            printable(&self.target.display().to_string())
        ))
    }
}

impl Drop for NewFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temp);
    }
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
            zip: Some(path),
        } => {
            // Before the download, so that an unusable path fails early.
            let file = NewFile::create(&path)?;
            let archive = trs
                .get_files_zip(&tool, &version, descriptor_type, ZIP_LIMIT)
                .await?;
            file.finish(&archive)
        }
        Request::Files {
            tool,
            version,
            descriptor_type,
            zip: None,
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
    use super::{parse_descriptor_type, plain_files, with_plain, NewFile};
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
    fn new_file_appears_complete_and_keeps_existing_files() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("workflow.zip");
        let entries = || std::fs::read_dir(directory.path()).unwrap().count();
        let file = NewFile::create(&path).unwrap();
        assert!(!path.exists());
        assert!(file
            .finish(b"PK new")
            .unwrap()
            .starts_with("Wrote 6 bytes to "));
        assert_eq!(std::fs::read(&path).unwrap(), b"PK new");
        assert_eq!(entries(), 1);

        // The file appeared after the arguments were checked.
        let file = NewFile::create(&path).unwrap();
        let error = file.finish(b"PK other").unwrap_err().to_string();
        assert!(error.contains("Can't create"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), b"PK new");
        assert_eq!(entries(), 1);

        // A dropped file (e.g. after a failed download) leaves nothing.
        drop(NewFile::create(&directory.path().join("other.zip")).unwrap());
        assert_eq!(entries(), 1);
    }

    #[test]
    fn new_file_fails_early_for_unusable_paths() {
        let directory = tempfile::TempDir::new().unwrap();
        for path in [directory.path().join("missing/workflow.zip"), "".into()] {
            assert!(NewFile::create(&path).is_err(), "{path:?}");
        }
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
