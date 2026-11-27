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
#[pyclass(name = "TES", module = "ga4gh_sdk", subclass)]
pub struct PyTES {
    /// The library client, for Rust extension crates. It is safe to share between Python
    /// threads while synchronous requests release the GIL.
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

    /// Creates a task.
    ///
    /// The task's JSON is sent as given, including fields that the TES models don't know.
    /// If creation fails after submission, the server may have created the task; don't
    /// retry automatically.
    ///
    /// Args:
    ///     task: A `TesTask`, a `dict` (or other mapping), or the path of a JSON file as
    ///         `str` or `os.PathLike`. A `str` is always a path; pass JSON text as
    ///         `TesTask.from_json(text)`.
    ///
    /// Returns:
    ///     The created task.
    ///
    /// Raises:
    ///     ValueError: The task is invalid, or a `str` starts with `{`.
    ///     OSError: The task file can't be read, for example `FileNotFoundError`.
    ///     RuntimeError: The request failed or the response doesn't fit the TES models.
    pub fn create(&self, py: Python<'_>, task: &Bound<'_, PyAny>) -> PyResult<PyTask> {
        let task = task_to_create(task, "TES.create()")?;
        io(py, |rt| rt.block_on(create_task(&self.inner, task))).map(|inner| PyTask { inner })
    }

    /// Gets a task.
    ///
    /// Args:
    ///     view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case.
    ///     id: The task's ID.
    ///
    /// Returns:
    ///     The task in that view.
    ///
    /// Raises:
    ///     ValueError: `view` is not a TES view.
    ///     RuntimeError: The request failed or the response doesn't fit the TES models.
    pub fn get(&self, py: Python<'_>, view: &str, id: &str) -> PyResult<PyTesTask> {
        let view = parse_view(view)?;
        io(py, |rt| rt.block_on(get_task(&self.inner, view, id)))
    }

    /// Lists tasks, one page at a time.
    ///
    /// Args:
    ///     params: Filters and paging; by default, the first page of all tasks.
    ///
    /// Returns:
    ///     The page.
    ///
    /// Raises:
    ///     RuntimeError: The request failed or the response doesn't fit the TES models.
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
