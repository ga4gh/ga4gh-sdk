use ga4gh_cli_protocol::encode;
use ga4gh_sdk::{
    clients::wes::models::RunRequest,
    workflow::{
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
}

#[derive(Default)]
pub(super) struct Preparations(Option<(String, PreparedInvocation)>);

impl Preparations {
    pub(super) fn invoke(&mut self, operation: &[u8], input: &[u8]) -> Option<Vec<u8>> {
        if !matches!(operation, b"prepare" | b"release") {
            return None;
        }
        let invalid = || error("invalid_request", "Malformed workflow request", &[]);
        let Ok(value) = serde_json::from_slice::<Value>(input) else {
            return invalid();
        };
        match operation {
            b"prepare" => {
                if self.0.is_some() {
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
                self.0 = Some((key, record));
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
                if !self.0.as_ref().is_some_and(|(held, _)| held == key) {
                    return error("invalid_state", "Unknown or released preparation", &[]);
                }
                self.0 = None;
                encode(&Reply::Released(true)).ok()
            }
            _ => None,
        }
    }
}
