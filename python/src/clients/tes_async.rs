//! The asyncio variants of the TES client and task classes.
//!
//! Their methods return `asyncio.Future`s of the running event loop. The requests run on the
//! shared runtime of `utils::runtime` (see `spawn_io`), without the GIL, so they don't block
//! the loop.
//! Cancelling a future drops its request. Arguments are checked and converted when a method
//! is called, results when the request is done, with the same functions as the sync classes.

use crate::clients::tes::{
    cancel_task, connect, create_task, get_task, list_endpoint, list_tasks, parse_view,
    state_to_python, task_status, task_to_create, PyListTasksParams,
};
use crate::utils::convert::check_extra_args;
// Public, so that Rust extensions can add async methods the same way.
pub use crate::utils::runtime::{future_into_py, spawn_io};
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::tes::{Task, TES};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;

/// The library client of an `AsyncTES`, connected by the first call that needs it.
pub type LazyTES = Arc<Connection>;

/// The connection of an `AsyncTES` to its service; see [`connected`].
#[derive(Default)]
pub struct Connection {
    /// The library client, once connected.
    client: OnceLock<Arc<TES>>,
    /// How many attempts to connect have failed so far.
    failures: AtomicU64,
    /// Held while connecting, with the error of the last failed attempt.
    attempt: Mutex<String>,
}

/// Gives the connected library client, connecting first if no call did yet.
///
/// Like `TES(config)`, connecting reads the service info and checks that it is a TES. One
/// call connects at a time. If an attempt fails, the calls that waited for it fail with its
/// error, and a later call tries again.
pub async fn connected(tes: &LazyTES, config: &Configuration) -> Result<Arc<TES>, String> {
    if let Some(client) = tes.client.get() {
        return Ok(client.clone());
    }
    let failures = tes.failures.load(Ordering::Acquire);
    let mut last_error = tes.attempt.lock().await;
    if let Some(client) = tes.client.get() {
        return Ok(client.clone());
    }
    if tes.failures.load(Ordering::Acquire) != failures {
        return Err(last_error.clone()); // An attempt failed while this call waited.
    }
    match connect(config).await {
        Ok(client) => Ok(tes.client.get_or_init(|| Arc::new(client)).clone()),
        Err(error) => {
            last_error.clone_from(&error);
            tes.failures.fetch_add(1, Ordering::AcqRel);
            Err(error)
        }
    }
}

/// A TES client for `asyncio`: `create`, `get` and `list` return awaitables.
///
/// `AsyncTES(config)` doesn't contact the server; its first call checks the service, as
/// `TES(config)` does. `await AsyncTES.connect(config)` checks it right away. Subclass it to
/// add endpoints, using `self.transport`.
#[pyclass(name = "AsyncTES", module = "GA4GH", subclass)]
pub struct PyAsyncTES {
    /// The client's configuration.
    pub config: Configuration,
    /// A transport with the client's configuration, for further endpoints.
    pub transport: Transport,
    /// The library client, once connected.
    pub inner: LazyTES,
}

impl PyAsyncTES {
    pub fn new(config: &PyConfiguration) -> Self {
        PyAsyncTES {
            config: config.inner.clone(),
            transport: Transport::new(&config.inner),
            inner: LazyTES::default(),
        }
    }
}

#[pymethods]
impl PyAsyncTES {
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
        Ok(Self::new(config))
    }

    /// Builds `cls(config, *args, **kwargs)` and gives it once it has checked the service.
    ///
    /// The client is built at once, on the event loop's thread. The check raises
    /// `RuntimeError` when awaited if it fails, like `TES(config)`.
    #[classmethod]
    #[pyo3(signature = (config, *args, **kwargs))]
    fn connect<'py>(
        cls: &Bound<'py, PyType>,
        config: &Bound<'py, PyConfiguration>,
        args: &Bound<'py, PyTuple>,
        kwargs: Option<&Bound<'py, PyDict>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let py = cls.py();
        // Fail as the other methods do, before building anything, without a running loop.
        py.import_bound("asyncio")?
            .call_method0("get_running_loop")?;
        let mut arguments = vec![config.clone().into_any()];
        arguments.extend(args.iter());
        let client = cls.call(PyTuple::new_bound(py, arguments), kwargs)?;
        let (tes, settings) = {
            let this = client.downcast::<Self>()?.borrow();
            (this.inner.clone(), this.config.clone())
        };
        let client = client.unbind();
        future_into_py(py, async move {
            spawn_io(move || async move { connected(&tes, &settings).await.map(drop) })
                .await
                .map_err(PyRuntimeError::new_err)?;
            Ok(client)
        })
    }

    /// The client's configuration.
    #[getter]
    fn config(&self) -> PyConfiguration {
        PyConfiguration {
            inner: self.config.clone(),
        }
    }

    /// A transport with the client's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.transport.clone(),
        }
    }

    /// Creates a task; gives an `AsyncTask` for it. Takes the same `task` as `TES.create()`.
    pub fn create<'py>(&self, task: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = task.py();
        let task = task_to_create(task, "AsyncTES.create()")?;
        let (tes, config) = (self.inner.clone(), self.config.clone());
        future_into_py(py, async move {
            let inner = spawn_io(move || async move {
                create_task(&*connected(&tes, &config).await?, task).await
            })
            .await
            .map_err(PyRuntimeError::new_err)?;
            Ok(PyAsyncTask { inner })
        })
    }

    /// Gets a task; gives a `TesTask`. `view` is `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any
    /// case; others raise `ValueError` when the method is called.
    pub fn get<'py>(
        &self,
        py: Python<'py>,
        view: String,
        id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let view = parse_view(&view)?;
        let (tes, config) = (self.inner.clone(), self.config.clone());
        future_into_py(py, async move {
            spawn_io(
                move || async move { get_task(&*connected(&tes, &config).await?, view, &id).await },
            )
            .await
            .map_err(PyRuntimeError::new_err)
        })
    }

    /// Lists tasks, one page at a time; gives a `ListTasksResponse`.
    #[pyo3(signature = (params=None))]
    pub fn list<'py>(
        &self,
        py: Python<'py>,
        params: Option<PyListTasksParams>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let endpoint = list_endpoint(params)?;
        let (tes, config) = (self.inner.clone(), self.config.clone());
        future_into_py(py, async move {
            spawn_io(move || async move {
                list_tasks(&connected(&tes, &config).await?.transport, &endpoint).await
            })
            .await
            .map_err(PyRuntimeError::new_err)
        })
    }
}

/// A TES task on the server, addressed by its ID, for `asyncio`: its methods return awaitables.
#[pyclass(name = "AsyncTask", module = "GA4GH", subclass)]
pub struct PyAsyncTask {
    pub inner: Task,
}

impl PyAsyncTask {
    pub fn new(id: String, transport: &PyTransport) -> Self {
        PyAsyncTask {
            inner: Task::new(id, transport.inner.clone()),
        }
    }
}

#[pymethods]
impl PyAsyncTask {
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
        Ok(Self::new(id, transport))
    }

    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    /// A transport with the task's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport.clone(),
        }
    }

    /// Gives the task's current state, as a `TesState`.
    pub fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let task = self.inner.clone();
        future_into_py(py, async move {
            let state = spawn_io(move || async move { task_status(&task).await })
                .await
                .map_err(PyRuntimeError::new_err)?;
            Python::with_gil(|py| state_to_python(py, state))
        })
    }

    /// Asks the server to cancel the task; gives `None`.
    pub fn cancel<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let task = self.inner.clone();
        future_into_py(py, async move {
            spawn_io(move || async move { cancel_task(&task).await })
                .await
                .map_err(PyRuntimeError::new_err)
        })
    }
}
