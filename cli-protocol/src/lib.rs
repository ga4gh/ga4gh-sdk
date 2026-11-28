//! JSON messages for `ga4gh.cli.commands`, negotiated revision 1.
use base64::{engine::general_purpose::STANDARD, Engine};
use ga4gh_sdk::clients::wes::{models::RunRequest, WorkflowAttachment};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Write},
};

pub const CAPABILITY: &str = "ga4gh.cli.commands";
pub const MAX_MESSAGE: usize = 64 << 20;

/// Serialize a complete UTF-8 message, refusing oversized buffers before copying them.
pub fn encode(value: &impl Serialize) -> serde_json::Result<Vec<u8>> {
    struct Buffer(Vec<u8>);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > MAX_MESSAGE - self.0.len() {
                return Err(io::Error::other("CLI extension message exceeds 64 MiB"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer(Vec::new());
    serde_json::to_writer(&mut buffer, value)?;
    Ok(buffer.0)
}

#[derive(Serialize, Deserialize)]
pub struct Invocation {
    pub group: String,
    pub command: String,
    pub profile: Option<String>,
    pub arguments: BTreeMap<String, Vec<String>>,
}

#[derive(Serialize, Deserialize)]
pub struct Attachment {
    filename: String,
    base64: String,
}

impl From<WorkflowAttachment> for Attachment {
    fn from(file: WorkflowAttachment) -> Self {
        Self {
            filename: file.filename,
            base64: STANDARD.encode(file.content),
        }
    }
}

impl TryFrom<Attachment> for WorkflowAttachment {
    type Error = base64::DecodeError;

    fn try_from(file: Attachment) -> Result<Self, Self::Error> {
        Ok(Self {
            filename: file.filename,
            content: STANDARD.decode(file.base64)?,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reply {
    Complete(Output),
    WesSubmit {
        request: Box<RunRequest>,
        attachments: Vec<Attachment>,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubmissionOutcome {
    Submitted { run_id: String },
    Failed { message: String },
}

#[derive(Serialize, Deserialize)]
pub struct Output {
    pub stdout: String,
    #[serde(default)]
    pub json_stdout: bool,
    pub stderr: String,
    pub exit_status: u8,
}

impl Output {
    pub fn failure(message: impl ToString) -> Self {
        Self {
            stdout: String::new(),
            json_stdout: false,
            stderr: format!("{}\n", message.to_string()),
            exit_status: 1,
        }
    }
}
