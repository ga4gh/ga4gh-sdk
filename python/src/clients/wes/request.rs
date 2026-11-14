//! `RunRequest`: a run to submit, and the conversion of the `attachments` argument of
//! `WES.run()`.

use crate::utils::convert::{check_extra_args, os_error, read_file, to_json, to_python};
use crate::utils::json::field;
use ga4gh_sdk::clients::wes::attachments::{
    validate_filename, AttachmentSource, StreamingAttachment,
};
use ga4gh_sdk::clients::wes::models::RunRequest;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyMapping, PyString, PyTuple, PyType};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A WES run request: a workflow to run, with its parameters.
///
/// `json()` and `to_dict()` return the request's JSON as given or received, including fields
/// that the WES models don't know. `WES.run()` sends the fields of the WES `RunRequest` form,
/// and fields that the WES models don't know as further form parts: strings as they are, other
/// values as JSON.
#[pyclass(name = "RunRequest", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyRunRequest {
    /// The request parsed into the WES model; derived from `raw`.
    pub inner: RunRequest,
    /// The request's JSON as given or received. Build both fields with `from_value` to keep
    /// them in step.
    pub raw: Value,
}

impl PyRunRequest {
    pub(super) fn parse(raw: Value) -> Result<Self, String> {
        if !raw.is_object() {
            return Err("not a JSON object".to_owned());
        }
        if raw
            .get("workflow_params")
            .is_some_and(|params| !params.is_null() && !params.is_object())
        {
            return Err("workflow_params must be a JSON object".to_owned());
        }
        let inner = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
        Ok(PyRunRequest { inner, raw })
    }

    /// Builds a request from a caller's JSON, keeping the JSON. Errors are `ValueError`s.
    pub fn from_value(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(|e| PyValueError::new_err(format!("Invalid run request: {e}")))
    }

    /// Builds a request from a `RunRequest`, a JSON-serialisable object or JSON text.
    pub fn from_python(request: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(request) = request.downcast::<PyRunRequest>() {
            return Ok(request.borrow().clone());
        }
        if let Ok(mapping) = request.downcast::<PyMapping>() {
            if !request.is_instance_of::<PyDict>() {
                // `json.dumps` takes only `dict`s, so copy other mappings into one.
                let dict = PyDict::new_bound(request.py());
                dict.update(mapping)?;
                return Self::from_value(to_json(&dict, "run request")?);
            }
        }
        Self::from_value(to_json(request, "run request")?)
    }

    /// Reads a request from a JSON file.
    pub fn from_path(py: Python<'_>, path: &Path) -> PyResult<Self> {
        let text = read_file(py, path)?;
        let raw = serde_json::from_str(&text).map_err(|e| {
            PyValueError::new_err(format!(
                "Invalid run request JSON in {}: {e}",
                path.display()
            ))
        })?;
        Self::from_value(raw)
    }

    /// Checks the fields that `WES::run_workflow` requires, and that no unknown field is
    /// named `workflow_attachment`, which only the `attachments` argument may send.
    pub(super) fn check_complete(&self) -> PyResult<()> {
        let request = &self.inner;
        for (name, value) in [
            ("workflow_type", &request.workflow_type),
            ("workflow_type_version", &request.workflow_type_version),
            ("workflow_url", &request.workflow_url),
        ] {
            if value.is_empty() {
                return Err(PyValueError::new_err(format!(
                    "Invalid run request: {name} is required"
                )));
            }
        }
        match request.workflow_params.as_ref() {
            Some(Value::Object(_)) => {}
            None | Some(Value::Null) => {
                return Err(PyValueError::new_err(
                    "Invalid run request: workflow_params is required",
                ))
            }
            Some(_) => {
                return Err(PyValueError::new_err(
                    "Invalid run request: workflow_params must be a JSON object",
                ))
            }
        }
        // Unknown fields go as further form parts, but the SDK refuses one that would
        // add `workflow_attachment` parts; say so before the request, with the argument
        // that takes files. The SDK's other reserved parts (`RESERVED_PARTS`) are model
        // fields and never reach `extra`; keep this check in step with that list.
        if request.extra.contains_key("workflow_attachment") {
            return Err(PyValueError::new_err(
                "Invalid run request: workflow_attachment is not a request field; \
                 pass files with the attachments argument of WES.run()",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl PyRunRequest {
    /// `request` is a `dict` (or other JSON-serialisable object), JSON text or a `RunRequest`.
    #[new]
    #[classmethod]
    #[pyo3(signature = (request, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        request: &Bound<'_, PyAny>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::from_python(request)
    }

    /// Builds a request from a mapping, such as the result of `to_dict()`.
    ///
    /// Like `from_json` and `from_file`, it calls `cls(request)`, so a subclass's further
    /// constructor arguments need defaults.
    #[classmethod]
    fn from_dict<'py>(
        cls: &Bound<'py, PyType>,
        request: &Bound<'py, PyMapping>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new_bound(cls.py());
        dict.update(request)?;
        cls.call1((dict,))
    }

    /// Builds a request from JSON text, such as the result of `json()`.
    #[classmethod]
    fn from_json<'py>(
        cls: &Bound<'py, PyType>,
        text: &Bound<'py, PyString>,
    ) -> PyResult<Bound<'py, PyAny>> {
        cls.call1((text,))
    }

    /// Reads a request from a JSON file. `path` is a `str` or `os.PathLike`.
    #[classmethod]
    fn from_file<'py>(cls: &Bound<'py, PyType>, path: PathBuf) -> PyResult<Bound<'py, PyAny>> {
        let request = Self::from_path(cls.py(), &path)?;
        cls.call1((to_python(cls.py(), &request.raw)?,))
    }

    /// Lets `pickle` and `copy` rebuild the request as `type(request)(request.to_dict())`,
    /// with the attributes that a subclass instance holds in `__dict__`.
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyType>, (PyObject,), Option<Bound<'py, PyAny>>)> {
        let raw = to_python(slf.py(), &slf.borrow().raw)?;
        let state = slf.getattr("__dict__").ok();
        Ok((slf.get_type(), (raw,), state))
    }

    #[getter]
    fn workflow_type(&self) -> String {
        self.inner.workflow_type.clone()
    }

    #[getter]
    fn workflow_type_version(&self) -> String {
        self.inner.workflow_type_version.clone()
    }

    #[getter]
    fn workflow_url(&self) -> String {
        self.inner.workflow_url.clone()
    }

    #[getter]
    fn workflow_params(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "workflow_params")
    }

    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "tags")
    }

    #[getter]
    fn workflow_engine(&self) -> Option<String> {
        self.inner.workflow_engine.clone()
    }

    #[getter]
    fn workflow_engine_version(&self) -> Option<String> {
        self.inner.workflow_engine_version.clone()
    }

    #[getter]
    fn workflow_engine_parameters(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "workflow_engine_parameters")
    }

    /// The request as JSON text, as given or received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The request as a new `dict`, as given or received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let workflow_type = slf.getattr("workflow_type")?.repr()?;
        let workflow_url = slf.getattr("workflow_url")?.repr()?;
        Ok(format!(
            "{name}(workflow_type={workflow_type}, workflow_url={workflow_url})"
        ))
    }
}

/// The source of an attachment's content: `bytes` (or `bytearray`), or a file path.
fn attachment_source(
    py: Python<'_>,
    filename: &str,
    content: &Bound<'_, PyAny>,
) -> PyResult<AttachmentSource> {
    if let Ok(bytes) = content.extract::<Cow<'_, [u8]>>() {
        return Ok(AttachmentSource::Bytes(bytes.into_owned()));
    }
    if !content.is_instance_of::<PyString>() && content.hasattr("__fspath__")? {
        // Opened here, so that errors are `OSError`s with the path, and the file that was
        // checked is the one that is sent. Only regular files are opened: opening a FIFO or
        // a device could block while holding the GIL.
        let path: PathBuf = content.extract()?;
        let not_a_file = || {
            PyValueError::new_err(format!(
                "Invalid attachment {filename}: {} is not a file",
                path.display()
            ))
        };
        let metadata = std::fs::metadata(&path).map_err(|error| os_error(py, error, &path))?;
        if !metadata.is_file() {
            return Err(not_a_file());
        }
        let file = std::fs::File::open(&path).map_err(|error| os_error(py, error, &path))?;
        let metadata = file
            .metadata()
            .map_err(|error| os_error(py, error, &path))?;
        if !metadata.is_file() {
            return Err(not_a_file());
        }
        // `tokio::fs` is enabled by `ga4gh-lib`, which depends on Tokio with all features.
        return Ok(AttachmentSource::Reader {
            reader: Box::pin(tokio::fs::File::from_std(file)),
            length: Some(metadata.len()),
        });
    }
    Err(PyValueError::new_err(format!(
        "Invalid attachment {filename}: content must be bytes or an os.PathLike, not {}",
        content.get_type().qualname()?
    )))
}

const ATTACHMENTS: &str = "attachments must be a mapping or a list of (filename, content) pairs";

/// Converts the `attachments` argument of `WES.run()`, checking the filenames.
pub(super) fn attachment_list(
    py: Python<'_>,
    attachments: &Bound<'_, PyAny>,
) -> PyResult<Vec<StreamingAttachment>> {
    let pairs = match attachments.downcast::<PyMapping>() {
        Ok(mapping) => mapping.items()?.into_any(),
        Err(_) => attachments.clone(),
    };
    let mut names = HashSet::new();
    let mut result = Vec::new();
    for pair in pairs.iter()? {
        let (filename, content): (String, Bound<'_, PyAny>) = pair?
            .extract()
            .map_err(|_| PyValueError::new_err(ATTACHMENTS))?;
        validate_filename(&filename).map_err(|error| {
            PyValueError::new_err(format!("Invalid attachment {filename:?}: {error}"))
        })?;
        if !names.insert(filename.clone()) {
            return Err(PyValueError::new_err(format!(
                "Invalid attachment {filename:?}: duplicate workflow attachment filename"
            )));
        }
        let source = attachment_source(py, &filename, &content)?;
        result.push(StreamingAttachment { filename, source });
    }
    Ok(result)
}
