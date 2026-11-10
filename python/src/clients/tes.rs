use crate::utils::runtime::io;
use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::serviceinfo::models::Service;
use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::models::TesListTasksResponse;
use ga4gh_sdk::clients::tes::models::TesTask;
use ga4gh_sdk::clients::tes::Task;
use ga4gh_sdk::clients::tes::TES;
use ga4gh_sdk::utils::{configuration::Configuration, transport::Transport};
use pyo3::prelude::*;

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

// Keep the successful discovery snapshot, rather than the SDK's non-Send
// Result<Service, Box<dyn Error>>. Reconstruct the lightweight facade inside
// the released-GIL closure, preserving both the service and HTTP connection pool.
#[pyclass(name = "TES", module = "GA4GH")]
pub struct PyTES {
    config: Configuration,
    service: Service,
    transport: Transport,
}

impl PyTES {
    fn client(&self) -> TES {
        TES {
            config: self.config.clone(),
            service: Ok(self.service.clone()),
            transport: self.transport.clone(),
        }
    }
}

#[pymethods]
impl PyTES {
    #[new]
    pub fn new(py: Python<'_>, config: &PyConfiguration) -> PyResult<Self> {
        let config = config.inner.clone();
        io(py, |rt| {
            let client = rt
                .block_on(TES::new(&config))
                .map_err(|e| format!("Failed to create TES instance: {e}"))?;
            let service = client.service.map_err(|e| e.to_string())?;
            Ok(Self {
                config,
                service,
                transport: client.transport,
            })
        })
    }

    pub fn create(&self, py: Python<'_>, task_file_path: String) -> PyResult<PyTask> {
        let task_json = std::fs::read_to_string(&task_file_path).unwrap_or(task_file_path);
        let task: TesTask = serde_json::from_str(&task_json).map_err(|e| {
            pyo3::exceptions::PyValueError::new_err(format!("Failed to parse JSON: {e}"))
        })?;
        io(py, |rt| {
            rt.block_on(self.client().create(task))
                .map(|task| PyTask { inner: task })
                .map_err(|e| format!("Failed to create task: {e}"))
        })
    }

    pub fn get(&self, py: Python<'_>, view: &str, id: &str) -> PyResult<PyTesTask> {
        io(py, |rt| {
            rt.block_on(self.client().get(view, id))
                .map(|task| PyTesTask { inner: task })
                .map_err(|e| format!("Failed to get task: {e}"))
        })
    }

    #[pyo3(signature = (params=None))]
    pub fn list(
        &self,
        py: Python<'_>,
        params: Option<PyListTasksParams>,
    ) -> PyResult<PyTesListTasksResponse> {
        let params = params.map(|p| p._inner);
        io(py, |rt| {
            rt.block_on(self.client().list_tasks(params))
                .map(|response| PyTesListTasksResponse { _inner: response })
                .map_err(|e| format!("Failed to list tasks: {e}"))
        })
    }
}
