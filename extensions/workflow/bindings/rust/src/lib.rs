//! Typed client for `ga4gh.workflow` revision 1. No installation or WES discovery is implicit.
//!
//! ```no_run
//! use ga4gh_sdk::{clients::wes::models::RunRequest, utils::extension_manager::ExtensionManager};
//! use ga4gh_workflow::{Error, Identity, Pins, Workflow};
//!
//! # unsafe fn inspect(manager: &ExtensionManager, profile: &[u8], pins: &Pins, request: &RunRequest) -> Result<(), Error> {
//! let client = unsafe { Workflow::open(manager)? }; // The application trusts this installation.
//! let identity = Identity { workflow: "public-qc@1".into(), descriptor_digest: None };
//! let plan = client.prepare(profile, pins, request, &identity)?;
//! println!("{}", plan.record().fingerprint);
//! plan.release()?;
//! # Ok(()) }
//! ```
use base64::{engine::general_purpose::STANDARD, Engine};
use ga4gh_cli_protocol::{encode, Attachment, MAX_MESSAGE};
use ga4gh_sdk::{
    clients::wes::{models::RunRequest, WorkflowAttachment, WES},
    utils::{
        extension::{ExtensionError, ExtensionInstance},
        extension_manager::ExtensionManager,
    },
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, path::Path};

const CAP: &str = "ga4gh.workflow";
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pins {
    pub profile_revision: String,
    pub vocabulary_revision: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    pub workflow: String,
    pub descriptor_digest: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diagnostic {
    pub instance_location: String,
    pub schema_location: String,
    pub message: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowFailure {
    pub code: String,
    pub message: String,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Extension(#[from] ExtensionError),
    #[error("Workflow operation failed: {}", .0.code)]
    Workflow(WorkflowFailure),
    #[error("Invalid workflow protocol message or message limit exceeded")]
    Protocol,
    #[error(transparent)]
    File(#[from] std::io::Error),
    #[error(transparent)]
    Wes(#[from] ga4gh_sdk::Error),
    #[error("WES accepted run {run_id:?}; completion failed, do not resubmit")]
    Accepted { run_id: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanRecord {
    pub format_version: u32,
    pub profile_document: Value,
    pub expected_outputs: Option<Value>,
    pub fingerprint: String,
    pub schema_digest: String,
    pub request: RunRequest,
    pub pins: Pins,
    pub identity: Identity,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultRecord {
    pub run_reference: String,
    pub execution_state: String,
    pub schema_digest: String,
    pub format_version: u32,
    pub outputs: BTreeMap<String, TypedOutput>,
    pub artifacts: BTreeMap<String, Evidence>,
    pub contract_document: Value,
    pub contract_diagnostics: Vec<Diagnostic>,
    pub context: BTreeMap<String, String>,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypedOutput {
    pub access_details_redacted: bool,
    pub value: Option<Value>,
    pub contract_status: String,
    pub annotations: Value,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub pointer: String,
    pub stable_identity: Option<String>,
    pub access_status: String,
    pub integrity_status: String,
    pub expected: Option<Checksum>,
    pub computed_sha256: Option<String>,
    pub detail: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Checksum {
    pub algorithm: String,
    pub digest: String,
    pub evidence_source: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    pub id: String,
    pub version: u32,
    pub outputs: BTreeMap<String, Value>,
    pub required_context: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    EquivalentUnderPolicy,
    Different,
    Inconclusive,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub policy: Policy,
    pub left: String,
    pub right: String,
    pub context_drift: BTreeMap<String, (Option<String>, Option<String>)>,
    pub missing_context: Vec<String>,
    pub outcome: Outcome,
    pub outputs: BTreeMap<String, Outcome>,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}
#[derive(Debug, Deserialize)]
pub struct Receipt {
    pub run_id: String,
    pub invocation: PlanRecord,
}

#[derive(Clone)]
pub struct Workflow(ExtensionInstance);
impl Workflow {
    /// Open the enabled provider and negotiate revision 1 before creating an instance.
    ///
    /// # Safety
    /// The installed provider must be trusted native code implementing the extension ABI.
    pub unsafe fn open(manager: &ExtensionManager) -> Result<Self> {
        let resolved = manager.resolve(
            "ga4gh-workflow-extension",
            None,
            None,
            &[(CAP.to_owned(), vec![1])].into(),
        )?;
        Ok(Self(unsafe { manager.create_instance(&resolved, b"{}")? }))
    }
    fn call<T: DeserializeOwned>(&self, op: &str, input: &Value, field: &str) -> Result<T> {
        let data = encode(input).map_err(|_| Error::Protocol)?;
        let bytes = self.0.invoke_bounded(CAP, op, &data, MAX_MESSAGE)?;
        let Value::Object(mut envelope) =
            serde_json::from_slice(&bytes).map_err(|_| Error::Protocol)?
        else {
            return Err(Error::Protocol);
        };
        if envelope.len() != 1 {
            return Err(Error::Protocol);
        }
        if let Some(error) = envelope.remove("error") {
            return Err(Error::Workflow(
                serde_json::from_value(error).map_err(|_| Error::Protocol)?,
            ));
        }
        serde_json::from_value(envelope.remove(field).ok_or(Error::Protocol)?)
            .map_err(|_| Error::Protocol)
    }
    /// Validate original UTF-8 profile bytes, revision pins, WES request and identity without I/O.
    /// Returns an instance-owned preparation. Invalid inputs return [`Error::Workflow`]; invalid
    /// UTF-8 or an oversized envelope returns [`Error::Protocol`]. A live plan must be released first.
    pub fn prepare(
        &self,
        profile: &[u8],
        pins: &Pins,
        request: &RunRequest,
        identity: &Identity,
    ) -> Result<Prepared> {
        #[derive(Deserialize)]
        struct Live {
            state_ref: String,
            record: PlanRecord,
        }
        let text = std::str::from_utf8(profile).map_err(|_| Error::Protocol)?;
        let input = json!({"profile_utf8":text,"pins":pins,"request":request,"identity":identity});
        let live: Live = self.call("prepare", &input, "prepared")?;
        Ok(Prepared {
            client: self.clone(),
            key: live.state_ref,
            record: live.record,
        })
    }
    /// Read the profile without reformatting it and call [`Self::prepare`]. A read failure is
    /// [`Error::File`]; validation and protocol errors are the same as for the in-memory call.
    pub fn prepare_file(
        &self,
        profile: &Path,
        pins: &Pins,
        request: &RunRequest,
        identity: &Identity,
    ) -> Result<Prepared> {
        self.prepare(&std::fs::read(profile)?, pins, request, identity)
    }
    /// Project the supplied run reference, state and output values under the pinned profile.
    /// No WES call is made. An invalid profile returns a workflow error; delivery diagnostics
    /// on individual outputs are retained in a successful record.
    pub fn results(
        &self,
        profile: &[u8],
        pins: &Pins,
        run_reference: &str,
        state: &str,
        outputs: &Value,
    ) -> Result<ResultRecord> {
        let text = std::str::from_utf8(profile).map_err(|_| Error::Protocol)?;
        let input = json!({"profile_utf8":text,"pins":pins,"run_reference":run_reference,"state":state,"outputs":outputs});
        self.call("results", &input, "ok")
    }
    /// Return a new record with evidence for `pointer` using exactly `bytes` and the optional
    /// trusted checksum. Invalid checksum metadata returns a workflow error; a checksum mismatch
    /// is an integrity finding in the record, not a transport failure.
    pub fn verify(
        &self,
        record: &ResultRecord,
        pointer: &str,
        bytes: &[u8],
        expected: Option<&Checksum>,
    ) -> Result<ResultRecord> {
        let input = json!({"record":record,"pointer":pointer,"base64":STANDARD.encode(bytes),"expected":expected});
        self.call("verify", &input, "ok")
    }
    /// Compare two records under the supplied versioned policy. An invalid policy or unsupported
    /// record version is a workflow error; missing evidence can yield an inconclusive report.
    pub fn compare(
        &self,
        policy: &Policy,
        left: &ResultRecord,
        right: &ResultRecord,
    ) -> Result<Report> {
        self.call(
            "compare",
            &json!({"policy":policy,"left":left,"right":right}),
            "ok",
        )
    }
}

/// One validated preparation retaining its exact native instance. Dropping it releases unused state.
pub struct Prepared {
    client: Workflow,
    key: String,
    record: PlanRecord,
}
impl Prepared {
    /// Borrow the exportable record without invoking the provider. It is not a submission handle.
    pub fn record(&self) -> &PlanRecord {
        &self.record
    }
    /// Consume the preparation without submitting. Unlike drop, report native or protocol failures.
    pub fn release(mut self) -> Result<()> {
        let released: bool =
            self.client
                .call("release", &json!({"state_ref":self.key}), "released")?;
        if !released {
            return Err(Error::Protocol);
        }
        self.key.clear();
        Ok(())
    }
    /// Consume the preparation, validate the attachments, then send the approved bytes once with `wes`.
    /// Validation fails before submission. [`Error::Wes`] retains the actual submission failure;
    /// [`Error::Accepted`] names the accepted run if the provider cannot complete. Cancellation
    /// after dispatch may leave a remote run and never causes an automatic retry.
    pub async fn submit(
        mut self,
        wes: &WES,
        attachments: Vec<WorkflowAttachment>,
    ) -> Result<Receipt> {
        #[derive(Deserialize)]
        struct Action {
            request: RunRequest,
            attachments: Vec<Attachment>,
        }
        let files: Vec<_> = attachments.into_iter().map(Attachment::from).collect();
        let input = json!({"state_ref":self.key,"attachments":files});
        let action: Action = self.client.call("begin_submit", &input, "ok")?;
        let issued_key = std::mem::take(&mut self.key);
        let files = action
            .attachments
            .into_iter()
            .map(WorkflowAttachment::try_from)
            .collect::<std::result::Result<_, _>>();
        let Ok(files) = files else {
            let _ = self.client.call::<bool>(
                "finish_submit",
                &json!({"state_ref":issued_key,"outcome":"failed"}),
                "ok",
            );
            return Err(Error::Protocol);
        };
        match wes
            .run_workflow_with_attachments(action.request, files)
            .await
        {
            Err(cause) => {
                let _ = self.client.call::<bool>(
                    "finish_submit",
                    &json!({"state_ref":issued_key,"outcome":"failed"}),
                    "ok",
                );
                Err(Error::Wes(cause))
            }
            Ok(run) => {
                let reply = self.client.call::<Receipt>(
                    "finish_submit",
                    &json!({"state_ref":issued_key,"outcome":{"submitted":{"run_id":run.id}}}),
                    "ok",
                );
                match reply {
                    Ok(receipt)
                        if receipt.run_id == run.id
                            && receipt.invocation.fingerprint == self.record.fingerprint =>
                    {
                        Ok(receipt)
                    }
                    _ => Err(Error::Accepted { run_id: run.id }),
                }
            }
        }
    }
}
impl Drop for Prepared {
    fn drop(&mut self) {
        if !self.key.is_empty() {
            let _ = self
                .client
                .call::<bool>("release", &json!({"state_ref":self.key}), "released");
        }
    }
}
