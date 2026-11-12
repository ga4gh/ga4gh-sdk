//! Offline workflow contract commands; submission is explicit and uses WES configuration.
use crate::CommandOutput;
use clap::{Arg, ArgMatches, Command};
use ga4gh_sdk::{
    clients::{
        wes::{models::RunRequest, WorkflowAttachment, WES},
        ServiceType,
    },
    utils::configuration::Configuration,
    workflow::{
        compare::ComparisonPolicy,
        results::{ExpectedChecksum, ResultRecord},
        InvocationIdentity, Pins, Profile,
    },
};
use serde_json::Value;
fn file_arg(name: &'static str) -> Arg {
    Arg::new(name).required(true)
}
pub fn command() -> Command {
    let pins = |c: Command| {
        c.arg(
            Arg::new("profile-revision")
                .long("profile-revision")
                .required(true),
        )
        .arg(
            Arg::new("vocabulary-revision")
                .long("vocabulary-revision")
                .required(true),
        )
    };
    let invocation = |name: &'static str| {
        let command = pins(
            Command::new(name)
                .arg(file_arg("schema"))
                .arg(file_arg("request"))
                .arg(file_arg("workflow-identity"))
                .arg(Arg::new("descriptor-digest").long("descriptor-digest")),
        );
        if name == "submit" {
            command.arg(
                Arg::new("attachment")
                    .long("attachment")
                    .action(clap::ArgAction::Append)
                    .help("Explicit filename=local-path attachment"),
            )
        } else {
            command
        }
    };
    Command::new("workflow")
        .about("Client-side workflow contracts; submission uses WES")
        .subcommand_required(true)
        .subcommand(invocation("prepare"))
        .subcommand(invocation("submit"))
        .subcommand(pins(
            Command::new("results")
                .arg(file_arg("schema"))
                .arg(file_arg("run-log"))
                .arg(file_arg("run-reference")),
        ))
        .subcommand(
            Command::new("verify")
                .arg(file_arg("result"))
                .arg(file_arg("pointer"))
                .arg(file_arg("bytes"))
                .arg(
                    Arg::new("sha256")
                        .long("sha256")
                        .requires("evidence-source"),
                )
                .arg(
                    Arg::new("evidence-source")
                        .long("evidence-source")
                        .requires("sha256"),
                ),
        )
        .subcommand(
            Command::new("compare")
                .arg(file_arg("policy"))
                .arg(file_arg("left"))
                .arg(file_arg("right")),
        )
}
fn read(path: &str) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take(8 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    if data.len() > 8 * 1024 * 1024 {
        anyhow::bail!("JSON document exceeds 8 MiB");
    }
    Ok(data)
}
fn load<T: serde::de::DeserializeOwned>(path: &str) -> anyhow::Result<T> {
    Ok(serde_json::from_slice(&read(path)?)?)
}
/// Run a workflow command. Submission uses the supplied configuration when present;
/// the standalone CLI reads its WES section when no configuration is supplied.
pub async fn run(
    args: &ArgMatches,
    service_config: Option<&Configuration>,
) -> anyhow::Result<CommandOutput> {
    let result: Result<Value, Box<dyn std::error::Error>> = async {
        let (name, m) = args.subcommand().ok_or("missing workflow command")?;
        let get = |key: &str| {
            m.get_one::<String>(key)
                .map(String::as_str)
                .ok_or("missing argument")
        };
        let read_value = |key: &str| -> Result<Value, Box<dyn std::error::Error>> {
            Ok(load(get(key)?).map_err(|e| e.to_string())?)
        };
        match name {
            "prepare" | "submit" => {
                let bytes = read(get("schema")?).map_err(|e| e.to_string())?;
                let p = Profile::from_slice(
                    &bytes,
                    Pins {
                        profile_revision: get("profile-revision")?.into(),
                        vocabulary_revision: get("vocabulary-revision")?.into(),
                    },
                )?;
                let request: RunRequest = serde_json::from_value(read_value("request")?)?;
                let invocation = p.prepare(
                    request,
                    InvocationIdentity {
                        workflow: get("workflow-identity")?.into(),
                        descriptor_digest: m.get_one::<String>("descriptor-digest").cloned(),
                    },
                )?;
                if name == "prepare" {
                    Ok(serde_json::to_value(invocation)?)
                } else {
                    let config = match service_config {
                        Some(config) => config.clone(),
                        None => Configuration::from_file(ServiceType::WES)?,
                    };
                    // The transport sends the configured credential; this
                    // client only adds the redirect policy and time limits.
                    let client = ga4gh_sdk::reqwest::Client::builder()
                        .redirect(ga4gh_sdk::reqwest::redirect::Policy::none())
                        .connect_timeout(std::time::Duration::from_secs(10))
                        .timeout(std::time::Duration::from_secs(60))
                        .build()?;
                    let wes = WES::from_transport(
                        ga4gh_sdk::utils::transport::Transport::with_client(&config, client),
                    )
                    .await?;
                    let attachments = m
                        .get_many::<String>("attachment")
                        .into_iter()
                        .flatten()
                        .map(|spec| {
                            let (filename, path) = spec
                                .split_once('=')
                                .ok_or("attachment must use filename=local-path")?;
                            Ok(WorkflowAttachment {
                                filename: filename.into(),
                                content: read(path).map_err(|e| e.to_string())?,
                            })
                        })
                        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
                    let run = if attachments.is_empty() {
                        invocation.submit(&wes).await?
                    } else {
                        invocation
                            .submit_with_attachments(&wes, attachments)
                            .await?
                    };
                    Ok(serde_json::json!({"run_id":run.id,"invocation":invocation}))
                }
            }
            "results" => {
                let p = Profile::from_slice(
                    &read(get("schema")?).map_err(|e| e.to_string())?,
                    Pins {
                        profile_revision: get("profile-revision")?.into(),
                        vocabulary_revision: get("vocabulary-revision")?.into(),
                    },
                )?;
                let log = read_value("run-log")?;
                Ok(serde_json::to_value(
                    p.project_results(
                        get("run-reference")?,
                        log.get("state")
                            .and_then(Value::as_str)
                            .ok_or("run log needs state")?,
                        log.get("outputs").ok_or("run log needs named outputs")?,
                    )?,
                )?)
            }
            "verify" => {
                let mut r: ResultRecord = serde_json::from_value(read_value("result")?)?;
                let expected = m
                    .get_one::<String>("sha256")
                    .map(|digest| ExpectedChecksum {
                        algorithm: "sha256".into(),
                        digest: digest.clone(),
                        evidence_source: m.get_one::<String>("evidence-source").unwrap().clone(),
                    });
                r.verify_bytes(
                    get("pointer")?,
                    &read(get("bytes")?).map_err(|e| e.to_string())?,
                    expected,
                )?;
                Ok(serde_json::to_value(r)?)
            }
            "compare" => {
                let policy: ComparisonPolicy = serde_json::from_value(read_value("policy")?)?;
                let left: ResultRecord = serde_json::from_value(read_value("left")?)?;
                let right: ResultRecord = serde_json::from_value(read_value("right")?)?;
                Ok(serde_json::to_value(policy.compare(&left, &right)?)?)
            }
            _ => Err("unknown workflow command".into()),
        }
    }
    .await;
    Ok(match result {
        Ok(value) => CommandOutput::success(format!("{}\n", serde_json::to_string_pretty(&value)?)),
        Err(e) => CommandOutput::failure(e),
    })
}
