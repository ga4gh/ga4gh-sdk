//! `TES`: the client for one TES service.

use super::calls::{connect, create_task, get_task, task_to_create};
use super::list::{list_endpoint, list_tasks, PyListTasksParams, PyTesListTasksResponse};
use super::state::parse_view;
use super::task::PyTask;
use super::tes_task::PyTesTask;
use crate::utils::convert::check_extra_args;
use crate::utils::runtime::io;
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::tes::TES;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

/// A TES client. Subclass it to add endpoints, using `self.transport`.
///
/// The public `inner` preserves the Rust extension API and is safe to share
/// between Python threads while synchronous requests release the GIL.
#[pyclass(name = "TES", module = "ga4gh_sdk", subclass)]
pub struct PyTES {
    pub inner: TES,
}

impl PyTES {
    /// Build a TES client for Rust extension crates while releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::attach(|py| Self::new_with_gil(py, config))
    }

    fn new_with_gil(py: Python<'_>, config: &PyConfiguration) -> PyResult<Self> {
        io(py, |rt| {
            rt.block_on(connect(&config.inner))
                .map(|inner| Self { inner })
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
            inner: self.inner.configuration().clone(),
        }
    }

    /// A transport with the client's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport().clone(),
        }
    }

    /// Creates a task and returns a `Task` for it.
    ///
    /// `task` is a `TesTask`, a `dict` (or other JSON-serialisable object), or the path of a
    /// JSON file as `str` or `os.PathLike`. A `str` is always a path; pass JSON text as
    /// `TesTask.from_json(text)`. The task's JSON is sent as given, including fields that
    /// the TES models don't know.
    pub fn create(&self, py: Python<'_>, task: &Bound<'_, PyAny>) -> PyResult<PyTask> {
        let task = task_to_create(task, "TES.create()")?;
        io(py, |rt| rt.block_on(create_task(&self.inner, task))).map(|inner| PyTask { inner })
    }

    /// Gets a task. `view` is `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case.
    pub fn get(&self, py: Python<'_>, view: &str, id: &str) -> PyResult<PyTesTask> {
        let view = parse_view(view)?;
        io(py, |rt| rt.block_on(get_task(&self.inner, view, id)))
    }

    /// Lists tasks, one page at a time.
    #[pyo3(signature = (params=None))]
    pub fn list(
        &self,
        py: Python<'_>,
        params: Option<PyListTasksParams>,
    ) -> PyResult<PyTesListTasksResponse> {
        let endpoint = list_endpoint(params)?;
        io(py, |rt| {
            rt.block_on(list_tasks(self.inner.transport(), &endpoint))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ga4gh_sdk::clients::serviceinfo::models::Service;
    use ga4gh_sdk::utils::configuration::Configuration;
    use ga4gh_sdk::utils::transport::Transport;

    /// The `config` getter returns the transport's configuration.
    #[test]
    fn config_is_transport_configuration() {
        let root = "https://example.org/custom/ga4gh/tes/v9/";
        let transport = Transport::new(&Configuration::new(url::Url::parse(root).unwrap()));
        let client = PyTES {
            inner: TES::without_discovery(transport, Service::default()),
        };
        assert_eq!(client.config().inner.base_path.as_str(), root);
    }
}
