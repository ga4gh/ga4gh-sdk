//! Output interpretation and opt-in artifact evidence, independent of execution state.
//!
//! [`Profile::project_results`] checks each named output of a run against
//! the profile's output contract and gives a [`ResultRecord`]. HTTP(S) URLs
//! in it lose their credentials, query and fragment, so the record can be
//! stored. The `verify_` methods of [`ResultRecord`] then add
//! [`ArtifactEvidence`] for selected output artifacts: whether they could
//! be read, and whether their SHA-256 matches an expected checksum.
use super::{digest, pointer_segment, ContractError, Diagnostic, Profile};
use crate::{
    clients::drs::uri::{AccessPlan, AccessPreferences, DrsResolver},
    utils::{operation::Operation, transport::read_bounded},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
/// Whether an output conforms to its schema in the output contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    /// The output conforms to its schema, or is optional and absent.
    Conformant,
    /// The output is required but absent.
    Missing,
    /// The output doesn't conform to its schema.
    Invalid,
}
/// Whether an output artifact could be read.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    /// The output has a value, but its bytes weren't read.
    NotChecked,
    /// The bytes were read.
    Available,
    /// Resolving or reading the artifact failed or was interrupted.
    Inaccessible,
    /// The result has no value at the pointer.
    Missing,
}
/// Whether an output artifact's bytes match the expected checksum.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    /// No bytes were read, or no expected checksum was given.
    Unverified,
    /// The SHA-256 of the bytes matches the expected checksum.
    Verified,
    /// The SHA-256 of the bytes differs from the expected checksum.
    Mismatch,
}
/// A checksum that an artifact is expected to have, and where it comes
/// from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpectedChecksum {
    /// The checksum algorithm; only SHA-256 (`sha256` or `SHA-256`, in any
    /// case) is supported.
    pub algorithm: String,
    /// The expected checksum, as 64 hex digits.
    pub digest: String,
    /// Where the expected checksum comes from, such as a DRS object's
    /// checksums; not blank.
    pub evidence_source: String,
}
/// What a check found out about one output artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactEvidence {
    /// The JSON pointer of the artifact in the outputs; its first segment
    /// names the output.
    pub pointer: String,
    /// The output value, if it is a `drs://` URI.
    pub stable_identity: Option<String>,
    /// Whether the artifact could be read.
    pub access_status: AccessStatus,
    /// Whether its bytes match `expected`.
    pub integrity_status: IntegrityStatus,
    /// The checksum that the artifact is expected to have.
    pub expected: Option<ExpectedChecksum>,
    /// The SHA-256 of the bytes that were read, as 64 lowercase hex digits.
    pub computed_sha256: Option<String>,
    /// Why the artifact is inaccessible. Holds no URL, header or provider
    /// detail.
    pub detail: Option<String>,
}
/// One named output of a run, checked against its schema.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypedOutput {
    /// Whether credentials, a query or a fragment were removed from HTTP(S)
    /// URLs in the value.
    #[serde(default)]
    pub access_details_redacted: bool,
    /// The output value, with credentials, query and fragment removed from
    /// HTTP(S) URLs; `None` if the run didn't give it.
    pub value: Option<Value>,
    /// Whether the value conforms to the output's schema.
    pub contract_status: ContractStatus,
    /// The output's schema from the profile, with the same URL redaction.
    pub annotations: Value,
    /// The errors of validating the value against the output's schema.
    pub diagnostics: Vec<Diagnostic>,
}
/// The outputs of one run, interpreted under a profile's output contract,
/// with optional evidence about artifacts.
///
/// It is meant to be stored: it doesn't depend on the run still existing,
/// and holds no access credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultRecord {
    /// The format version; 1.
    pub format_version: u32,
    /// The caller's reference to the run, such as its WES run ID.
    pub run_reference: String,
    /// The run's state, as the caller passed it; not interpreted.
    pub execution_state: String,
    /// The [`Profile::schema_digest`] of the profile.
    pub schema_digest: String,
    /// Root scope for retained schema references and scientific/viewer annotations.
    #[serde(default)]
    pub contract_document: Value,
    /// The errors of validating all outputs against the output contract.
    pub contract_diagnostics: Vec<Diagnostic>,
    /// The outputs that the contract names, by name.
    pub outputs: BTreeMap<String, TypedOutput>,
    /// Evidence about output artifacts, by JSON pointer; filled by the
    /// `verify_` methods.
    pub artifacts: BTreeMap<String, ArtifactEvidence>,
    /// Context of the run, such as engine or reference versions, that the
    /// caller adds; [`ComparisonPolicy`](super::compare::ComparisonPolicy)
    /// compares it.
    pub context: BTreeMap<String, String>,
}
/// Strip access capabilities from URI values before durable serialization.
fn durable(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            if let Ok(mut u) = url::Url::parse(s) {
                if ["http", "https"].contains(&u.scheme()) {
                    let _ = u.set_username("");
                    let _ = u.set_password(None);
                    u.set_query(None);
                    u.set_fragment(None);
                    return json!(u.as_str());
                }
            }
            value.clone()
        }
        Value::Array(a) => Value::Array(a.iter().map(durable).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), durable(v))).collect()),
        _ => value.clone(),
    }
}
impl Profile {
    /// Checks a run's outputs against the output contract and returns them
    /// as a [`ResultRecord`], with empty `artifacts` and `context`.
    ///
    /// Each output named in the contract's `properties` gets a
    /// [`TypedOutput`]; other outputs appear only in
    /// `contract_diagnostics`. Reads no artifact.
    ///
    /// # Errors
    ///
    /// Fails if the profile has no output contract, if the contract has no
    /// named `properties`, or if an output's schema can't be compiled.
    pub fn project_results(
        &self,
        run_reference: &str,
        execution_state: &str,
        outputs: &Value,
    ) -> Result<ResultRecord, ContractError> {
        let all = self.validate_outputs(outputs)?;
        let contract = self
            .document()
            .pointer("/properties/outputs")
            .ok_or("no output contract")?;
        let properties = contract
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("output contract needs named properties")?;
        let required = contract.get("required").and_then(Value::as_array);
        let mut result = ResultRecord {
            format_version: 1,
            run_reference: run_reference.into(),
            execution_state: execution_state.into(),
            schema_digest: self.schema_digest.clone(),
            contract_document: self.document().clone(),
            contract_diagnostics: all,
            outputs: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            context: BTreeMap::new(),
        };
        for (key, annotation) in properties {
            let value = outputs.get(key);
            let is_required = required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some(key)));
            let errors = if let Some(value) = value {
                self.validate_at(
                    &format!("/properties/outputs/properties/{}", pointer_segment(key)),
                    value,
                )?
            } else {
                vec![]
            };
            let status = if value.is_none() && is_required {
                ContractStatus::Missing
            } else if !errors.is_empty() {
                ContractStatus::Invalid
            } else {
                ContractStatus::Conformant
            };
            result.outputs.insert(
                key.clone(),
                TypedOutput {
                    access_details_redacted: value.is_some_and(|v| durable(v) != *v),
                    value: value.map(durable),
                    contract_status: status,
                    annotations: durable(annotation),
                    diagnostics: errors,
                },
            );
        }
        Ok(result)
    }
}
impl ResultRecord {
    /// Returns the value at a JSON pointer into the outputs, whose first
    /// segment names an output; `None` if there is none.
    pub fn value_at(&self, pointer: &str) -> Option<&Value> {
        let path = pointer.strip_prefix('/')?;
        let (key, rest) = path
            .split_once('/')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((path, None));
        let key = key.replace("~1", "/").replace("~0", "~");
        let value = self.outputs.get(&key)?.value.as_ref()?;
        match rest {
            Some(rest) => value.pointer(&format!("/{rest}")),
            None => Some(value),
        }
    }
    fn evidence(&self, pointer: &str, expected: Option<ExpectedChecksum>) -> ArtifactEvidence {
        let value = self.value_at(pointer);
        ArtifactEvidence {
            pointer: pointer.into(),
            stable_identity: value
                .and_then(Value::as_str)
                .filter(|s| s.starts_with("drs://"))
                .map(str::to_owned),
            access_status: if value.is_some() {
                AccessStatus::NotChecked
            } else {
                AccessStatus::Missing
            },
            integrity_status: IntegrityStatus::Unverified,
            expected,
            computed_sha256: None,
            detail: None,
        }
    }
    /// Records evidence for bytes that the caller read for the artifact at
    /// `pointer`, under that pointer in `artifacts`.
    ///
    /// The SHA-256 of `bytes` is compared with `expected`; without it, the
    /// integrity stays [`IntegrityStatus::Unverified`]. If the result has
    /// no value at `pointer`, the access is [`AccessStatus::Missing`] and
    /// `bytes` and `expected` are ignored.
    ///
    /// # Errors
    ///
    /// Fails, and records nothing, if `expected` names an algorithm other
    /// than SHA-256, its digest isn't 64 hex digits, or its evidence source
    /// is blank.
    pub fn verify_bytes(
        &mut self,
        pointer: &str,
        bytes: &[u8],
        expected: Option<ExpectedChecksum>,
    ) -> Result<(), ContractError> {
        let mut evidence = self.evidence(pointer, expected);
        if self.value_at(pointer).is_none() {
            self.artifacts.insert(pointer.into(), evidence);
            return Ok(());
        }
        evidence.access_status = AccessStatus::Available;
        evidence.computed_sha256 = Some(digest(bytes));
        if let Some(expected) = &evidence.expected {
            if expected.algorithm.to_ascii_lowercase().replace('-', "") != "sha256" {
                return Err("unsupported checksum algorithm".into());
            }
            if expected.evidence_source.trim().is_empty()
                || expected.digest.len() != 64
                || !expected.digest.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("expected SHA-256 needs a valid digest and evidence source".into());
            }
            evidence.integrity_status = if expected
                .digest
                .eq_ignore_ascii_case(evidence.computed_sha256.as_ref().unwrap())
            {
                IntegrityStatus::Verified
            } else {
                IntegrityStatus::Mismatch
            };
        }
        self.artifacts.insert(pointer.into(), evidence);
        Ok(())
    }
    /// Reads the artifact at `pointer` with a DRS access plan and records
    /// its evidence, as [`Self::verify_bytes`] does.
    ///
    /// The plan's URL and headers are used once and never stored. The
    /// request uses a fresh client: it follows no redirects and sends no
    /// SDK credentials. At most `max_bytes` are read, within the bounds of
    /// `operation`. If the request fails, returns no success status or
    /// goes over `max_bytes`, the access is [`AccessStatus::Inaccessible`].
    ///
    /// # Errors
    ///
    /// Fails if the HTTP client can't be built, or as
    /// [`Self::verify_bytes`] does.
    pub async fn verify_access_plan(
        &mut self,
        pointer: &str,
        plan: &AccessPlan,
        expected: Option<ExpectedChecksum>,
        operation: &Operation,
        max_bytes: usize,
    ) -> Result<(), ContractError> {
        if self.value_at(pointer).is_none() {
            let evidence = self.evidence(pointer, expected);
            self.artifacts.insert(pointer.into(), evidence);
            return Ok(());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let result = operation
            .run(async {
                let response = client
                    .get(plan.url.clone())
                    .headers(plan.headers.clone())
                    .send()
                    .await?;
                if !response.status().is_success() {
                    return Err::<Vec<u8>, ContractError>(
                        format!("artifact HTTP {}", response.status()).into(),
                    );
                }
                Ok(read_bounded(response, max_bytes).await?.body)
            })
            .await;
        match result {
            Ok(Ok(bytes)) => self.verify_bytes(pointer, &bytes, expected),
            _ => {
                let mut evidence = self.evidence(pointer, expected);
                evidence.access_status = AccessStatus::Inaccessible;
                evidence.detail=Some("selected artifact access failed or was interrupted; resolve again through the owning provider".into());
                self.artifacts.insert(pointer.into(), evidence);
                Ok(())
            }
        }
    }
    /// Resolves the DRS URI at `pointer` with `resolver` and reads the
    /// artifact, as [`Self::verify_access_plan`] does.
    ///
    /// Resolution takes at most `max_attempts` attempts, as
    /// [`DrsResolver::access_plan`] does. If it fails, the access is
    /// [`AccessStatus::Inaccessible`].
    ///
    /// # Errors
    ///
    /// Fails if the value at `pointer` is missing or isn't a string, or as
    /// [`Self::verify_access_plan`] does.
    #[allow(clippy::too_many_arguments)] // Explicit resolver, evidence and operation bounds stay caller-owned.
    pub async fn verify_drs(
        &mut self,
        pointer: &str,
        resolver: &DrsResolver,
        preferences: &AccessPreferences,
        expected: Option<ExpectedChecksum>,
        operation: &Operation,
        max_attempts: usize,
        max_bytes: usize,
    ) -> Result<(), ContractError> {
        let uri = self
            .value_at(pointer)
            .and_then(Value::as_str)
            .ok_or("selected output is not an artifact URI")?
            .to_owned();
        match resolver
            .access_plan(&uri, preferences, operation, max_attempts)
            .await
        {
            Ok(plan) => {
                self.verify_access_plan(pointer, &plan, expected, operation, max_bytes)
                    .await
            }
            Err(_) => {
                let mut evidence = self.evidence(pointer, expected);
                evidence.access_status = AccessStatus::Inaccessible;
                evidence.detail = Some("DRS resolution unavailable or interrupted".into());
                self.artifacts.insert(pointer.into(), evidence);
                Ok(())
            }
        }
    }
    /// Returns the outputs that the profile marks for an HTML viewer
    /// (`x-ga4gh-ui-widget: html-viewer`), with their key, media type,
    /// value and contract status. Consumers decide authorization and
    /// presentation.
    pub fn html_viewer_metadata(&self) -> Vec<Value> {
        self.outputs.iter().filter(|(_,v)|v.annotations.get("x-ga4gh-ui-widget").and_then(Value::as_str)==Some("html-viewer")).map(|(key,v)|json!({"key":key,"widget":"html-viewer","contentMediaType":v.annotations.get("contentMediaType"),"value":v.value,"contract_status":v.contract_status})).collect()
    }
}
