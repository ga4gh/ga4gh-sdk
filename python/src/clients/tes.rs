use crate::utils::convert::{check_extra_args, to_python, with_query};
use crate::utils::runtime::io;
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
use pyo3::types::{PyDict, PyTuple, PyType};
use serde_json::Value;

fn invalid_response(error: serde_json::Error) -> PyErr {
    PyRuntimeError::new_err(format!("Invalid TES response: {error}"))
}

/// A TES task on the server, addressed by its ID.
#[pyclass(name = "Task", module = "GA4GH", subclass)]
pub struct PyTask {
    pub inner: Task,
}

impl PyTask {
    pub fn new(id: String, transport: &PyTransport) -> PyResult<Self> {
        Ok(PyTask {
            inner: Task::new(id, transport.inner.clone()),
        })
    }
}

#[pymethods]
impl PyTask {
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

    pub fn status(&self, py: Python<'_>) -> PyResult<String> {
        io(py, |rt| {
            rt.block_on(self.inner.status())
                .map(|state| format!("{state:?}"))
                .map_err(|e| format!("Failed to get task status: {e}"))
        })
    }

    pub fn cancel(&self, py: Python<'_>) -> PyResult<String> {
        io(py, |rt| {
            rt.block_on(self.inner.cancel())
                .map(|response| response.to_string())
                .map_err(|e| format!("Failed to cancel task: {e}"))
        })
    }
}

/// A TES task as returned by the server.
///
/// `json()` and `to_dict()` return the task's JSON as received, including fields that the
/// TES models don't know (object keys may come back in another order).
#[pyclass(name = "TesTask", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyTesTask {
    /// The task parsed into the TES model; derived from `raw`.
    pub inner: TesTask,
    /// The task's JSON as received. Build both fields with `from_json` to keep them in step.
    pub raw: Value,
}

impl PyTesTask {
    /// Builds a task from a server's JSON, keeping the JSON.
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
///
/// The public `inner` preserves the Rust extension API and is safe to share
/// between Python threads while synchronous requests release the GIL.
#[pyclass(name = "TES", module = "GA4GH", subclass)]
pub struct PyTES {
    pub inner: TES,
}

impl PyTES {
    /// Build a TES client for Rust extension crates while releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::with_gil(|py| Self::new_with_gil(py, config))
    }

    fn new_with_gil(py: Python<'_>, config: &PyConfiguration) -> PyResult<Self> {
        io(py, |rt| {
            rt.block_on(TES::new(&config.inner))
                .map(|inner| Self { inner })
                .map_err(|error| format!("Failed to create TES instance: {error}"))
        })
    }
}

#[pymethods]
impl PyTES {
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

    pub fn create(&self, py: Python<'_>, task_file_path: String) -> PyResult<PyTask> {
        let task_json = std::fs::read_to_string(&task_file_path).unwrap_or(task_file_path);
        let task: TesTask = serde_json::from_str(&task_json)
            .map_err(|error| PyValueError::new_err(format!("Failed to parse JSON: {error}")))?;
        io(py, |rt| {
            rt.block_on(self.inner.create(task))
                .map(|task| PyTask { inner: task })
                .map_err(|error| format!("Failed to create task: {error}"))
        })
    }

    /// Gets a task. `view` is `"MINIMAL"`, `"BASIC"` or `"FULL"`.
    pub fn get(&self, py: Python<'_>, view: &str, id: &str) -> PyResult<PyTesTask> {
        let view = view.to_owned();
        let id = id.to_owned();
        let response = io(py, |rt| {
            rt.block_on(self.inner.get_raw(&view, &id))
                .map_err(|error| format!("Failed to get task: {error}"))
        })?;
        PyTesTask::from_json(serde_json::from_str(&response).map_err(invalid_response)?)
    }

    #[pyo3(signature = (params=None))]
    pub fn list(
        &self,
        py: Python<'_>,
        params: Option<PyListTasksParams>,
    ) -> PyResult<PyTesListTasksResponse> {
        // Not `TES::list_tasks`: reqwest's query encoder cannot repeat tag lists.
        let params = params
            .map(|params| serde_json::to_value(params.inner))
            .transpose()
            .map_err(|error| PyValueError::new_err(format!("Invalid params: {error}")))?;
        let endpoint = with_query("tasks", params)?;
        let transport = self.inner.transport.clone();
        let response = io(py, move |rt| {
            rt.block_on(transport.get(&endpoint, None))
                .map_err(|error| format!("Failed to list tasks: {error}"))
        })?;
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
