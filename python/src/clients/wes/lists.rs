//! `RunSummary`, `RunListResponse` and `TaskListResponse`: pages of runs and tasks, and the
//! paging parameters of the WES list endpoints.

use super::helpers::{invalid_response, items, parse_model};
use super::logs::PyTaskLog;
use super::state::state_to_python;
use crate::utils::convert::{to_python, with_query};
use crate::utils::json::{field, view_accessors};
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
    /// The run summary parsed into the model; derived from `raw`.
    #[deprecated(note = "read `inner()`; build a changed run summary from JSON with `from_json`")]
    pub inner: RunListResponseRunsInner,
    /// The run's JSON as received.
    #[deprecated(note = "read `raw()`; build a changed run summary from JSON with `from_json`")]
    pub raw: Value,
}

view_accessors!(PyRunSummary, RunListResponseRunsInner);

impl PyRunSummary {
    #[allow(deprecated)] // Builds the fields that the accessors read.
    fn parse(raw: Value) -> Result<Self, String> {
        Ok(PyRunSummary {
            inner: parse_model(&raw)?,
            raw,
        })
    }

    /// Builds a run summary from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }
}

#[pymethods]
impl PyRunSummary {
    #[getter]
    fn run_id(&self) -> String {
        self.inner().run_id.clone()
    }

    /// The run's state as a `WesState`; `UNKNOWN` if the run has none or one
    /// that WES 1.1 does not define.
    #[getter]
    fn state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        state_to_python(py, self.inner().state.unwrap_or_default())
    }

    #[getter]
    fn start_time(&self) -> Option<String> {
        self.inner().start_time.clone()
    }

    #[getter]
    fn end_time(&self) -> Option<String> {
        self.inner().end_time.clone()
    }

    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "tags")
    }

    /// The run as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// The run as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let run_id = slf.getattr("run_id")?.repr()?;
        let state = slf.borrow().inner().state.unwrap_or_default();
        Ok(format!("{name}(run_id={run_id}, state='{state}')"))
    }
}

/// One page of runs from `WES.list()`.
#[pyclass(name = "RunListResponse", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyRunListResponse {
    /// The response parsed into the model; derived from `raw`.
    #[deprecated(note = "read `inner()`; build a changed response from JSON with `from_json`")]
    pub inner: RunListResponse,
    /// The runs of `raw`, each with its JSON.
    #[deprecated(note = "read `runs()`; build a changed response from JSON with `from_json`")]
    pub runs: Vec<PyRunSummary>,
    /// The response's JSON as received; a bare array of runs becomes `{"runs": [...]}`.
    #[deprecated(note = "read `raw()`; build a changed response from JSON with `from_json`")]
    pub raw: Value,
}

view_accessors!(PyRunListResponse, RunListResponse);

impl PyRunListResponse {
    /// Builds a response from a server's JSON, keeping the JSON.
    ///
    /// Accepts a bare array of runs, as WES 0.2.0 servers (GA4GH Starter Kit) send it.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }

    #[allow(deprecated)] // Builds the fields that the accessors read.
    fn parse(raw: Value) -> Result<Self, String> {
        let raw = match raw {
            Value::Array(runs) => serde_json::json!({ "runs": runs }),
            raw => raw,
        };
        let runs = items(&raw, "runs")
            .into_iter()
            .map(PyRunSummary::parse)
            .collect::<Result<_, _>>()?;
        Ok(PyRunListResponse {
            inner: parse_model(&raw)?,
            runs,
            raw,
        })
    }
}

#[pymethods]
impl PyRunListResponse {
    #[getter]
    #[allow(deprecated)] // The accessor that replaces the field.
    pub fn runs(&self) -> Vec<PyRunSummary> {
        self.runs.clone()
    }

    #[getter]
    pub fn next_page_token(&self) -> Option<String> {
        self.inner().next_page_token.clone()
    }

    /// The response as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// The response as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }
}

/// One page of tasks from `Run.tasks()`.
#[pyclass(name = "TaskListResponse", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTaskListResponse {
    /// The response parsed into the model; derived from `raw`.
    #[deprecated(note = "read `inner()`; build a changed response from JSON with `from_json`")]
    pub inner: TaskListResponse,
    /// The tasks of `raw`, each with its JSON.
    #[deprecated(note = "read `task_logs()`; build a changed response from JSON with `from_json`")]
    pub task_logs: Vec<PyTaskLog>,
    /// The response's JSON as received.
    #[deprecated(note = "read `raw()`; build a changed response from JSON with `from_json`")]
    pub raw: Value,
}

view_accessors!(PyTaskListResponse, TaskListResponse);

impl PyTaskListResponse {
    /// Builds a response from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }

    #[allow(deprecated)] // Builds the fields that the accessors read.
    fn parse(raw: Value) -> Result<Self, String> {
        let task_logs = items(&raw, "task_logs")
            .into_iter()
            .map(PyTaskLog::parse)
            .collect::<Result<_, _>>()?;
        Ok(PyTaskListResponse {
            inner: parse_model(&raw)?,
            task_logs,
            raw,
        })
    }
}

#[pymethods]
impl PyTaskListResponse {
    #[getter]
    #[allow(deprecated)] // The accessor that replaces the field.
    pub fn task_logs(&self) -> Vec<PyTaskLog> {
        self.task_logs.clone()
    }

    #[getter]
    pub fn next_page_token(&self) -> Option<String> {
        self.inner().next_page_token.clone()
    }

    /// The response as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// The response as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }
}

// The tests call `parse`, which `from_json` shares (see `utils::json::tests`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::json::tests::assert_describes;
    use serde_json::json;

    #[test]
    fn a_page_of_runs_describes_its_json() {
        let raw = json!({
            "runs": [{"run_id": "run-1", "state": null, "x_run": true}],
            "next_page_token": null,
            "x_page": "kept"
        });
        let page = PyRunListResponse::parse(raw.clone()).unwrap();
        assert_describes(page.inner(), page.raw(), &page.json(), &raw);
        assert_eq!(page.inner().extra["x_page"], "kept");
        let runs = page.runs();
        assert_eq!(runs.len(), 1);
        assert_describes(
            runs[0].inner(),
            runs[0].raw(),
            &runs[0].json(),
            &raw["runs"][0],
        );
        assert_eq!(runs[0].inner().state, None);
    }

    #[test]
    fn a_bare_array_of_runs_reads_as_a_page() {
        let page = PyRunListResponse::parse(json!([{"run_id": "run-1"}])).unwrap();
        let expected = json!({"runs": [{"run_id": "run-1"}]});
        assert_describes(page.inner(), page.raw(), &page.json(), &expected);
        assert_eq!(page.runs()[0].raw(), &expected["runs"][0]);
    }

    #[test]
    fn a_page_of_tasks_describes_its_json() {
        let raw = json!({
            "task_logs": [{"id": "t-1", "name": "step", "stdout": null, "x_task": 2}],
            "next_page_token": "next",
            "x_page": "kept"
        });
        let page = PyTaskListResponse::parse(raw.clone()).unwrap();
        assert_describes(page.inner(), page.raw(), &page.json(), &raw);
        assert_eq!(page.inner().next_page_token.as_deref(), Some("next"));
        let tasks = page.task_logs();
        assert_eq!(tasks.len(), 1);
        assert_describes(
            tasks[0].inner(),
            tasks[0].raw(),
            &tasks[0].json(),
            &raw["task_logs"][0],
        );
    }
}
