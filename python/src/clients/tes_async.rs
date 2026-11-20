//! The asyncio variants of the TES client and task classes.
//!
//! Their methods return `asyncio.Future`s of the running event loop. The requests run on the
//! shared runtime of `utils::runtime` (see `spawn_io`), without the GIL, so they don't block
//! the loop.
//! Cancelling a future drops its request. Arguments are checked and converted when a method
//! is called, results when the request is done, with the same functions as the sync classes.

use crate::clients::tes::{
    cancel_task, connect_with, create_task, get_task, list_endpoint, list_tasks, parse_view,
    state_to_python, task_path, task_status, task_to_create, wait_end, wait_result, wait_settings,
    PyListTasksParams, WaitEnd, WaitSettings,
};
use crate::utils::convert::check_extra_args;
// Public, so that Rust extensions can add async methods the same way.
pub use crate::utils::runtime::{future_into_py, spawn_io};
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::tes::{Task, TesView, TES};
use ga4gh_sdk::clients::wait::observe;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::operation::{OperationError, ReplaySafety};
use ga4gh_sdk::utils::transport::{BodyLimits, RequestOptions, Transport};
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use reqwest::Method;
use serde_json::Value;
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

/// Gives the connected library client, connecting it with `transport` first if no call did
/// yet.
///
/// Like `TES(config)`, connecting reads the service info and checks that it is a TES. The
/// client sends that request and all later ones with `transport`. One call connects at a
/// time. If an attempt fails, the calls that waited for it fail with its error, and a later
/// call tries again. `transport` only takes effect on the attempt that connects: once
/// connected, the client keeps its transport, so pass the transport that
/// [`PyAsyncTES::connection`] gives with the `LazyTES`.
pub async fn connected(tes: &LazyTES, transport: &Transport) -> Result<Arc<TES>, String> {
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
    match connect_with(transport.clone()).await {
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
/// The methods return `asyncio.Future`s of the running event loop, so they must be called
/// from it. `AsyncTES(config)` doesn't contact the server; `await AsyncTES.connect(config)`
/// first checks that `config` points at a TES service; otherwise the first call does.
/// Invalid arguments raise when a method is called; failed requests raise `RuntimeError`
/// when awaited. Cancelling an awaitable drops its request.
#[pyclass(name = "AsyncTES", module = "ga4gh_sdk", subclass)]
pub struct PyAsyncTES {
    /// The configuration that the client was built with, kept for compatibility. The SDK
    /// doesn't read it: the client's configuration is `transport.configuration()`.
    pub config: Configuration,
    /// The client's transport: the built-in calls connect with it, and further endpoints
    /// can use it.
    pub transport: Transport,
    /// The library client, once connected.
    pub inner: LazyTES,
}

impl PyAsyncTES {
    /// Builds a client with a transport for `config`; sends no request.
    pub fn new(config: &PyConfiguration) -> Self {
        Self::with_transport(Transport::new(&config.inner))
    }

    /// Builds a client that sends its requests with `transport`, for example one from
    /// [`Transport::with_client`]; sends no request.
    pub fn with_transport(transport: Transport) -> Self {
        PyAsyncTES {
            config: transport.configuration().clone(),
            transport,
            inner: LazyTES::default(),
        }
    }

    /// The lazy client and the transport that [`connected`] connects it with, for a call
    /// to move to [`spawn_io`].
    pub fn connection(&self) -> (LazyTES, Transport) {
        (self.inner.clone(), self.transport.clone())
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

    /// Builds `cls(config, *args, **kwargs)`; gives it once it has checked the service.
    ///
    /// Args:
    ///     config: The service's configuration.
    ///     *args: Further arguments of a subclass's constructor.
    ///     **kwargs: Further keyword arguments of a subclass's constructor.
    ///
    /// Returns:
    ///     A future of the client; it raises `RuntimeError` if the service isn't
    ///     reachable or isn't a TES.
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
        py.import("asyncio")?.call_method0("get_running_loop")?;
        let mut arguments = vec![config.clone().into_any()];
        arguments.extend(args.iter());
        let client = cls.call(PyTuple::new(py, arguments)?, kwargs)?;
        let (tes, transport) = client.cast::<Self>()?.borrow().connection();
        let client = client.unbind();
        future_into_py(py, async move {
            spawn_io(move || async move { connected(&tes, &transport).await.map(drop) })
                .await
                .map_err(PyRuntimeError::new_err)?;
            Ok(client)
        })
    }

    /// The client's configuration.
    #[getter]
    fn config(&self) -> PyConfiguration {
        PyConfiguration {
            inner: self.transport.configuration().clone(),
        }
    }

    /// A transport with the client's configuration; its methods are synchronous.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.transport.clone(),
        }
    }

    /// Creates a task; takes the same `task` as `TES.create()`.
    ///
    /// Args:
    ///     task: A `TesTask`, a `dict` (or other mapping), or the path of a JSON file.
    ///
    /// Returns:
    ///     A future of the created task.
    ///
    /// Raises:
    ///     ValueError: The task is invalid.
    ///     OSError: The task file can't be read.
    pub fn create<'py>(&self, task: &Bound<'py, PyAny>) -> PyResult<Bound<'py, PyAny>> {
        let py = task.py();
        let task = task_to_create(task, "AsyncTES.create()")?;
        let (tes, transport) = self.connection();
        future_into_py(py, async move {
            let inner = spawn_io(move || async move {
                create_task(&*connected(&tes, &transport).await?, task).await
            })
            .await
            .map_err(PyRuntimeError::new_err)?;
            Ok(PyAsyncTask { inner })
        })
    }

    /// Gets a task.
    ///
    /// Args:
    ///     view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case.
    ///     id: The task's ID.
    ///
    /// Returns:
    ///     A future of the task in that view.
    ///
    /// Raises:
    ///     ValueError: `view` is not a TES view.
    pub fn get<'py>(
        &self,
        py: Python<'py>,
        view: String,
        id: String,
    ) -> PyResult<Bound<'py, PyAny>> {
        let view = parse_view(&view)?;
        let (tes, transport) = self.connection();
        future_into_py(py, async move {
            spawn_io(move || async move {
                get_task(&*connected(&tes, &transport).await?, view, &id).await
            })
            .await
            .map_err(PyRuntimeError::new_err)
        })
    }

    /// Lists tasks, one page at a time.
    ///
    /// Args:
    ///     params: Filters and paging; by default, the first page of all tasks.
    ///
    /// Returns:
    ///     A future of the page.
    #[pyo3(signature = (params=None))]
    pub fn list<'py>(
        &self,
        py: Python<'py>,
        params: Option<PyListTasksParams>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let endpoint = list_endpoint(params)?;
        let (tes, transport) = self.connection();
        future_into_py(py, async move {
            spawn_io(move || async move {
                list_tasks(connected(&tes, &transport).await?.transport(), &endpoint).await
            })
            .await
            .map_err(PyRuntimeError::new_err)
        })
    }
}

/// A TES task on the server, addressed by its ID, for `asyncio`.
#[pyclass(name = "AsyncTask", module = "ga4gh_sdk", subclass)]
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

    /// The task's ID.
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

    /// Gets the task's current state.
    ///
    /// Returns:
    ///     A future of the state; it raises `RuntimeError` if the request fails.
    pub fn status<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let task = self.inner.clone();
        future_into_py(py, async move {
            let state = spawn_io(move || async move { task_status(&task).await })
                .await
                .map_err(PyRuntimeError::new_err)?;
            Python::attach(|py| state_to_python(py, state))
        })
    }

    /// Asks the server to cancel the task.
    ///
    /// Returns:
    ///     A future of `None`; it raises `RuntimeError` if the request fails.
    pub fn cancel<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        let task = self.inner.clone();
        future_into_py(py, async move {
            spawn_io(move || async move { cancel_task(&task).await })
                .await
                .map_err(PyRuntimeError::new_err)?;
            Ok(None::<()>)
        })
    }

    /// Waits until the task ends, like `Task.wait()`.
    ///
    /// Cancelling the awaiting coroutine stops polling; it doesn't cancel the task.
    /// Between polls, the wait holds no call thread, so sleeping waits do not delay other
    /// async calls.
    ///
    /// Args:
    ///     timeout: The most seconds for the whole wait; `None` waits as long as it takes.
    ///     poll_interval: The seconds between the first polls.
    ///     max_interval: The longest interval between polls; by default 10 seconds, or
    ///         `poll_interval` if that's longer.
    ///     view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, to get the ended task in that view.
    ///
    /// Returns:
    ///     A future of the final state, or of the ended task in `view`. It raises
    ///     `WaitTimeoutError` when the time runs out and `RuntimeError` if a poll fails.
    ///
    /// Raises:
    ///     ValueError: An argument is invalid, when the method is called.
    #[pyo3(signature = (timeout=None, *, poll_interval=1.0, max_interval=None, view=None))]
    pub fn wait<'py>(
        &self,
        py: Python<'py>,
        timeout: Option<f64>,
        poll_interval: f64,
        max_interval: Option<f64>,
        view: Option<&str>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let settings = wait_settings(timeout, poll_interval, max_interval, view)?;
        let task = self.inner.clone();
        let id = task.id.clone();
        future_into_py(py, async move {
            let end = wait_task_send(task, settings).await;
            Python::attach(|py| wait_result(py, &id, end))
        })
    }
}

/// Why a poll on a call thread failed.
enum PollError {
    /// The wait's deadline or cancellation, kept for the waiter to tell apart.
    Operation(OperationError),
    /// Any other error, as its message.
    Other(String),
}

/// Polls the task in the minimal view on a call thread, as the library's waiter does.
///
/// The future is `Send`: it takes a call thread only while the request runs.
async fn poll_on_call_thread(task: Task, options: RequestOptions) -> ga4gh_sdk::Result<Value> {
    let polled = spawn_io(move || async move {
        let params = serde_json::json!({ "view": TesView::Minimal });
        let result = task
            .transport
            .request_with_policy(Method::GET, &task_path(&task), None, Some(params), &options)
            .await;
        Ok(match result {
            Ok(body) => {
                serde_json::from_str::<Value>(&body).map_err(|e| PollError::Other(e.to_string()))
            }
            Err(error) => Err(match error.downcast_ref::<OperationError>() {
                Some(error) => PollError::Operation(*error),
                None => PollError::Other(error.to_string()),
            }),
        })
    })
    .await;
    match polled {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(PollError::Operation(error))) => Err(error.into()),
        Ok(Err(PollError::Other(error))) | Err(error) => Err(error.into()),
    }
}

/// Waits for the task like [`crate::clients::tes::wait_task`], with the library's state
/// machine ([`observe`]), as a `Send` future.
///
/// The SDK's futures, and so `Task::wait`, aren't `Send`: `wait_task` would hold a call
/// thread for the whole wait. This one sleeps on the runtime between polls and takes a call
/// thread (see [`spawn_io`]) only for each request. The wait's deadline and cancellation
/// hold on the runtime, also while a request waits for a call thread. Like the sync wait,
/// it doesn't fetch failure logs (`WaitOptions::fetch_failure_logs` is off).
pub async fn wait_task_send(task: Task, settings: WaitSettings) -> WaitEnd {
    let WaitSettings { options, view } = settings;
    let request = RequestOptions {
        operation: options.operation.clone(),
        retry: options.retry.clone(),
        safety: ReplaySafety::ReadOnly,
        limits: BodyLimits::default(),
    };
    let result = observe(task.remote_handle(), &options, || {
        poll_on_call_thread(task.clone(), request.clone())
    })
    .await;
    wait_end(result, &options, view, |view| {
        let task = task.clone();
        spawn_io(move || async move {
            let params = serde_json::json!({ "view": view });
            task.transport
                .get(&task_path(&task), Some(params))
                .await
                .map_err(|error| error.to_string())
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::{mock, Matcher, Mock};
    use url::Url;

    /// The user agent of the transport that a test gives the client.
    const OWNED_AGENT: &str = "owned-transport";

    /// A configuration for the API root `/<root>/ga4gh/tes/v1` on the mock server.
    fn config(root: &str) -> Configuration {
        let root = format!("{}/{root}/ga4gh/tes/v1", mockito::server_url());
        Configuration::new(Url::parse(&root).unwrap())
    }

    /// A client built with a transport whose user agent is [`OWNED_AGENT`].
    fn client_with_owned_transport(root: &str) -> PyAsyncTES {
        let config = config(root).with_user_agent(OWNED_AGENT.to_owned());
        PyAsyncTES::with_transport(Transport::new(&config))
    }

    /// A mock of the service info of a TES beneath `root` that expects one request with
    /// [`OWNED_AGENT`]; call `create()` on it to serve it.
    fn service_info(root: &str, status: usize) -> Mock {
        mock("GET", format!("/{root}/ga4gh/tes/v1/service-info").as_str())
            .match_header("user-agent", OWNED_AGENT)
            .with_status(status)
            .with_body(
                serde_json::json!({
                    "id": "org.example.tes",
                    "name": "Example TES",
                    "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
                    "organization": {"name": "Example", "url": "https://example.org"},
                    "version": "1.0.0"
                })
                .to_string(),
            )
            .expect(1)
    }

    #[tokio::test]
    async fn lazy_client_shares_transport() {
        let info = service_info("shared-transport", 200).create();
        let created = mock("POST", "/shared-transport/ga4gh/tes/v1/tasks")
            .match_header("user-agent", OWNED_AGENT)
            .match_body(Matcher::Json(serde_json::json!({"executors": []})))
            .with_status(200)
            .with_body(r#"{"id":"task-1"}"#)
            .expect(1)
            .create();
        let fetched = mock(
            "GET",
            "/shared-transport/ga4gh/tes/v1/tasks/task-1?view=MINIMAL",
        )
        .match_header("user-agent", OWNED_AGENT)
        .with_status(200)
        .with_body(r#"{"id":"task-1","state":"QUEUED"}"#)
        .expect(1)
        .create();
        let mut client = client_with_owned_transport("shared-transport");
        // A Rust extension can still change the public copy; the client doesn't read it.
        client.config = config("shared-transport");
        let (tes, transport) = client.connection();

        let connected_client = connected(&tes, &transport).await.unwrap();
        let task = create_task(&connected_client, serde_json::json!({"executors": []}))
            .await
            .unwrap();
        get_task(&connected_client, TesView::Minimal, "task-1")
            .await
            .unwrap();

        info.assert();
        created.assert();
        fetched.assert();
        assert_eq!(task.id, "task-1");
        assert_eq!(
            client.config().inner.user_agent.as_deref(),
            Some(OWNED_AGENT)
        );
    }

    #[tokio::test]
    async fn concurrent_first_calls_connect_once_with_the_transport() {
        let info = service_info("concurrent-transport", 200).create();
        let client = client_with_owned_transport("concurrent-transport");
        let (tes, transport) = client.connection();

        let (first, second) =
            tokio::join!(connected(&tes, &transport), connected(&tes, &transport));

        info.assert();
        assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
    }

    #[tokio::test]
    async fn failed_connection_is_retried_with_the_same_transport() {
        let client = client_with_owned_transport("retried-transport");
        let (tes, transport) = client.connection();
        let failed = service_info("retried-transport", 500).create();

        let error = connected(&tes, &transport).await.err().unwrap();

        failed.assert();
        assert!(
            error.starts_with("Failed to create TES instance"),
            "{error}"
        );
        drop(failed);
        let info = service_info("retried-transport", 200).create();

        connected(&tes, &transport).await.unwrap();

        info.assert();
    }

    #[tokio::test]
    async fn client_with_transport_connects_with_the_injected_http_client() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-http-client", "injected".parse().unwrap());
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap();
        let config = config("injected-client").with_user_agent(OWNED_AGENT.to_owned());
        let client = PyAsyncTES::with_transport(Transport::with_client(&config, http));
        let info = service_info("injected-client", 200)
            .match_header("x-http-client", "injected")
            .create();
        let (tes, transport) = client.connection();

        connected(&tes, &transport).await.unwrap();

        info.assert();
        for configuration in [&client.config, &client.config().inner] {
            assert_eq!(configuration.base_path, config.base_path);
            assert_eq!(configuration.user_agent.as_deref(), Some(OWNED_AGENT));
        }
    }
}
