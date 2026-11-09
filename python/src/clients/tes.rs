use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::models::TesListTasksResponse;
use ga4gh_sdk::clients::tes::models::TesTask;
use ga4gh_sdk::clients::tes::Task;
use ga4gh_sdk::clients::tes::TES;
use pyo3::prelude::*;
use tokio::runtime::Runtime;

#[pyclass(name = "Task", module = "GA4GH")]
pub struct PyTask {
    inner: Task,
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

    pub fn status(&self) -> PyResult<String> {
        let rt = Runtime::new().unwrap();
        match rt.block_on(self.inner.status()) {
            Ok(state) => Ok(format!("{:?}", state)),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to get task status: {}",
                e
            ))),
        }
    }

    pub fn cancel(&self) -> PyResult<String> {
        let rt = Runtime::new().unwrap();
        match rt.block_on(self.inner.cancel()) {
            Ok(response) => Ok(response.to_string()),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to cancel task: {}",
                e
            ))),
        }
    }
}

#[pyclass(name = "PyTesTask", module = "GA4GH")]
#[derive(Clone)]
pub struct PyTesTask {
    inner: TesTask,
}

#[pymethods]
impl PyTesTask {
    #[getter]
    pub fn id(&self) -> PyResult<String> {
        Ok(self.inner.id.clone().unwrap_or_default())
    }

    #[getter]
    pub fn status(&self) -> PyResult<String> {
        Ok(self.inner.state.clone().unwrap_or_default().to_string())
    }
}

#[pyclass(name = "ListTasksParams", module = "GA4GH")]
#[derive(Clone)]
pub struct PyListTasksParams {
    _inner: ListTasksParams,
}

#[pyclass(name = "ListTasksParams", module = "GA4GH")]
#[derive(Clone)]
pub struct PyTesListTasksResponse {
    _inner: TesListTasksResponse,
}

#[pymethods]
impl PyTesListTasksResponse {
    #[getter]
    pub fn tasks(&self) -> Vec<PyTesTask> {
        self._inner
            .tasks
            .iter()
            .map(|task| PyTesTask {
                inner: task.clone(),
            })
            .collect()
    }
}

#[pyclass(unsendable, name = "TES", module = "GA4GH")]
pub struct PyTES {
    inner: TES,
}

#[pymethods]
impl PyTES {
    #[new]
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        let rt = Runtime::new().unwrap();
        match rt.block_on(TES::new(&config.inner)) {
            Ok(instance) => Ok(PyTES { inner: instance }),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to create TES instance: {}",
                e
            ))),
        }
    }

    pub fn create(&self, task_file_path: String) -> PyResult<PyTask> {
        let rt = Runtime::new().unwrap();

        let task_json = match std::fs::read_to_string(task_file_path.clone()) {
            Ok(contents) => contents,
            Err(e) => {
                eprintln!("Failed to read file: {}", e);
                task_file_path.to_string()
            }
        };
        let task: TesTask = serde_json::from_str(&task_json).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Failed to parse JSON: {}", e))
        })?;

        match rt.block_on(self.inner.create(task.clone())) {
            Ok(task) => Ok(PyTask { inner: task }),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to create task: {}",
                e
            ))),
        }
    }

    pub fn get(&self, view: &str, id: &str) -> PyResult<PyTesTask> {
        let rt = Runtime::new().unwrap();
        match rt.block_on(self.inner.get(view, id)) {
            Ok(task) => Ok(PyTesTask { inner: task }),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to get task: {}",
                e
            ))),
        }
    }

    #[pyo3(signature = (params=None))]
    pub fn list(&self, params: Option<PyListTasksParams>) -> PyResult<PyTesListTasksResponse> {
        let rt = Runtime::new().unwrap();
        let params = params.map(|p| p._inner);
        match rt.block_on(self.inner.list_tasks(params)) {
            Ok(response) => Ok(PyTesListTasksResponse { _inner: response }),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to list tasks: {}",
                e
            ))),
        }
    }
}
