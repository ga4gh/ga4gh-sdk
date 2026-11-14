//! Optional, revisioned workflow contracts. Preparation and comparison never execute jobs.
pub mod compare;
pub mod results;
pub mod reuse;
use crate::clients::wes::{models::RunRequest, Run, WorkflowAttachment, WES};
use jsonschema::{Draft, Retrieve, Uri, Validator};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, error::Error};
pub type ContractError = Box<dyn Error>;
pub const PROFILE_REVISION: &str = "45d41324353cb4900fdd999c307c2e36a3f2a410";
const FRAGMENT: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b' ')
    .add(b'%')
    .add(b'#')
    .add(b'?')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    .add(b'\\');
const DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";
const PROFILE_URI: &str = "https://raw.githubusercontent.com/uniqueg/ga4gh-io-schema/main/schemas/ga4gh-io-profile.schema.json";
const WIDGET_URI: &str = "https://raw.githubusercontent.com/uniqueg/ga4gh-io-schema/main/schemas/vocabularies/ui-widgets.schema.json";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pins {
    pub profile_revision: String,
    pub vocabulary_revision: String,
}
impl Pins {
    pub fn reference() -> Self {
        Self {
            profile_revision: PROFILE_REVISION.into(),
            vocabulary_revision: PROFILE_REVISION.into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    pub instance_location: String,
    pub schema_location: String,
    pub message: String,
}
#[derive(Clone)]
struct LocalResources(HashMap<String, Value>);
impl Retrieve for LocalResources {
    fn retrieve(&self, uri: &Uri<String>) -> Result<Value, Box<dyn Error + Send + Sync>> {
        self.0
            .get(uri.as_str())
            .cloned()
            .ok_or_else(|| format!("unresolved local schema reference: {uri}").into())
    }
}
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn vocabulary_check(value: &Value) -> Result<(), ContractError> {
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if let Some(vocab) = object.get("$vocabulary").and_then(Value::as_object) {
        for (uri, required) in vocab {
            if required == true
                && ![
                    "core",
                    "applicator",
                    "unevaluated",
                    "validation",
                    "meta-data",
                    "format-annotation",
                    "format-assertion",
                    "content",
                ]
                .iter()
                .any(|name| uri == &format!("https://json-schema.org/draft/2020-12/vocab/{name}"))
            {
                return Err(format!("unsupported required vocabulary: {uri}").into());
            }
        }
    }
    // Traverse schema-bearing keywords only: const/default/examples are data.
    for key in [
        "additionalProperties",
        "contains",
        "not",
        "if",
        "then",
        "else",
        "propertyNames",
        "unevaluatedItems",
        "unevaluatedProperties",
        "items",
        "contentSchema",
    ] {
        if let Some(schema) = object.get(key) {
            vocabulary_check(schema)?;
        }
    }
    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(schemas) = object.get(key).and_then(Value::as_array) {
            for schema in schemas {
                vocabulary_check(schema)?;
            }
        }
    }
    for key in [
        "properties",
        "patternProperties",
        "$defs",
        "dependentSchemas",
    ] {
        if let Some(schemas) = object.get(key).and_then(Value::as_object) {
            for schema in schemas.values() {
                vocabulary_check(schema)?;
            }
        }
    }
    Ok(())
}
/// Immutable validated schema with exact document digest and explicit draft pins.
pub struct Profile {
    document: Value,
    schema_digest: String,
    pins: Pins,
    resources: LocalResources,
    root_uri: String,
    input: Validator,
    output: Option<Validator>,
}
impl Profile {
    pub fn from_slice(bytes: &[u8], pins: Pins) -> Result<Self, ContractError> {
        if pins != Pins::reference() {
            return Err(
                "unsupported profile/vocabulary revision; supply the pinned reference revision"
                    .into(),
            );
        }
        let document: Value = serde_json::from_slice(bytes)?;
        vocabulary_check(&document)?;
        if !document.is_object() {
            return Err("workflow profile must be an object".into());
        }
        let declared = document
            .get("$schema")
            .and_then(Value::as_str)
            .ok_or("profile must declare $schema")?;
        if ![DRAFT, PROFILE_URI].contains(&declared) {
            return Err("unsupported schema dialect".into());
        }
        if document.pointer("/properties/inputs").is_none() {
            return Err("profile requires properties.inputs".into());
        }
        let mut runtime = document.clone();
        runtime["$schema"] = json!(DRAFT);
        jsonschema::draft202012::meta::validate(&runtime)
            .map_err(|e| format!("invalid profile schema: {e}"))?;
        let root_uri = runtime
            .get("$id")
            .and_then(Value::as_str)
            .unwrap_or("urn:ga4gh:supplied-profile")
            .to_owned();
        if url::Url::parse(&root_uri).is_err() || root_uri.contains('#') {
            return Err("profile $id must be an absolute URI without fragment".into());
        }
        runtime["$id"] = json!(root_uri);
        let meta: Value =
            serde_json::from_str(include_str!("schemas/ga4gh-io-profile.schema.json"))?;
        let widgets: Value = serde_json::from_str(include_str!("schemas/ui-widgets.schema.json"))?;
        let resources = LocalResources(HashMap::from([
            (root_uri.clone(), runtime),
            (PROFILE_URI.into(), meta.clone()),
            (WIDGET_URI.into(), widgets),
        ]));
        let meta_validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .with_retriever(resources.clone())
            .build(&meta)
            .map_err(|e| e.to_string())?;
        meta_validator
            .validate(&document)
            .map_err(|e| format!("invalid I/O profile: {e}"))?;
        let compile = |part: &str| -> Result<Validator, ContractError> {
            Ok(jsonschema::options()
                .with_draft(Draft::Draft202012)
                .should_validate_formats(true)
                .with_retriever(resources.clone())
                .build(&json!({"$schema":DRAFT,"$ref":format!("{root_uri}#/properties/{part}")}))
                .map_err(|e| e.to_string())?)
        };
        let input = compile("inputs")?;
        let output = if document.pointer("/properties/outputs").is_some() {
            Some(compile("outputs")?)
        } else {
            None
        };
        Ok(Self {
            document,
            schema_digest: digest(bytes),
            pins,
            resources,
            root_uri,
            input,
            output,
        })
    }
    pub fn schema_digest(&self) -> &str {
        &self.schema_digest
    }
    pub fn pins(&self) -> &Pins {
        &self.pins
    }
    pub fn document(&self) -> &Value {
        &self.document
    }
    pub fn validate_inputs(&self, params: &Value) -> Vec<Diagnostic> {
        diagnostics(&self.input, params)
    }
    pub fn validate_outputs(&self, outputs: &Value) -> Result<Vec<Diagnostic>, ContractError> {
        Ok(diagnostics(
            self.output
                .as_ref()
                .ok_or("profile has no output contract")?,
            outputs,
        ))
    }
    /// Select a subschema by JSON pointer while preserving the original root scope.
    pub fn annotations(&self, pointer: &str) -> Option<&Value> {
        self.document.pointer(pointer)
    }
    pub fn prepare(
        &self,
        request: RunRequest,
        identity: InvocationIdentity,
    ) -> Result<PreparedInvocation, ContractError> {
        if identity.workflow.trim().is_empty() {
            return Err("workflow identity must be pinned and nonempty".into());
        }
        for field in [
            &request.workflow_url,
            &request.workflow_type,
            &request.workflow_type_version,
        ] {
            if field.trim().is_empty() {
                return Err(
                    "request requires workflow_url, workflow_type and workflow_type_version".into(),
                );
            }
        }
        let errors = self.validate_inputs(
            request
                .workflow_params
                .as_ref()
                .ok_or("request requires workflow_params")?,
        );
        if !errors.is_empty() {
            return Err(serde_json::to_string(&errors)?.into());
        }
        let mut prepared = PreparedInvocation {
            format_version: 1,
            identity,
            pins: self.pins.clone(),
            schema_digest: self.schema_digest.clone(),
            request,
            expected_outputs: self.document.pointer("/properties/outputs").cloned(),
            profile_document: self.document.clone(),
            fingerprint: String::new(),
        };
        prepared.fingerprint = digest(&serde_json::to_vec(&prepared)?);
        Ok(prepared)
    }
    pub(crate) fn validate_at(
        &self,
        pointer: &str,
        value: &Value,
    ) -> Result<Vec<Diagnostic>, ContractError> {
        let validator = jsonschema::options()
            .with_draft(Draft::Draft202012)
            .should_validate_formats(true)
            .with_retriever(self.resources.clone())
            .build(&json!({"$schema":DRAFT,"$ref":format!("{}#{}",self.root_uri,percent_encoding::utf8_percent_encode(pointer,FRAGMENT))}))
            .map_err(|e| e.to_string())?;
        Ok(diagnostics(&validator, value))
    }
}
fn diagnostics(validator: &Validator, value: &Value) -> Vec<Diagnostic> {
    validator
        .iter_errors(value)
        .map(|e| Diagnostic {
            instance_location: e.instance_path().to_string(),
            schema_location: e.schema_path().to_string(),
            message: e.to_string(),
        })
        .collect()
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationIdentity {
    pub workflow: String,
    pub descriptor_digest: Option<String>,
}
/// Fields remain private: alterations require preparing/validating again. Deserialized plans must be revalidated via Profile::prepare before submission.
#[derive(Debug, Serialize)]
pub struct PreparedInvocation {
    format_version: u32,
    identity: InvocationIdentity,
    pins: Pins,
    schema_digest: String,
    request: RunRequest,
    expected_outputs: Option<Value>,
    profile_document: Value,
    fingerprint: String,
}
impl PreparedInvocation {
    pub fn request(&self) -> &RunRequest {
        &self.request
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    /// Explicit attachment submission binds the declared primary descriptor digest
    /// to the actual uploaded bytes. Dependencies can use PreparedBundle separately.
    pub async fn submit_with_attachments(
        &self,
        wes: &WES,
        attachments: Vec<WorkflowAttachment>,
    ) -> Result<Run, ContractError> {
        if let Some(expected) = &self.identity.descriptor_digest {
            let expected = expected
                .strip_prefix("sha256:")
                .ok_or("descriptor digest must use sha256:")?;
            let primary = attachments
                .iter()
                .find(|a| a.filename == self.request.workflow_url)
                .ok_or("primary descriptor attachment missing")?;
            if !digest(&primary.content).eq_ignore_ascii_case(expected) {
                return Err("primary descriptor digest mismatch; prepare a new invocation".into());
            }
        }
        wes.run_workflow_with_attachments(self.request.clone(), attachments)
            .await
    }
    pub async fn submit(&self, wes: &WES) -> Result<Run, ContractError> {
        wes.run_workflow(self.request.clone()).await
    }
}
pub(crate) fn pointer_segment(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
