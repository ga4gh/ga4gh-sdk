//! Streaming WES attachment sources. No request replay and no eager reader buffering.
use super::{models::RunRequest, Run, WES};
use crate::utils::{operation::Operation, transport::BodyLimits};
use reqwest::multipart::{Form, Part};
use std::{collections::HashSet, error::Error, path::PathBuf, pin::Pin};
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

pub enum AttachmentSource {
    Bytes(Vec<u8>),
    File(PathBuf),
    Reader {
        reader: Pin<Box<dyn AsyncRead + Send>>,
        length: Option<u64>,
    },
}
pub struct StreamingAttachment {
    pub filename: String,
    pub source: AttachmentSource,
}
impl StreamingAttachment {
    pub fn reader(
        filename: String,
        reader: impl AsyncRead + Send + 'static,
        length: Option<u64>,
    ) -> Self {
        Self {
            filename,
            source: AttachmentSource::Reader {
                reader: Box::pin(reader),
                length,
            },
        }
    }
}
/// WES paths are relative to the attachment root, with '/' separators.
pub fn validate_filename(filename: &str) -> Result<(), Box<dyn Error>> {
    if filename.is_empty()
        || filename.starts_with('/')
        || filename.contains('\\')
        || filename.contains(':')
        || filename.chars().any(char::is_control)
        || filename
            .split('/')
            .any(|part| matches!(part, "" | "." | ".."))
    {
        return Err("workflow attachment filename must be a safe relative path".into());
    }
    Ok(())
}
fn workflow_form(mut request: RunRequest) -> Result<Form, Box<dyn Error>> {
    let extra = std::mem::take(&mut request.extra);
    let value = serde_json::to_value(request)?;
    let required = |field: &str| -> Result<String, Box<dyn Error>> {
        value
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| format!("{field} is required").into())
    };
    let mut form = Form::new()
        .text("workflow_type", required("workflow_type")?)
        .text("workflow_type_version", required("workflow_type_version")?)
        .text("workflow_url", required("workflow_url")?)
        .text(
            "workflow_params",
            serde_json::to_string(
                value
                    .get("workflow_params")
                    .filter(|v| !v.is_null())
                    .ok_or("workflow_params is required")?,
            )?,
        );
    for field in ["tags", "workflow_engine_parameters"] {
        if let Some(value) = value.get(field).filter(|v| !v.is_null()) {
            form = form.text(field, serde_json::to_string(value)?);
        }
    }
    for field in ["workflow_engine", "workflow_engine_version"] {
        if let Some(value) = value.get(field).and_then(serde_json::Value::as_str) {
            form = form.text(field, value.to_owned());
        }
    }
    // Fields that the spec doesn't define go as further parts: strings as
    // they are, other values as JSON, like `tags`.
    for (field, value) in extra {
        if RESERVED_PARTS.contains(&field.as_str()) {
            return Err(
                format!("extra field {field} would replace a RunWorkflow form part").into(),
            );
        }
        let text = match value {
            serde_json::Value::String(text) => text,
            value => serde_json::to_string(&value)?,
        };
        form = form.text(field, text);
    }
    Ok(form)
}

/// The parts of a RunWorkflow request that `RunRequest.extra` must not
/// replace: the `RunRequest` fields and the attachments. Keep in step with
/// `workflow_form` and the attachment parts.
pub(super) const RESERVED_PARTS: [&str; 9] = [
    "workflow_params",
    "workflow_type",
    "workflow_type_version",
    "tags",
    "workflow_engine_parameters",
    "workflow_engine",
    "workflow_engine_version",
    "workflow_url",
    "workflow_attachment",
];

impl WES {
    /// Validate all paths before I/O; stream with reqwest backpressure. File handles
    /// are opened once for this attempt. A consumed source is never reopened/replayed.
    /// The Operation includes preparation, upload and response-body consumption.
    pub async fn run_workflow_streaming(
        &self,
        request: RunRequest,
        attachments: Vec<StreamingAttachment>,
        operation: &Operation,
    ) -> Result<Run, Box<dyn Error>> {
        self.check()?;
        let mut names = HashSet::new();
        for attachment in &attachments {
            validate_filename(&attachment.filename)?;
            if !names.insert(&attachment.filename) {
                return Err("duplicate workflow attachment filename".into());
            }
        }
        let mut form = workflow_form(request)?;
        let form = operation
            .run(async {
                for attachment in attachments {
                    let part = match attachment.source {
                        AttachmentSource::Bytes(bytes) => Part::bytes(bytes),
                        AttachmentSource::File(path) => {
                            let file = tokio::fs::File::open(path).await?;
                            let length = file.metadata().await?.len();
                            Part::stream_with_length(
                                reqwest::Body::wrap_stream(ReaderStream::new(file)),
                                length,
                            )
                        }
                        AttachmentSource::Reader { reader, length } => {
                            let body = reqwest::Body::wrap_stream(ReaderStream::new(reader));
                            match length {
                                Some(length) => Part::stream_with_length(body, length),
                                None => Part::stream(body),
                            }
                        }
                    };
                    form = form.part("workflow_attachment", part.file_name(attachment.filename));
                }
                Ok::<_, Box<dyn Error>>(form)
            })
            .await??;
        // The token provider runs within the operation; running out of time
        // there is not an ambiguous submission, as nothing was sent.
        let request = operation
            .run(
                self.transport
                    .authorized_request_builder(reqwest::Method::POST, "runs"),
            )
            .await??
            .multipart(form);
        let body = self
            .transport
            .send_once(request, operation, BodyLimits::default())
            .await?;
        let id =
            crate::utils::acknowledgement::execution_id(&body, "run_id", "WES creation", None)?;
        Ok(Run::new(id, self.transport.clone()))
    }
}
