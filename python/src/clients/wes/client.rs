//! `WES`: the client for one WES service.

use super::helpers::get_json;
use super::lists::{paged, PyRunListResponse};
use super::logs::PyRunLog;
use super::request::{attachment_list, PyRunRequest};
use super::run::{get_run_log, PyRun};
use crate::utils::convert::check_extra_args;
use crate::utils::runtime::io;
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::wes::WES;
use ga4gh_sdk::utils::operation::Operation;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple, PyType};
use std::path::PathBuf;

/// A WES client. Subclass it to add endpoints, using `self.transport`.
///
/// The public `inner` preserves the Rust extension API and is safe to share
/// between Python threads while synchronous requests release the GIL.
#[pyclass(name = "WES", module = "ga4gh_sdk", subclass)]
pub struct PyWES {
    pub inner: WES,
}

impl PyWES {
    /// Build a WES client for Rust extension crates while releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::with_gil(|py| Self::new_with_gil(py, config))
    }

    fn new_with_gil(py: Python<'_>, config: &PyConfiguration) -> PyResult<Self> {
        io(py, |rt| {
            rt.block_on(WES::new(&config.inner))
                .map(|inner| Self { inner })
                .map_err(|error| format!("Failed to create WES instance: {error}"))
        })
    }
}

#[pymethods]
impl PyWES {
    #[new]
    #[classmethod]
    #[pyo3(signature = (config, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        config: &PyConfiguration,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::new_with_gil(cls.py(), config)
    }

    /// The client's configuration.
    #[getter]
    fn config(&self) -> PyConfiguration {
        PyConfiguration {
            inner: self.inner.config.clone(),
        }
    }

    /// A transport with the client's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport.clone(),
        }
    }

    /// Starts a workflow run and returns a `Run` for it.
    ///
    /// `request` is a `RunRequest`, a `dict` (or other mapping), or the path of a JSON file as
    /// `str` or `os.PathLike`. A `str` is always a path; pass JSON text as
    /// `RunRequest.from_json(text)`. `attachments` maps filenames (relative paths) to
    /// contents as `bytes` or file paths (`os.PathLike`); a list of `(filename, content)`
    /// pairs works too. Files are streamed, not read into memory. Request fields that the WES
    /// models don't know are sent as further form parts; `workflow_attachment` raises
    /// `ValueError`.
    #[pyo3(signature = (request, attachments=None))]
    pub fn run(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyAny>,
        attachments: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRun> {
        if let Ok(text) = request.downcast::<PyString>() {
            // Lossy, so that paths with surrogate escapes (non-UTF-8 file names) pass.
            if text.to_string_lossy().trim_start().starts_with('{') {
                return Err(PyValueError::new_err(
                    "WES.run() takes a str as a file path; pass JSON text as \
                     RunRequest.from_json(text), or a path that starts with '{' as a \
                     pathlib.Path",
                ));
            }
        }
        let request = if request.is_instance_of::<PyString>() || request.hasattr("__fspath__")? {
            PyRunRequest::from_path(py, &request.extract::<PathBuf>()?)?
        } else {
            PyRunRequest::from_python(request)?
        };
        request.check_complete()?;
        let attachments = match attachments {
            Some(attachments) => attachment_list(py, attachments)?,
            None => Vec::new(),
        };
        let request = request.inner;
        io(py, |rt| {
            rt.block_on(self.inner.run_workflow_streaming(
                request,
                attachments,
                // The submission limit, as for `WES::run_workflow_with_attachments`.
                &Operation::new(self.inner.transport.config.timeouts.submission),
            ))
            .map(|inner| PyRun { inner })
            .map_err(|error| format!("Failed to run workflow: {error}"))
        })
    }

    /// Lists runs, one page at a time.
    #[pyo3(signature = (page_size=None, page_token=None))]
    pub fn list(
        &self,
        py: Python<'_>,
        page_size: Option<i64>,
        page_token: Option<String>,
    ) -> PyResult<PyRunListResponse> {
        let endpoint = paged("runs", page_size, page_token)?;
        PyRunListResponse::from_json(get_json(py, &self.inner.transport, &endpoint, "list runs")?)
    }

    /// Gets a run's details, with its request, state, logs and outputs, as a `RunLog`.
    pub fn get(&self, py: Python<'_>, id: &str) -> PyResult<PyRunLog> {
        get_run_log(py, &self.inner.transport, id)
    }
}
