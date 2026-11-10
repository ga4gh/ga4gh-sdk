//! Streaming WES attachment sources. No request replay and no eager reader buffering.
use super::{models::WesRunRequest, Run, WES};
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
fn workflow_form(request: WesRunRequest) -> Result<Form, Box<dyn Error>> {
    let workflow_type = request
        .workflow_type
        .filter(|v| !v.is_empty())
        .ok_or("workflow_type is required")?;
    let version = request
        .workflow_type_version
        .filter(|v| !v.is_empty())
        .ok_or("workflow_type_version is required")?;
    let url = request
        .workflow_url
        .filter(|v| !v.is_empty())
        .ok_or("workflow_url is required")?;
    let params = request
        .workflow_params
        .ok_or("workflow_params is required")?;
    let mut form = Form::new()
        .text("workflow_type", workflow_type)
        .text("workflow_type_version", version)
        .text("workflow_url", url)
        .text("workflow_params", serde_json::to_string(&params)?);
    if let Some(value) = request.tags {
        form = form.text("tags", serde_json::to_string(&value)?);
    }
    if let Some(value) = request.workflow_engine_parameters {
        form = form.text("workflow_engine_parameters", serde_json::to_string(&value)?);
    }
    if let Some(value) = request.workflow_engine {
        form = form.text("workflow_engine", value);
    }
    if let Some(value) = request.workflow_engine_version {
        form = form.text("workflow_engine_version", value);
    }
    Ok(form)
}
impl WES {
    /// Validate all paths before I/O; stream with reqwest backpressure. File handles
    /// are opened once for this attempt. A consumed source is never reopened/replayed.
    /// The Operation includes preparation, upload and response-body consumption.
    pub async fn run_workflow_streaming(
        &self,
        request: WesRunRequest,
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
        let request = self
            .transport
            .request_builder(reqwest::Method::POST, "runs")?
            .multipart(form);
        let body = self
            .transport
            .send_once(request, operation, BodyLimits::default())
            .await?;
        let value: serde_json::Value = serde_json::from_str(&body).map_err(|_| {
            "invalid WES acknowledgement; submission may have succeeded; reconcile before retrying"
        })?;
        let id = value
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or(
                "invalid WES run_id; submission may have succeeded; reconcile before retrying",
            )?;
        Ok(Run::new(id.into(), self.transport.clone()))
    }
}
