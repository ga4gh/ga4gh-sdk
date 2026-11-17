//! `ListTasksParams` and `ListTasksResponse`, the filters and results of `TES.list()`, and
//! the calls that list tasks.

use super::helpers::{invalid_response, invalid_response_text};
use super::state::{parse_state, parse_view, state_to_python};
use super::tes_task::PyTesTask;
use crate::utils::convert::{to_python, with_query};
use crate::utils::json::view_accessors;
use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::models::TesListTasksResponse;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde_json::Value;

/// The endpoint that lists tasks with `params`.
pub fn list_endpoint(params: Option<PyListTasksParams>) -> PyResult<String> {
    // Not `TES::list_tasks`, which drops the raw JSON that `ListTasksResponse`
    // keeps; the query is the same (see `ga4gh_sdk::utils::query`).
    let params = params
        .map(|params| serde_json::to_value(params.inner))
        .transpose()
        .map_err(|error| PyValueError::new_err(format!("Invalid params: {error}")))?;
    with_query("tasks", params)
}

/// Lists tasks from an endpoint made by [`list_endpoint`].
pub async fn list_tasks(
    transport: &Transport,
    endpoint: &str,
) -> Result<PyTesListTasksResponse, String> {
    let response = transport
        .get(endpoint, None)
        .await
        .map_err(|error| format!("Failed to list tasks: {error}"))?;
    let raw = serde_json::from_str(&response).map_err(invalid_response_text)?;
    PyTesListTasksResponse::parse(raw).map_err(invalid_response_text)
}

/// Filters and paging for `TES.list()`.
#[pyclass(name = "ListTasksParams", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyListTasksParams {
    pub inner: ListTasksParams,
}

// Preserve the owned Rust argument type used by TES and AsyncTES.
impl<'a, 'py> FromPyObject<'a, 'py> for PyListTasksParams {
    type Error = PyErr;

    fn extract(value: Borrowed<'a, 'py, PyAny>) -> PyResult<Self> {
        Ok(value.cast::<Self>()?.borrow().clone())
    }
}

#[pymethods]
impl PyListTasksParams {
    /// All arguments are optional. `state` and `view` names are case-insensitive.
    #[new]
    #[pyo3(signature = (
        name_prefix=None, state=None, tag_key=None, tag_value=None,
        page_size=None, page_token=None, view=None
    ))]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name_prefix: Option<String>,
        state: Option<String>,
        tag_key: Option<Vec<String>>,
        tag_value: Option<Vec<String>>,
        page_size: Option<i32>,
        page_token: Option<String>,
        view: Option<String>,
    ) -> PyResult<Self> {
        let state = state.as_deref().map(parse_state).transpose()?;
        let view = view.as_deref().map(parse_view).transpose()?;
        Ok(PyListTasksParams {
            inner: ListTasksParams {
                name_prefix,
                state,
                tag_key,
                tag_value,
                page_size,
                page_token,
                view,
            },
        })
    }

    #[getter]
    fn name_prefix(&self) -> Option<String> {
        self.inner.name_prefix.clone()
    }

    /// The state filter as a `TesState`, or `None`.
    #[getter]
    fn state(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        self.inner
            .state
            .map(|state| state_to_python(py, state))
            .transpose()
    }

    #[getter]
    fn tag_key(&self) -> Option<Vec<String>> {
        self.inner.tag_key.clone()
    }

    #[getter]
    fn tag_value(&self) -> Option<Vec<String>> {
        self.inner.tag_value.clone()
    }

    #[getter]
    fn page_size(&self) -> Option<i32> {
        self.inner.page_size
    }

    #[getter]
    fn page_token(&self) -> Option<String> {
        self.inner.page_token.clone()
    }

    #[getter]
    fn view(&self) -> Option<String> {
        self.inner.view.map(|view| view.as_str().to_owned())
    }
}

/// One page of tasks from `TES.list()`.
#[pyclass(name = "ListTasksResponse", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTesListTasksResponse {
    /// The response parsed into the model; derived from `raw`.
    #[deprecated(note = "read `inner()`; build a changed response from JSON with `from_json`")]
    pub inner: TesListTasksResponse,
    /// The tasks of `raw`, each with its JSON.
    #[deprecated(note = "read `tasks()`; build a changed response from JSON with `from_json`")]
    pub tasks: Vec<PyTesTask>,
    /// The response's JSON as received.
    #[deprecated(note = "read `raw()`; build a changed response from JSON with `from_json`")]
    pub raw: Value,
}

view_accessors!(PyTesListTasksResponse, TesListTasksResponse);

impl PyTesListTasksResponse {
    #[allow(deprecated)] // Builds the fields that the accessors read.
    fn parse(raw: Value) -> Result<Self, String> {
        let inner: TesListTasksResponse =
            serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
        let tasks = match raw.get("tasks") {
            Some(Value::Array(tasks)) => tasks
                .iter()
                .cloned()
                .map(PyTesTask::parse)
                .collect::<Result<_, _>>()?,
            _ => Vec::new(),
        };
        Ok(PyTesListTasksResponse { inner, tasks, raw })
    }

    /// Builds a response from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }
}

#[pymethods]
impl PyTesListTasksResponse {
    #[getter]
    #[allow(deprecated)] // The accessor that replaces the field.
    pub fn tasks(&self) -> Vec<PyTesTask> {
        self.tasks.clone()
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
    fn a_page_of_tasks_describes_its_json() {
        let raw = json!({
            "tasks": [{"id": "task-1", "name": null, "x_task": 1}],
            "next_page_token": null,
            "x_page": "kept"
        });
        let page = PyTesListTasksResponse::parse(raw.clone()).unwrap();
        assert_describes(page.inner(), page.raw(), &page.json(), &raw);
        assert_eq!(page.inner().next_page_token, None);
        assert_eq!(page.inner().extra["x_page"], "kept");
        let tasks = page.tasks();
        assert_eq!(tasks.len(), 1);
        assert_describes(
            tasks[0].inner(),
            tasks[0].raw(),
            &tasks[0].json(),
            &raw["tasks"][0],
        );
        assert_eq!(&page.inner().tasks[0], tasks[0].inner());
    }
}
