//! `RunLog` and `TaskLog`: a run's and a task's details, as returned by the server.

use super::helpers::parse_response;
use super::request::PyRunRequest;
use super::state::state_to_python;
use crate::utils::convert::to_python;
use crate::utils::json::field;
use ga4gh_sdk::clients::wes::models::{RunLog, TaskLog};
use pyo3::prelude::*;
use serde_json::Value;

/// A run's details, with its request, state, logs and outputs, as returned by the server.
#[pyclass(name = "RunLog", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyRunLog {
    pub inner: RunLog,
    /// The run's JSON as received.
    pub raw: Value,
}

impl PyRunLog {
    /// Builds a run log from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyRunLog {
            inner: parse_response(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyRunLog {
    #[getter]
    fn run_id(&self) -> Option<String> {
        self.inner.run_id.clone()
    }

    /// The run's request as a `RunRequest`, or `None` if it is missing or doesn't fit.
    #[getter]
    fn request(&self) -> Option<PyRunRequest> {
        self.inner.request.as_ref()?;
        PyRunRequest::parse(self.raw.get("request")?.clone()).ok()
    }

    /// The run's state as a `WesState`; `UNKNOWN` if the run has none or one
    /// that WES 1.1 does not define.
    #[getter]
    fn state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        state_to_python(py, self.inner.state.unwrap_or_default())
    }

    #[getter]
    fn run_log(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "run_log")
    }

    #[getter]
    fn task_logs_url(&self) -> Option<String> {
        self.inner.task_logs_url.clone()
    }

    #[getter]
    fn task_logs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "task_logs")
    }

    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "outputs")
    }

    /// The run as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The run as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let run_id = slf.getattr("run_id")?.repr()?;
        let state = slf.borrow().inner.state.unwrap_or_default();
        Ok(format!("{name}(run_id={run_id}, state='{state}')"))
    }
}

/// A task of a run, from `Run.tasks()` or `Run.task()` (WES 1.1).
#[pyclass(name = "TaskLog", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTaskLog {
    pub inner: TaskLog,
    /// The task's JSON as received.
    pub raw: Value,
}

impl PyTaskLog {
    /// Builds a task log from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyTaskLog {
            inner: parse_response(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyTaskLog {
    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn cmd(&self) -> Option<Vec<String>> {
        self.inner.cmd.clone()
    }

    #[getter]
    fn start_time(&self) -> Option<String> {
        self.inner.start_time.clone()
    }

    #[getter]
    fn end_time(&self) -> Option<String> {
        self.inner.end_time.clone()
    }

    #[getter]
    fn stdout(&self) -> Option<String> {
        self.inner.stdout.clone()
    }

    #[getter]
    fn stderr(&self) -> Option<String> {
        self.inner.stderr.clone()
    }

    #[getter]
    fn exit_code(&self) -> Option<i32> {
        self.inner.exit_code
    }

    #[getter]
    fn system_logs(&self) -> Option<Vec<String>> {
        self.inner.system_logs.clone()
    }

    #[getter]
    fn tes_uri(&self) -> Option<String> {
        self.inner.tes_uri.clone()
    }

    /// The task as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The task as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let task_name = slf.getattr("name")?.repr()?;
        Ok(format!("{name}(id={id}, name={task_name})"))
    }
}
