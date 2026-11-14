//! Output interpretation and opt-in artifact evidence, independent of execution state.
use super::{digest, pointer_segment, ContractError, Diagnostic, Profile};
use crate::{
    clients::drs::uri::{AccessPlan, AccessPreferences, DrsResolver},
    utils::{operation::Operation, transport::read_bounded},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    Conformant,
    Missing,
    Invalid,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    NotChecked,
    Available,
    Inaccessible,
    Missing,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    Unverified,
    Verified,
    Mismatch,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpectedChecksum {
    pub algorithm: String,
    pub digest: String,
    pub evidence_source: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactEvidence {
    pub pointer: String,
    pub stable_identity: Option<String>,
    pub access_status: AccessStatus,
    pub integrity_status: IntegrityStatus,
    pub expected: Option<ExpectedChecksum>,
    pub computed_sha256: Option<String>,
    pub detail: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypedOutput {
    #[serde(default)]
    pub access_details_redacted: bool,
    pub value: Option<Value>,
    pub contract_status: ContractStatus,
    pub annotations: Value,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultRecord {
    pub format_version: u32,
    pub run_reference: String,
    pub execution_state: String,
    pub schema_digest: String,
    /// Root scope for retained schema references and scientific/viewer annotations.
    #[serde(default)]
    pub contract_document: Value,
    pub contract_diagnostics: Vec<Diagnostic>,
    pub outputs: BTreeMap<String, TypedOutput>,
    pub artifacts: BTreeMap<String, ArtifactEvidence>,
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
    /// Verify caller-selected bytes. A digest with no expected evidence remains unverified.
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
    /// The access plan is ephemeral and is never stored in the result. No redirects or SDK auth reuse.
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
    /// Metadata for an existing HTML viewer. Consumers decide authorization and presentation.
    pub fn html_viewer_metadata(&self) -> Vec<Value> {
        self.outputs.iter().filter(|(_,v)|v.annotations.get("x-ga4gh-ui-widget").and_then(Value::as_str)==Some("html-viewer")).map(|(key,v)|json!({"key":key,"widget":"html-viewer","contentMediaType":v.annotations.get("contentMediaType"),"value":v.value,"contract_status":v.contract_status})).collect()
    }
}
