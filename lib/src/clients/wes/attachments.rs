//! Streaming WES attachment sources. No request replay and no eager reader buffering.
use super::{models::RunRequest, Run, WES};
use crate::utils::{operation::Operation, transport::BodyLimits};
use reqwest::multipart::{Form, Part};
use std::{collections::HashSet, path::PathBuf, pin::Pin};
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

/// Where the content of a [`StreamingAttachment`] comes from.
pub enum AttachmentSource {
    /// Content in memory.
    Bytes(Vec<u8>),
    /// A file, streamed from disk when the request is sent; its length is
    /// read from its metadata.
    File(PathBuf),
    /// Any reader, streamed when the request is sent. It can be read only
    /// once, so a request with it is never sent twice.
    Reader {
        /// The reader of the content.
        reader: Pin<Box<dyn AsyncRead + Send>>,
        /// The content's length in bytes, if known.
        length: Option<u64>,
    },
}

/// A workflow attachment of a WES run request, whose content can be streamed
/// from a file or reader instead of buffered in memory.
pub struct StreamingAttachment {
    /// The path of the file relative to the attachment root, with `/`
    /// separators; see [`validate_filename`].
    pub filename: String,
    /// Where the content comes from.
    pub source: AttachmentSource,
}
impl StreamingAttachment {
    /// Builds an attachment that streams `reader`, of `length` bytes if known.
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
pub fn validate_filename(filename: &str) -> crate::Result<()> {
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
fn workflow_form(mut request: RunRequest) -> crate::Result<Form> {
    let extra = std::mem::take(&mut request.extra);
    let value = serde_json::to_value(request)?;
    let required = |field: &str| -> crate::Result<String> {
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

pub(crate) fn validate_filenames<'a>(
    filenames: impl Iterator<Item = &'a str>,
) -> crate::Result<()> {
    let mut names = HashSet::new();
    for filename in filenames {
        validate_filename(filename)?;
        if !names.insert(filename) {
            return Err("duplicate workflow attachment filename".into());
        }
    }
    Ok(())
}

/// The parts of a RunWorkflow request that `RunRequest.extra` must not
/// replace: the `RunRequest` fields and the attachments. Keep in step with
/// `workflow_form` and the attachment parts.
pub(crate) const RESERVED_PARTS: [&str; 9] = [
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
    ) -> crate::Result<Run> {
        validate_filenames(
            attachments
                .iter()
                .map(|attachment| attachment.filename.as_str()),
        )?;
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
                Ok::<_, crate::Error>(form)
            })
            .await??;
        // The token provider runs within the operation; running out of time
        // there is not an ambiguous submission, as nothing was sent.
        let request = operation
            .run(
                self.transport()
                    .authorized_request_builder(reqwest::Method::POST, "runs"),
            )
            .await??
            .multipart(form);
        let body = self
            .transport()
            .send_once(request, operation, BodyLimits::default())
            .await?;
        let id =
            crate::utils::acknowledgement::execution_id(&body, "run_id", "WES creation", None)?;
        Ok(Run::new(id, self.transport().clone()))
    }
}
