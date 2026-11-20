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

    /// The run's ID.
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

    /// Gets the run's current state.
    ///
    /// Returns:
    ///     The state.
    ///
    /// Raises:
    ///     RuntimeError: The request failed.
    pub fn status(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = io(py, |rt| {
            rt.block_on(self.inner.status())
                .map_err(|e| format!("Failed to get run status: {e}"))
        })?;
        state_to_python(py, state)
    }

    /// Asks the server to cancel the run.
    ///
    /// Raises:
    ///     RuntimeError: The request failed.
    pub fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        io(py, |rt| {
            rt.block_on(self.inner.cancel())
                .map(|_| ())
                .map_err(|e| format!("Failed to cancel run: {e}"))
        })
    }

    /// Gets the run's details, with its request, state, logs and outputs.
    ///
    /// Returns:
    ///     The run's details.
    ///
    /// Raises:
    ///     RuntimeError: The request failed or the response doesn't fit the WES models.
    pub fn log(&self, py: Python<'_>) -> PyResult<PyRunLog> {
        get_run_log(py, &self.inner.transport, &self.inner.id)
    }

    /// Lists the run's tasks, one page at a time (WES 1.1).
    ///
    /// Args:
    ///     page_size: The most tasks on the page.
    ///     page_token: The `next_page_token` of the previous page.
    ///
    /// Returns:
    ///     The page.
    ///
    /// Raises:
    ///     RuntimeError: The request failed or the response doesn't fit the WES models.
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
    ///
    /// Args:
    ///     task_id: The task's ID.
    ///
    /// Returns:
    ///     The task.
    ///
    /// Raises:
    ///     ValueError: `task_id` is empty, `.` or `..`.
    ///     RuntimeError: The request failed or the response doesn't fit the WES models.
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
