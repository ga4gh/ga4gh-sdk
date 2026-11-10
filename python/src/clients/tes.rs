use crate::utils::convert::{block_on, to_python, with_query};
use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::models::TesListTasksResponse;
use ga4gh_sdk::clients::tes::models::TesState;
use ga4gh_sdk::clients::tes::models::TesTask;
use ga4gh_sdk::clients::tes::Task;
use ga4gh_sdk::clients::tes::TES;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use serde_json::Value;

fn invalid_response(error: serde_json::Error) -> PyErr {
    PyRuntimeError::new_err(format!("Invalid TES response: {error}"))
}

/// A TES task on the server, addressed by its ID.
#[pyclass(name = "Task", module = "GA4GH", subclass)]
pub struct PyTask {
    pub inner: Task,
}

#[pymethods]
impl PyTask {
    #[new]
    pub fn new(id: String, transport: &PyTransport) -> PyResult<Self> {
        Ok(PyTask {
            inner: Task::new(id, transport.inner.clone()),
        })
    }

    #[getter]
    fn id(&self) -> PyResult<String> {
        Ok(self.inner.id.clone())
    }

    /// A transport with the task's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport.clone(),
        }
    }

    pub fn status(&self) -> PyResult<String> {
        match block_on(self.inner.status())? {
            Ok(state) => Ok(format!("{:?}", state)),
            Err(e) => Err(PyRuntimeError::new_err(format!(
                "Failed to get task status: {}",
                e
            ))),
        }
    }

    pub fn cancel(&self) -> PyResult<String> {
        match block_on(self.inner.cancel())? {
            Ok(response) => Ok(response.to_string()),
            Err(e) => Err(PyRuntimeError::new_err(format!(
                "Failed to cancel task: {}",
                e
            ))),
        }
    }
}

/// A TES task as returned by the server.
///
/// `json()` and `to_dict()` return the task exactly as received, including fields that the
/// TES models don't know.
#[pyclass(name = "TesTask", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyTesTask {
    pub inner: TesTask,
    pub raw: Value,
}

impl PyTesTask {
    /// Builds a task from a server's JSON, keeping the JSON as received.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        let inner = serde_json::from_value(raw.clone()).map_err(invalid_response)?;
        Ok(PyTesTask { inner, raw })
    }
}

#[pymethods]
impl PyTesTask {
    #[getter]
    pub fn id(&self) -> PyResult<String> {
        Ok(self.inner.id.clone().unwrap_or_default())
    }

    #[getter]
    pub fn status(&self) -> PyResult<String> {
        Ok(self.inner.state.unwrap_or_default().to_string())
    }

    /// The task as JSON text, as received from the server.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The task as a `dict`, as received from the server.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }
}

/// Filters and paging for `TES.list()`.
#[pyclass(name = "ListTasksParams", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyListTasksParams {
    pub inner: ListTasksParams,
}

#[pymethods]
impl PyListTasksParams {
    /// All arguments are optional. `state` is a TES state such as `"RUNNING"`, in any case.
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
        let state = state
            .map(|state| {
                serde_json::from_value::<TesState>(Value::String(state.to_uppercase()))
                    .map_err(|_| PyValueError::new_err(format!("Invalid TES state: {state}")))
            })
            .transpose()?;
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

    #[getter]
    fn state(&self) -> Option<String> {
        self.inner.state.as_ref().map(ToString::to_string)
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
        self.inner.view.clone()
    }
}

/// One page of tasks from `TES.list()`.
#[pyclass(name = "ListTasksResponse", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyTesListTasksResponse {
    pub inner: TesListTasksResponse,
    pub tasks: Vec<PyTesTask>,
}

#[pymethods]
impl PyTesListTasksResponse {
    #[getter]
    pub fn tasks(&self) -> Vec<PyTesTask> {
        self.tasks.clone()
    }

    #[getter]
    pub fn next_page_token(&self) -> Option<String> {
        self.inner.next_page_token.clone()
    }
}

/// A TES client. Subclass it to add endpoints, using `self.transport`.
#[pyclass(unsendable, name = "TES", module = "GA4GH", subclass)]
pub struct PyTES {
    pub inner: TES,
}

#[pymethods]
impl PyTES {
    #[new]
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        match block_on(TES::new(&config.inner))? {
            Ok(instance) => Ok(PyTES { inner: instance }),
            Err(e) => Err(PyRuntimeError::new_err(format!(
                "Failed to create TES instance: {}",
                e
            ))),
        }
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

    pub fn create(&self, task_file_path: String) -> PyResult<PyTask> {
        let task_json = match std::fs::read_to_string(task_file_path.clone()) {
            Ok(contents) => contents,
            Err(e) => {
                eprintln!("Failed to read file: {}", e);
                task_file_path.to_string()
            }
        };
        let task: TesTask = serde_json::from_str(&task_json)
            .map_err(|e| PyValueError::new_err(format!("Failed to parse JSON: {}", e)))?;

        match block_on(self.inner.create(task))? {
            Ok(task) => Ok(PyTask { inner: task }),
            Err(e) => Err(PyRuntimeError::new_err(format!(
                "Failed to create task: {}",
                e
            ))),
        }
    }

    /// Gets a task. `view` is `"MINIMAL"`, `"BASIC"` or `"FULL"`.
    pub fn get(&self, view: &str, id: &str) -> PyResult<PyTesTask> {
        let response = block_on(self.inner.get_raw(view, id))?
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to get task: {}", e)))?;
        PyTesTask::from_json(serde_json::from_str(&response).map_err(invalid_response)?)
    }

    #[pyo3(signature = (params=None))]
    pub fn list(&self, params: Option<PyListTasksParams>) -> PyResult<PyTesListTasksResponse> {
        let params = params
            .map(|params| serde_json::to_value(params.inner))
            .transpose()
            .map_err(|e| PyValueError::new_err(format!("Invalid params: {e}")))?;
        let endpoint = with_query("tasks", params)?;
        let response = block_on(self.inner.transport.get(&endpoint, None))?
            .map_err(|e| PyRuntimeError::new_err(format!("Failed to list tasks: {}", e)))?;
        let raw: Value = serde_json::from_str(&response).map_err(invalid_response)?;
        let inner: TesListTasksResponse =
            serde_json::from_value(raw.clone()).map_err(invalid_response)?;
        let tasks = match raw.get("tasks") {
            Some(Value::Array(tasks)) => tasks
                .iter()
                .cloned()
                .map(PyTesTask::from_json)
                .collect::<PyResult<_>>()?,
            _ => Vec::new(),
        };
        Ok(PyTesListTasksResponse { inner, tasks })
    }
}
