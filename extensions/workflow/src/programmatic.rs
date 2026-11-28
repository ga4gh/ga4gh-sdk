use base64::{engine::general_purpose::STANDARD, Engine};
use ga4gh_cli_protocol::{encode, Attachment};
use ga4gh_sdk::{
    clients::wes::{models::RunRequest, WorkflowAttachment},
    workflow::{
        compare::ComparisonPolicy,
        results::{ExpectedChecksum, ResultRecord},
        Diagnostic, InputValidationError, InvocationIdentity, Pins, PreparedInvocation, Profile,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::LazyLock;

static PREPARE: LazyLock<jsonschema::Validator> = LazyLock::new(|| {
    let schema =
        serde_json::from_str(include_str!("../api/v1/prepare-request.schema.json")).unwrap();
    jsonschema::validator_for(&schema).unwrap()
});

#[derive(Deserialize)]
struct Request {
    profile_utf8: String,
    request: RunRequest,
    pins: Pins,
    identity: InvocationIdentity,
}

#[derive(Serialize)]
struct Prepared<'a> {
    state_ref: &'a str,
    record: &'a PreparedInvocation,
}

#[derive(Serialize)]
struct PublicDiagnostic<'a> {
    instance_location: &'a str,
    schema_location: &'a str,
    message: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum Reply<'a> {
    Prepared(Prepared<'a>),
    Error {
        code: &'static str,
        message: &'static str,
        diagnostics: Vec<PublicDiagnostic<'a>>,
    },
    Released(bool),
}

fn error(code: &'static str, message: &'static str, diagnostics: &[Diagnostic]) -> Option<Vec<u8>> {
    let oversized = || {
        encode(&Reply::Error {
            code: "result_too_large",
            message: "Workflow diagnostics exceed the message limit",
            diagnostics: Vec::new(),
        })
        .ok()
    };
    if diagnostics
        .iter()
        .try_fold(0usize, |size, item| {
            size.checked_add(64)?
                .checked_add(item.instance_location.len())?
                .checked_add(item.schema_location.len())
        })
        .is_none_or(|size| size > ga4gh_cli_protocol::MAX_MESSAGE)
    {
        return oversized();
    }
    encode(&Reply::Error {
        code,
        message,
        diagnostics: diagnostics
            .iter()
            .map(|item| PublicDiagnostic {
                instance_location: &item.instance_location,
                schema_location: &item.schema_location,
                message: "Input does not satisfy this schema constraint",
            })
            .collect(),
    })
    .ok()
    .or_else(oversized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_diagnostic_stays_a_workflow_error() {
        let diagnostic = Diagnostic {
            instance_location: "x".repeat(ga4gh_cli_protocol::MAX_MESSAGE),
            schema_location: String::new(),
            message: String::new(),
        };
        let reply = error("validation_failed", "Invalid inputs", &[diagnostic]).unwrap();
        let value: Value = serde_json::from_slice(&reply).unwrap();
        assert_eq!(value["error"]["code"], "result_too_large");
        assert_eq!(value["error"]["diagnostics"], serde_json::json!([]));
    }
}

#[derive(Default)]
pub(super) struct Preparations {
    live: Option<Live>,
}

struct Live {
    key: String,
    plan: PreparedInvocation,
    action_issued: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Projection {
    profile_utf8: String,
    pins: Pins,
    run_reference: String,
    state: String,
    outputs: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Verification {
    record: ResultRecord,
    pointer: String,
    base64: String,
    expected: Option<ExpectedChecksum>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Comparison {
    policy: ComparisonPolicy,
    left: ResultRecord,
    right: ResultRecord,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Submission {
    state_ref: String,
    attachments: Vec<Attachment>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Completion {
    state_ref: String,
    outcome: Outcome,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Submitted { run_id: String },
    Failed,
}

fn success(value: &impl Serialize) -> Option<Vec<u8>> {
    #[derive(Serialize)]
    struct Success<'a, T> {
        ok: &'a T,
    }
    encode(&Success { ok: value }).ok().or_else(|| {
        error(
            "result_too_large",
            "Workflow reply exceeds message limit",
            &[],
        )
    })
}

impl Preparations {
    pub(super) fn invoke(&mut self, operation: &[u8], input: &[u8]) -> Option<Vec<u8>> {
        let invalid = || error("invalid_request", "Malformed workflow request", &[]);
        let validation = || {
            error(
                "validation_failed",
                "Invalid workflow operation inputs",
                &[],
            )
        };
        let state_error = || {
            error(
                "invalid_state",
                "Unknown, released, or already submitted preparation",
                &[],
            )
        };
        let Ok(value) = serde_json::from_slice::<Value>(input) else {
            return invalid();
        };
        match operation {
            b"results" => {
                let Ok(req) = serde_json::from_value::<Projection>(value) else {
                    return invalid();
                };
                let Ok(record) = Profile::from_slice(req.profile_utf8.as_bytes(), req.pins)
                    .and_then(|profile| {
                        profile.project_results(&req.run_reference, &req.state, &req.outputs)
                    })
                else {
                    return validation();
                };
                success(&record)
            }
            b"verify" => {
                let Ok(mut req) = serde_json::from_value::<Verification>(value) else {
                    return invalid();
                };
                let Ok(bytes) = STANDARD.decode(req.base64) else {
                    return invalid();
                };
                if req.record.format_version != 1
                    || req
                        .record
                        .verify_bytes(&req.pointer, &bytes, req.expected)
                        .is_err()
                {
                    return validation();
                }
                success(&req.record)
            }
            b"compare" => {
                let Ok(req) = serde_json::from_value::<Comparison>(value) else {
                    return invalid();
                };
                if req.left.format_version != 1 || req.right.format_version != 1 {
                    return validation();
                }
                let Ok(report) = req.policy.compare(&req.left, &req.right) else {
                    return validation();
                };
                success(&report)
            }
            b"begin_submit" => {
                let Ok(req) = serde_json::from_value::<Submission>(value) else {
                    return invalid();
                };
                let Some(live) = self
                    .live
                    .as_mut()
                    .filter(|live| live.key == req.state_ref && !live.action_issued)
                else {
                    return state_error();
                };
                let Ok(files) = req
                    .attachments
                    .into_iter()
                    .map(WorkflowAttachment::try_from)
                    .collect::<Result<Vec<_>, _>>()
                else {
                    return invalid();
                };
                if live.plan.validate_attachments(&files).is_err() {
                    return validation();
                }
                #[derive(Serialize)]
                struct Action<'a> {
                    request: &'a RunRequest,
                    attachments: Vec<Attachment>,
                }
                #[derive(Serialize)]
                struct ActionReply<'a> {
                    ok: Action<'a>,
                }
                let Ok(reply) = encode(&ActionReply {
                    ok: Action {
                        request: live.plan.request(),
                        attachments: files.into_iter().map(Into::into).collect(),
                    },
                }) else {
                    return error(
                        "result_too_large",
                        "Submission action exceeds message limit",
                        &[],
                    );
                };
                live.action_issued = true;
                Some(reply)
            }
            b"finish_submit" => {
                let Ok(req) = serde_json::from_value::<Completion>(value) else {
                    return invalid();
                };
                if !self
                    .live
                    .as_ref()
                    .is_some_and(|live| live.action_issued && live.key == req.state_ref)
                {
                    return state_error();
                }
                if matches!(&req.outcome, Outcome::Submitted {run_id} if run_id.trim().is_empty()) {
                    return invalid();
                }
                let plan = self.live.take()?.plan;
                match req.outcome {
                    Outcome::Failed => success(&false),
                    Outcome::Submitted { run_id } => {
                        #[derive(Serialize)]
                        struct Receipt {
                            run_id: String,
                            invocation: PreparedInvocation,
                        }
                        success(&Receipt {
                            run_id,
                            invocation: plan,
                        })
                    }
                }
            }
            b"prepare" => {
                if self.live.is_some() {
                    return error("state_in_use", "Release the current preparation first", &[]);
                }
                if !PREPARE.is_valid(&value) {
                    return invalid();
                }
                let Ok(request) = serde_json::from_value::<Request>(value) else {
                    return invalid();
                };
                let Ok(profile) =
                    Profile::from_slice(request.profile_utf8.as_bytes(), request.pins)
                else {
                    return error(
                        "validation_failed",
                        "Invalid workflow profile or revision pins",
                        &[],
                    );
                };
                let record = match profile.prepare(request.request, request.identity) {
                    Ok(record) => record,
                    Err(cause) => {
                        let diagnostics = cause
                            .downcast_ref::<InputValidationError>()
                            .map_or(&[][..], |failure| failure.0.as_slice());
                        return error(
                            "validation_failed",
                            "Invalid workflow preparation",
                            diagnostics,
                        );
                    }
                };
                let key = uuid::Uuid::new_v4().to_string();
                let Ok(reply) = encode(&Reply::Prepared(Prepared {
                    state_ref: &key,
                    record: &record,
                })) else {
                    return error(
                        "result_too_large",
                        "Prepared reply exceeds the workflow message limit",
                        &[],
                    );
                };
                self.live = Some(Live {
                    key,
                    plan: record,
                    action_issued: false,
                });
                Some(reply)
            }
            b"release" => {
                let Some(key) = value
                    .as_object()
                    .filter(|object| object.len() == 1)
                    .and_then(|object| object.get("state_ref"))
                    .and_then(Value::as_str)
                    .filter(|key| !key.is_empty())
                else {
                    return invalid();
                };
                if !self.live.as_ref().is_some_and(|live| live.key == key) {
                    return error("invalid_state", "Unknown or released preparation", &[]);
                }
                if self.live.as_ref().is_some_and(|live| live.action_issued) {
                    return error(
                        "invalid_state",
                        "An issued action requires a submission outcome",
                        &[],
                    );
                }
                self.live = None;
                encode(&Reply::Released(true)).ok()
            }
            _ => None,
        }
    }
}
