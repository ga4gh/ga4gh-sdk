//! Typed client for `ga4gh.workflow` revision 1. No installation or WES discovery is implicit.
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
    pub outputs: BTreeMap<String, Value>,
    pub artifacts: BTreeMap<String, Value>,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
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
    pub fn prepare_file(
        &self,
        profile: &Path,
        pins: &Pins,
        request: &RunRequest,
        identity: &Identity,
    ) -> Result<Prepared> {
        self.prepare(&std::fs::read(profile)?, pins, request, identity)
    }
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

pub struct Prepared {
    client: Workflow,
    key: String,
    record: PlanRecord,
}
impl Prepared {
    pub fn record(&self) -> &PlanRecord {
        &self.record
    }
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
    /// Consumes the preparation; cancellation never causes an automatic retry.
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
        let files = action
            .attachments
            .into_iter()
            .map(WorkflowAttachment::try_from)
            .collect::<std::result::Result<_, _>>()
            .map_err(|_| Error::Protocol)?;
        match wes
            .run_workflow_with_attachments(action.request, files)
            .await
        {
            Err(cause) => {
                let _ = self.client.call::<bool>(
                    "finish_submit",
                    &json!({"state_ref":self.key,"outcome":"failed"}),
                    "ok",
                );
                Err(Error::Wes(cause))
            }
            Ok(run) => {
                let reply = self.client.call::<Receipt>(
                    "finish_submit",
                    &json!({"state_ref":self.key,"outcome":{"submitted":{"run_id":run.id}}}),
                    "ok",
                );
                match reply {
                    Ok(receipt)
                        if receipt.run_id == run.id
                            && receipt.invocation.fingerprint == self.record.fingerprint =>
                    {
                        self.key.clear();
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
