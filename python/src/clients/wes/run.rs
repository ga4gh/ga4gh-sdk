//! `Run`: a WES run on the server, addressed by its ID.

use super::helpers::get_json;
use super::lists::{paged, PyTaskListResponse};
use super::logs::{PyRunLog, PyTaskLog};
use super::state::state_to_python;
use crate::utils::convert::check_extra_args;
use crate::utils::runtime::io;
use crate::utils::validate::require_id;
use crate::PyTransport;
use ga4gh_sdk::clients::wes::{urlencode, Run};
use ga4gh_sdk::utils::transport::Transport;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

/// Gets a run's details.
pub(super) fn get_run_log(py: Python<'_>, transport: &Transport, id: &str) -> PyResult<PyRunLog> {
    require_id(id, "run ID")?;
    let endpoint = format!("runs/{}", urlencode(id));
    PyRunLog::from_json(get_json(py, transport, &endpoint, "get run")?)
}

/// A WES run on the server, addressed by its ID.
#[pyclass(name = "Run", module = "ga4gh_sdk", subclass)]
pub struct PyRun {
    pub inner: Run,
}

impl PyRun {
    /// Addresses the run `id` with `transport`; an empty ID raises `ValueError`.
    pub fn new(id: String, transport: &PyTransport) -> PyResult<Self> {
        require_id(&id, "run ID")?;
        Ok(PyRun {
            inner: Run::new(id, transport.inner.clone()),
        })
    }
}

#[pymethods]
impl PyRun {
    #[new]
    #[classmethod]
    #[pyo3(signature = (id, transport, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        id: String,
        transport: &PyTransport,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::new(id, transport)
    }

    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    /// A transport with the run's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport.clone(),
        }
    }

    /// The run's current state, as a `WesState`.
    pub fn status(&self, py: Python<'_>) -> PyResult<PyObject> {
        let state = io(py, |rt| {
            rt.block_on(self.inner.status())
                .map_err(|e| format!("Failed to get run status: {e}"))
        })?;
        state_to_python(py, state)
    }

    /// Asks the server to cancel the run. Returns `None`.
    pub fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        io(py, |rt| {
            rt.block_on(self.inner.cancel())
                .map(|_| ())
                .map_err(|e| format!("Failed to cancel run: {e}"))
        })
    }

    /// The run's details, with its request, state, logs and outputs, as a `RunLog`.
    pub fn log(&self, py: Python<'_>) -> PyResult<PyRunLog> {
        get_run_log(py, &self.inner.transport, &self.inner.id)
    }

    /// Lists the run's tasks, one page at a time (WES 1.1).
    #[pyo3(signature = (page_size=None, page_token=None))]
    pub fn tasks(
        &self,
        py: Python<'_>,
        page_size: Option<i64>,
        page_token: Option<String>,
    ) -> PyResult<PyTaskListResponse> {
        let endpoint = paged(
            &format!("runs/{}/tasks", urlencode(&self.inner.id)),
            page_size,
            page_token,
        )?;
        PyTaskListResponse::from_json(get_json(
            py,
            &self.inner.transport,
            &endpoint,
            "list run tasks",
        )?)
    }

    /// Gets one of the run's tasks (WES 1.1).
    pub fn task(&self, py: Python<'_>, task_id: &str) -> PyResult<PyTaskLog> {
        require_id(task_id, "task ID")?;
        let endpoint = format!(
            "runs/{}/tasks/{}",
            urlencode(&self.inner.id),
            urlencode(task_id)
        );
        PyTaskLog::from_json(get_json(
            py,
            &self.inner.transport,
            &endpoint,
            "get run task",
        )?)
    }
}
