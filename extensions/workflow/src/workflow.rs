use ga4gh_cli_protocol::{Invocation, Output, Reply, SubmissionOutcome, MAX_MESSAGE};
use ga4gh_sdk::{
    clients::wes::{models::RunRequest, WorkflowAttachment},
    workflow::{
        compare::ComparisonPolicy,
        results::{ExpectedChecksum, ResultRecord},
        InvocationIdentity, Pins, PreparedInvocation, Profile,
    },
};
use serde_json::{json, Value};
use std::io::Read;

fn read(path: &str) -> anyhow::Result<Vec<u8>> {
    let mut data = Vec::new();
    std::fs::File::open(path)?
        .take((8 << 20) + 1)
        .read_to_end(&mut data)?;
    anyhow::ensure!(data.len() <= 8 << 20, "JSON document exceeds 8 MiB");
    Ok(data)
}

fn output(value: Value) -> anyhow::Result<Output> {
    Ok(Output {
        stdout: format!("{}\n", serde_json::to_string_pretty(&value)?),
        json_stdout: true,
        stderr: String::new(),
        exit_status: 0,
    })
}

pub fn execute(
    call: Invocation,
    pending: &mut Option<PreparedInvocation>,
) -> anyhow::Result<Reply> {
    anyhow::ensure!(
        call.group == "workflow" && pending.is_none(),
        "Unexpected workflow invocation"
    );
    let optional = |key: &str| call.arguments.get(key).and_then(|values| values.first());
    let get = |key: &str| {
        optional(key)
            .map(String::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing argument: {key}"))
    };
    let load =
        |key: &str| -> anyhow::Result<Value> { Ok(serde_json::from_slice(&read(get(key)?)?)?) };
    let profile = || -> anyhow::Result<Profile> {
        Ok(Profile::from_slice(
            &read(get("schema")?)?,
            Pins {
                profile_revision: get("profile-revision")?.into(),
                vocabulary_revision: get("vocabulary-revision")?.into(),
            },
        )?)
    };
    let value = match call.command.as_str() {
        "prepare" | "submit" => {
            let profile = profile()?;
            let request: RunRequest = serde_json::from_value(load("request")?)?;
            let invocation = profile.prepare(
                request,
                InvocationIdentity {
                    workflow: get("workflow-identity")?.into(),
                    descriptor_digest: optional("descriptor-digest").cloned(),
                },
            )?;
            if call.command == "submit" {
                let mut attachments = Vec::new();
                let mut encoded_size = 0;
                for spec in call.arguments.get("attachment").into_iter().flatten() {
                    let (filename, path) = spec.split_once('=').ok_or_else(|| {
                        anyhow::anyhow!("attachment must use filename=local-path")
                    })?;
                    let content = read(path)?;
                    encoded_size += content.len().div_ceil(3) * 4 + filename.len() + 32;
                    anyhow::ensure!(
                        encoded_size <= MAX_MESSAGE,
                        "CLI extension message exceeds 64 MiB"
                    );
                    attachments.push(WorkflowAttachment {
                        filename: filename.to_owned(),
                        content,
                    });
                }
                if !attachments.is_empty() {
                    invocation.validate_attachments(&attachments)?;
                }
                let request = Box::new(invocation.request().clone());
                *pending = Some(invocation);
                return Ok(Reply::WesSubmit {
                    request,
                    attachments: attachments.into_iter().map(Into::into).collect(),
                });
            }
            serde_json::to_value(invocation)?
        }
        "results" => {
            let profile = profile()?;
            let log = load("run-log")?;
            serde_json::to_value(
                profile.project_results(
                    get("run-reference")?,
                    log.get("state")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow::anyhow!("run log needs state"))?,
                    log.get("outputs")
                        .ok_or_else(|| anyhow::anyhow!("run log needs named outputs"))?,
                )?,
            )?
        }
        "verify" => {
            let mut record: ResultRecord = serde_json::from_value(load("result")?)?;
            let expected = match optional("sha256") {
                Some(digest) => Some(ExpectedChecksum {
                    algorithm: "sha256".into(),
                    digest: digest.clone(),
                    evidence_source: get("evidence-source")?.into(),
                }),
                None => None,
            };
            record.verify_bytes(get("pointer")?, &read(get("bytes")?)?, expected)?;
            serde_json::to_value(record)?
        }
        "compare" => {
            let policy: ComparisonPolicy = serde_json::from_value(load("policy")?)?;
            let left: ResultRecord = serde_json::from_value(load("left")?)?;
            let right: ResultRecord = serde_json::from_value(load("right")?)?;
            serde_json::to_value(policy.compare(&left, &right)?)?
        }
        _ => anyhow::bail!("unknown workflow command"),
    };
    Ok(Reply::Complete(output(value)?))
}

pub fn finish(
    invocation: PreparedInvocation,
    outcome: SubmissionOutcome,
) -> anyhow::Result<Output> {
    match outcome {
        SubmissionOutcome::Submitted { run_id } => {
            output(json!({"run_id":run_id,"invocation":invocation}))
        }
        SubmissionOutcome::Failed { message } => Ok(Output::failure(message)),
    }
}
