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

/// A WES client (WES 1.1). Subclass it to add endpoints, using `self.transport`.
#[pyclass(name = "WES", module = "ga4gh_sdk", subclass)]
pub struct PyWES {
    /// The library client, for Rust extension crates. It is safe to share between Python
    /// threads while synchronous requests release the GIL.
    pub inner: WES,
}

impl PyWES {
    /// Build a WES client for Rust extension crates while releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::attach(|py| Self::new_with_gil(py, config))
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
            inner: self.inner.configuration().clone(),
        }
    }

    /// A transport with the client's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport().clone(),
        }
    }

    /// Starts a workflow run.
    ///
    /// `workflow_type`, `workflow_type_version`, `workflow_url` and `workflow_params` are
    /// required. Request fields that the WES models don't know are sent as further form
    /// parts. The upload and the answer must finish within the `submission` time limit.
    /// If starting fails after submission, the server may have started the run; don't
    /// retry automatically.
    ///
    /// Args:
    ///     request: A `RunRequest`, a `dict` (or other mapping), or the path of a JSON
    ///         file as `str` or `os.PathLike`. A `str` is always a path; pass JSON text
    ///         as `RunRequest.from_json(text)`.
    ///     attachments: Filenames (relative paths with `/`, such as `tools/echo.cwl`)
    ///         mapped to contents as `bytes` or to file paths (`os.PathLike`), which are
    ///         streamed; or a list of `(filename, content)` pairs.
    ///
    /// Returns:
    ///     The run.
    ///
    /// Raises:
    ///     ValueError: The request or an attachment name is invalid, a `str` starts with
    ///         `{` (after whitespace), or the request has a `workflow_attachment` field
    ///         (pass files with `attachments`).
    ///     OSError: A request or attachment file can't be read.
    ///     RuntimeError: The request failed or the response doesn't fit the WES models.
    #[pyo3(signature = (request, attachments=None))]
    pub fn run(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyAny>,
        attachments: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRun> {
        if let Ok(text) = request.cast::<PyString>() {
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
        let request = request.inner().clone();
        io(py, |rt| {
            rt.block_on(self.inner.run_workflow_streaming(
                request,
                attachments,
                // The submission limit, as for `WES::run_workflow_with_attachments`.
                &Operation::new(self.inner.transport().configuration().timeouts.submission),
            ))
            .map(|inner| PyRun { inner })
            .map_err(|error| format!("Failed to run workflow: {error}"))
        })
    }

    /// Lists runs, one page at a time.
    ///
    /// A bare array of runs, as WES 0.2.0 servers send it, becomes a page without a token.
    ///
    /// Args:
    ///     page_size: The most runs on the page.
    ///     page_token: The `next_page_token` of the previous page.
    ///
    /// Returns:
    ///     The page.
    ///
    /// Raises:
    ///     RuntimeError: The request failed or the response doesn't fit the WES models.
    #[pyo3(signature = (page_size=None, page_token=None))]
    pub fn list(
        &self,
        py: Python<'_>,
        page_size: Option<i64>,
        page_token: Option<String>,
    ) -> PyResult<PyRunListResponse> {
        let endpoint = paged("runs", page_size, page_token)?;
        PyRunListResponse::from_json(get_json(
            py,
            self.inner.transport(),
            &endpoint,
            "list runs",
        )?)
    }

    /// Gets a run's details, like `Run(id, wes.transport).log()`.
    ///
    /// Args:
    ///     id: The run's ID.
    ///
    /// Returns:
    ///     The run's details.
    ///
    /// Raises:
    ///     ValueError: `id` is empty, `.` or `..`.
    ///     RuntimeError: The request failed or the response doesn't fit the WES models.
    pub fn get(&self, py: Python<'_>, id: &str) -> PyResult<PyRunLog> {
        get_run_log(py, self.inner.transport(), id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ga4gh_sdk::clients::serviceinfo::models::Service;
    use ga4gh_sdk::utils::configuration::Configuration;
    use ga4gh_sdk::utils::transport::Transport;

    /// The `config` getter returns the transport's configuration.
    #[test]
    fn config_is_transport_configuration() {
        let root = "https://example.org/custom/ga4gh/wes/v9/";
        let transport = Transport::new(&Configuration::new(url::Url::parse(root).unwrap()));
        let client = PyWES {
            inner: WES::without_discovery(transport, Service::default()),
        };
        assert_eq!(client.config().inner.base_path.as_str(), root);
    }
}
