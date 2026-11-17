//! Optional, revisioned workflow contracts. Preparation and comparison never execute jobs.
//!
//! A [`Profile`] is a JSON Schema that describes a workflow's inputs and,
//! optionally, its outputs, following the GA4GH I/O profile at the pinned
//! revision [`PROFILE_REVISION`]. With a profile, a caller can:
//!
//! - validate a WES [`RunRequest`] and turn it into a [`PreparedInvocation`]
//!   with [`Profile::prepare`], then submit it with
//!   [`PreparedInvocation::submit`] or
//!   [`PreparedInvocation::submit_with_attachments`];
//! - project a run's outputs onto the output contract as a
//!   [`results::ResultRecord`] with [`Profile::project_results`], and add
//!   evidence about output artifacts to it;
//! - compare two result records under a [`compare::ComparisonPolicy`];
//! - check whether the result of an earlier run can be reused, with
//!   [`reuse`].
//!
//! The module needs the crate feature `workflow-contracts`.
pub mod compare;
pub mod results;
pub mod reuse;
use crate::clients::wes::{
    attachments::RESERVED_PARTS, models::RunRequest, Run, WorkflowAttachment, WES,
};
use jsonschema::{Draft, Retrieve, Uri, Validator};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    error::Error,
};
/// The error type of the workflow contract functions: [`crate::Error`],
/// whose message says which check failed.
pub type ContractError = crate::Error;
/// The revision of the GA4GH I/O profile and widget vocabulary schemas that
/// this crate embeds; [`Pins::reference`] pins both to it.
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
/// The schema revisions that a [`Profile`] is checked against.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pins {
    /// The revision of the GA4GH I/O profile meta-schema.
    pub profile_revision: String,
    /// The revision of the widget vocabulary schema.
    pub vocabulary_revision: String,
}
impl Pins {
    /// Returns the pins of the schemas that this crate embeds: both are
    /// [`PROFILE_REVISION`]. [`Profile::from_slice`] accepts only these.
    pub fn reference() -> Self {
        Self {
            profile_revision: PROFILE_REVISION.into(),
            vocabulary_revision: PROFILE_REVISION.into(),
        }
    }
}
/// One JSON Schema validation error.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostic {
    /// The JSON pointer of the value that failed, within the validated
    /// value.
    pub instance_location: String,
    /// The JSON pointer of the schema keyword that failed.
    pub schema_location: String,
    /// A description of the error.
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
/// Returns the SHA-256 of `bytes` as 64 lowercase hex digits.
pub fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
// Explicit recursive sorting also remains stable if a consumer enables serde_json's
// preserve_order feature through Cargo feature unification.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.into_iter().collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k, canonical(v))).collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}
/// Returns the SHA-256 of `value`'s canonical JSON: compact, with the keys
/// of every object sorted, so map iteration order doesn't change it.
fn content_digest(value: &impl Serialize) -> Result<String, ContractError> {
    Ok(digest(&serde_json::to_vec(&canonical(
        serde_json::to_value(value)?,
    ))?))
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
/// A validated, immutable workflow I/O profile: a JSON Schema for a
/// workflow's inputs and, optionally, its outputs.
///
/// It keeps the SHA-256 of the exact bytes it was read from and the
/// [`Pins`] it was checked against. Schema references resolve only to the
/// profile itself and the embedded GA4GH schemas; nothing is fetched.
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
    /// Reads and validates a profile from its JSON.
    ///
    /// # Errors
    ///
    /// Fails if:
    ///
    /// - `pins` isn't [`Pins::reference`];
    /// - `bytes` aren't a JSON object;
    /// - the profile requires a JSON Schema vocabulary other than the
    ///   standard 2020-12 ones;
    /// - its `$schema` is missing or is neither JSON Schema 2020-12 nor the
    ///   GA4GH I/O profile;
    /// - it has no `properties.inputs`;
    /// - it isn't a valid 2020-12 schema, or doesn't conform to the GA4GH
    ///   I/O profile;
    /// - its `$id` isn't an absolute URI without fragment;
    /// - its input or output schema can't be compiled.
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
    /// Returns the SHA-256 of the bytes that the profile was read from, as
    /// 64 lowercase hex digits.
    pub fn schema_digest(&self) -> &str {
        &self.schema_digest
    }
    /// Returns the pins that the profile was checked against.
    pub fn pins(&self) -> &Pins {
        &self.pins
    }
    /// Returns the profile's JSON document, as read.
    pub fn document(&self) -> &Value {
        &self.document
    }
    /// Validates workflow parameters against the input schema
    /// (`properties.inputs`) and returns the errors; empty if they conform.
    pub fn validate_inputs(&self, params: &Value) -> Vec<Diagnostic> {
        diagnostics(&self.input, params)
    }
    /// Validates a run's outputs against the output schema
    /// (`properties.outputs`) and returns the errors; empty if they
    /// conform.
    ///
    /// # Errors
    ///
    /// Fails if the profile has no output schema.
    pub fn validate_outputs(&self, outputs: &Value) -> Result<Vec<Diagnostic>, ContractError> {
        Ok(diagnostics(
            self.output
                .as_ref()
                .ok_or("profile has no output contract")?,
            outputs,
        ))
    }
    /// Returns the part of the profile document at the JSON pointer
    /// `pointer`, such as an output's schema with its annotations, or
    /// `None` if there is none. References in it aren't resolved.
    pub fn annotations(&self, pointer: &str) -> Option<&Value> {
        self.document.pointer(pointer)
    }
    /// Validates a WES run request against the profile and returns it as a
    /// [`PreparedInvocation`], with a
    /// [`fingerprint`](PreparedInvocation::fingerprint) over the request,
    /// the identity, the pins and the profile. Submits nothing.
    ///
    /// # Errors
    ///
    /// Fails if `identity.workflow` is blank; if `workflow_url`,
    /// `workflow_type` or `workflow_type_version` of the request is blank;
    /// if a key of the request's `extra` names a RunWorkflow part, such as
    /// `workflow_params`, `tags` or `workflow_attachment`, which submitting
    /// would reject too; if the request has no
    /// `workflow_params`; or if they don't conform to the input schema,
    /// with the [`Diagnostic`]s as JSON in the message.
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
        // A flattened `extra` key could stand in for, or duplicate, a request
        // field in the fingerprinted JSON; submitting rejects these keys too
        if let Some(key) = request
            .extra
            .keys()
            .find(|k| RESERVED_PARTS.contains(&k.as_str()))
        {
            return Err(format!("request extra field {key} is a reserved request part").into());
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
        prepared.fingerprint = content_digest(&prepared)?;
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
/// Identifies the workflow that a [`PreparedInvocation`] runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvocationIdentity {
    /// A pinned identifier of the workflow, such as a TRS tool version;
    /// not blank.
    pub workflow: String,
    /// The SHA-256 of the primary descriptor, as `sha256:<hex>`. If set,
    /// [`PreparedInvocation::submit_with_attachments`] checks the uploaded
    /// descriptor against it.
    pub descriptor_digest: Option<String>,
}
/// A WES run request that [`Profile::prepare`] validated, ready to submit.
///
/// Its fields are private, so a change requires preparing it again. It
/// can be serialized but not deserialized; a stored plan must go through
/// [`Profile::prepare`] again before it is submitted.
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
    /// Returns the validated run request.
    pub fn request(&self) -> &RunRequest {
        &self.request
    }
    /// Returns the SHA-256 of the prepared invocation's canonical JSON
    /// (compact, keys sorted at every level), computed with the fingerprint
    /// itself empty. The same request, identity, pins and profile always
    /// give the same fingerprint, whatever the order of the entries in the
    /// request's `tags` and `workflow_engine_parameters` maps.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    /// Submits the run request to `wes` with `attachments`, as
    /// [`WES::run_workflow_with_attachments`] does.
    ///
    /// If the identity has a `descriptor_digest`, the attachment whose
    /// file name is the request's `workflow_url` must have that SHA-256.
    /// Only the primary descriptor is checked; dependencies can use
    /// [`crate::clients::bundle::PreparedBundle`] separately.
    ///
    /// # Errors
    ///
    /// Fails if the descriptor digest doesn't start with `sha256:`, if no
    /// attachment is the primary descriptor, if its digest differs, or if
    /// the submission fails.
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
    /// Submits the run request to `wes`, as [`WES::run_workflow`] does.
    /// Doesn't check `descriptor_digest`, as nothing is uploaded.
    ///
    /// # Errors
    ///
    /// Fails if the submission fails.
    pub async fn submit(&self, wes: &WES) -> Result<Run, ContractError> {
        wes.run_workflow(self.request.clone()).await
    }
}
pub(crate) fn pointer_segment(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
