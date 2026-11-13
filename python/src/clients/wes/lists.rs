//! `RunSummary`, `RunListResponse` and `TaskListResponse`: pages of runs and tasks, and the
//! paging parameters of the WES list endpoints.

use super::helpers::{items, parse_response};
use super::logs::PyTaskLog;
use super::state::state_to_python;
use crate::utils::convert::{to_python, with_query};
use crate::utils::json::field;
use ga4gh_sdk::clients::wes::models::{
    RunListResponse, RunListResponseRunsInner, TaskListResponse,
};
use pyo3::prelude::*;
use serde_json::Value;

/// Appends the paging parameters of the WES list endpoints.
pub(super) fn paged(
    endpoint: &str,
    page_size: Option<i64>,
    page_token: Option<String>,
) -> PyResult<String> {
    with_query(
        endpoint,
        Some(serde_json::json!({ "page_size": page_size, "page_token": page_token })),
    )
}

/// A run in the list of `WES.list()`.
#[pyclass(name = "RunSummary", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyRunSummary {
    pub inner: RunListResponseRunsInner,
    /// The run's JSON as received.
    pub raw: Value,
}

impl PyRunSummary {
    /// Builds a run summary from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyRunSummary {
            inner: parse_response(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyRunSummary {
    #[getter]
    fn run_id(&self) -> String {
        self.inner.run_id.clone()
    }

    /// The run's state as a `WesState`; `UNKNOWN` if the run has none or one
    /// that WES 1.1 does not define.
    #[getter]
    fn state(&self, py: Python<'_>) -> PyResult<PyObject> {
        state_to_python(py, self.inner.state.unwrap_or_default())
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
    fn tags(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "tags")
    }

    /// The run as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The run as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let run_id = slf.getattr("run_id")?.repr()?;
        let state = slf.borrow().inner.state.unwrap_or_default();
        Ok(format!("{name}(run_id={run_id}, state='{state}')"))
    }
}

/// One page of runs from `WES.list()`.
#[pyclass(name = "RunListResponse", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyRunListResponse {
    pub inner: RunListResponse,
    pub runs: Vec<PyRunSummary>,
    /// The response's JSON as received; a bare array of runs becomes `{"runs": [...]}`.
    pub raw: Value,
}

impl PyRunListResponse {
    /// Builds a response from a server's JSON, keeping the JSON.
    ///
    /// Accepts a bare array of runs, as WES 0.2.0 servers (GA4GH Starter Kit) send it.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        let raw = match raw {
            Value::Array(runs) => serde_json::json!({ "runs": runs }),
            raw => raw,
        };
        let runs = items(&raw, "runs")
            .into_iter()
            .map(PyRunSummary::from_json)
            .collect::<PyResult<_>>()?;
        Ok(PyRunListResponse {
            inner: parse_response(&raw)?,
            runs,
            raw,
        })
    }
}

#[pymethods]
impl PyRunListResponse {
    #[getter]
    pub fn runs(&self) -> Vec<PyRunSummary> {
        self.runs.clone()
    }

    #[getter]
    pub fn next_page_token(&self) -> Option<String> {
        self.inner.next_page_token.clone()
    }

    /// The response as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The response as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }
}

/// One page of tasks from `Run.tasks()`.
#[pyclass(name = "TaskListResponse", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTaskListResponse {
    pub inner: TaskListResponse,
    pub task_logs: Vec<PyTaskLog>,
    /// The response's JSON as received.
    pub raw: Value,
}

impl PyTaskListResponse {
    /// Builds a response from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        let task_logs = items(&raw, "task_logs")
            .into_iter()
            .map(PyTaskLog::from_json)
            .collect::<PyResult<_>>()?;
        Ok(PyTaskListResponse {
            inner: parse_response(&raw)?,
            task_logs,
            raw,
        })
    }
}

#[pymethods]
impl PyTaskListResponse {
    #[getter]
    pub fn task_logs(&self) -> Vec<PyTaskLog> {
        self.task_logs.clone()
    }

    #[getter]
    pub fn next_page_token(&self) -> Option<String> {
        self.inner.next_page_token.clone()
    }

    /// The response as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The response as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }
}
