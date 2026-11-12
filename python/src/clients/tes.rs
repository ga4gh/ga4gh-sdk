use crate::utils::convert::{check_extra_args, read_file, to_json, to_python, with_query};
use crate::utils::runtime::io;
use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::models::TesListTasksResponse;
use ga4gh_sdk::clients::tes::models::TesState;
use ga4gh_sdk::clients::tes::models::TesTask;
use ga4gh_sdk::clients::tes::Task;
use ga4gh_sdk::clients::tes::TesView;
use ga4gh_sdk::clients::tes::TES;
use ga4gh_sdk::clients::wait::{WaitOptions, WaitOutcome, WaitResult};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::operation::OperationError;
use ga4gh_sdk::utils::transport::{urlencode, Transport};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::{PyDict, PyMapping, PyString, PyTuple, PyType};
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn invalid_response_text(error: impl std::fmt::Display) -> String {
    format!("Invalid TES response: {error}")
}

fn invalid_response(error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(invalid_response_text(error))
}

/// Parses a TES view name, in any case; raises `ValueError` for others.
pub fn parse_view(view: &str) -> PyResult<TesView> {
    view.parse::<TesView>()
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Python source of the `TesState` enum. Its members must match the variants of `TesState`.
const TES_STATE_SOURCE: &str = r#"
import enum


class TesState(str, enum.Enum):
    """The state of a TES task.

    Members are strings, so `TesState.RUNNING == "RUNNING"`, and `str()`, f-strings and
    `json.dumps()` give the state's name. `TesState("RUNNING")` looks a state up by name.
    """

    __module__ = "ga4gh_sdk"

    UNKNOWN = "UNKNOWN"
    QUEUED = "QUEUED"
    INITIALIZING = "INITIALIZING"
    RUNNING = "RUNNING"
    PAUSED = "PAUSED"
    COMPLETE = "COMPLETE"
    EXECUTOR_ERROR = "EXECUTOR_ERROR"
    SYSTEM_ERROR = "SYSTEM_ERROR"
    CANCELED = "CANCELED"
    PREEMPTED = "PREEMPTED"
    CANCELING = "CANCELING"

    def __str__(self):
        return self.value

    def __format__(self, format_spec):
        return format(self.value, format_spec)
"#;

static TES_STATE: GILOnceCell<Py<PyType>> = GILOnceCell::new();

/// The `TesState` Python enum (a `str` enum, created once per interpreter).
pub fn tes_state_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    TES_STATE
        .get_or_try_init(py, || define_type(py, TES_STATE_SOURCE, "TesState"))
        .map(|cls| cls.bind(py))
}

/// Runs the Python `source` of a class and gives the class `name` that it defines.
fn define_type(py: Python<'_>, source: &str, name: &str) -> PyResult<Py<PyType>> {
    // Runs in a namespace of its own. `PyModule::from_code_bound` would register a
    // module in `sys.modules` and, under the name `ga4gh_sdk`, overwrite the package's
    // attributes; the class itself names `ga4gh_sdk` as its module.
    let namespace = PyDict::new_bound(py);
    namespace.set_item("__name__", "ga4gh_sdk")?;
    py.run_bound(source, Some(&namespace), None)?;
    Ok(namespace
        .get_item(name)?
        .ok_or_else(|| PyRuntimeError::new_err(format!("{name} is not defined")))?
        .downcast_into::<PyType>()?
        .unbind())
}

/// Converts a TES state to the matching `TesState` member.
pub fn state_to_python(py: Python<'_>, state: TesState) -> PyResult<PyObject> {
    Ok(tes_state_type(py)?
        .getattr(state.to_string().as_str())?
        .unbind())
}

/// Parses a TES state name, in any case.
fn parse_state(state: &str) -> PyResult<TesState> {
    let name = state.to_uppercase();
    // `TesState` reads states that it doesn't know as `Unknown`; only exact
    // names are valid here.
    serde_json::from_value::<TesState>(Value::String(name.clone()))
        .ok()
        .filter(|parsed| parsed.to_string() == name)
        .ok_or_else(|| PyValueError::new_err(format!("Invalid TES state: {state}")))
}

// The TES calls, shared by the sync classes (`TES`, `Task`) and the async ones (`AsyncTES`,
// `AsyncTask`). The plain functions need the GIL and only prepare a request. The `async`
// ones send it and convert the result without the GIL; they hold only owned, `Send` data
// and give errors as text (the SDK's boxed errors are not `Send`), which the callers raise
// as `RuntimeError`.

/// Connects to a TES service: reads its service info and checks that it is a TES.
pub async fn connect(config: &Configuration) -> Result<TES, String> {
    TES::new(config)
        .await
        .map_err(|error| format!("Failed to create TES instance: {error}"))
}

/// The JSON of a task to create, from the argument of `create()`.
///
/// `task` is a `TesTask`, a `dict` (or other JSON-serialisable object), or the path of a
/// JSON file as `str` or `os.PathLike`. A `str` is always a path. `method` names the
/// caller in errors, for example `"TES.create()"`.
pub fn task_to_create(task: &Bound<'_, PyAny>, method: &str) -> PyResult<Value> {
    if let Ok(text) = task.downcast::<PyString>() {
        // Lossy, so that paths with surrogate escapes (non-UTF-8 file names) pass.
        if text.to_string_lossy().trim_start().starts_with('{') {
            return Err(PyValueError::new_err(format!(
                "{method} takes a str as a file path; pass JSON text as \
                 TesTask.from_json(text), or a path that starts with '{{' as a \
                 pathlib.Path"
            )));
        }
    }
    let task = if task.is_instance_of::<PyString>() || task.hasattr("__fspath__")? {
        PyTesTask::from_path(task.py(), &task.extract::<PathBuf>()?)?
    } else {
        PyTesTask::from_python(task)?
    };
    Ok(task.raw)
}

/// Creates a task from its JSON, sent as given.
pub async fn create_task(tes: &TES, task: Value) -> Result<Task, String> {
    tes.create_value(task)
        .await
        .map_err(|error| format!("Failed to create task: {error}"))
}

/// Gets a task in a view parsed by [`parse_view`].
pub async fn get_task(tes: &TES, view: TesView, id: &str) -> Result<PyTesTask, String> {
    let response = tes
        .get_raw(view, id)
        .await
        .map_err(|error| format!("Failed to get task: {error}"))?;
    let raw = serde_json::from_str(&response).map_err(invalid_response_text)?;
    PyTesTask::parse(raw).map_err(invalid_response_text)
}

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

/// The task's current state.
pub async fn task_status(task: &Task) -> Result<TesState, String> {
    task.status()
        .await
        .map_err(|e| format!("Failed to get task status: {e}"))
}

/// Asks the server to cancel the task.
pub async fn cancel_task(task: &Task) -> Result<(), String> {
    task.cancel()
        .await
        .map(drop)
        .map_err(|e| format!("Failed to cancel task: {e}"))
}

/// Python source of the `WaitTimeoutError` exception.
const WAIT_TIMEOUT_ERROR_SOURCE: &str = r#"
class WaitTimeoutError(TimeoutError):
    """Raised when `Task.wait()` or `AsyncTask.wait()` runs out of time.

    `id` is the task's ID, and `last_state` its last polled state as a `TesState`, or
    `None` if no poll succeeded. The wait doesn't cancel the task, which may still be
    running (if the time ran out while getting an ended task in a view, `last_state` is
    its final state). Both are `None` on an error raised by other code.
    """

    __module__ = "ga4gh_sdk"

    id = None
    last_state = None
"#;

static WAIT_TIMEOUT_ERROR: GILOnceCell<Py<PyType>> = GILOnceCell::new();

/// The `WaitTimeoutError` Python exception, a `TimeoutError` (created once per interpreter).
pub fn wait_timeout_error_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    WAIT_TIMEOUT_ERROR
        .get_or_try_init(py, || {
            define_type(py, WAIT_TIMEOUT_ERROR_SOURCE, "WaitTimeoutError")
        })
        .map(|cls| cls.bind(py))
}

/// How long a sync wait blocks without the GIL before it checks for signals, such as Ctrl-C.
const SIGNAL_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// The settings of a wait, from the arguments of `wait()`.
pub struct WaitSettings {
    /// The library waiter's options; their deadline counts from the call of `wait()`.
    pub options: WaitOptions,
    /// The view in which to give the ended task, or `None` to give its state.
    pub view: Option<TesView>,
}

/// A duration in seconds from Python; `name` names the argument in errors.
fn seconds(name: &str, value: f64) -> PyResult<Duration> {
    Duration::try_from_secs_f64(value).map_err(|_| {
        PyValueError::new_err(format!(
            "{name} must be a finite number of seconds, at least 0, not {value}"
        ))
    })
}

/// Checks the arguments of `wait()` and builds its settings; errors are `ValueError`s.
///
/// `timeout` is the time for the whole wait (`None` waits as long as it takes). Polls
/// start `poll_interval` apart, and the interval doubles up to `max_interval` (by default
/// 10 seconds, or `poll_interval` if that's longer).
pub fn wait_settings(
    timeout: Option<f64>,
    poll_interval: f64,
    max_interval: Option<f64>,
    view: Option<&str>,
) -> PyResult<WaitSettings> {
    let budget = timeout
        .map(|timeout| seconds("timeout", timeout))
        .transpose()?
        .unwrap_or(Duration::MAX);
    let poll_interval = seconds("poll_interval", poll_interval)?;
    if poll_interval.is_zero() {
        return Err(PyValueError::new_err("poll_interval must be more than 0"));
    }
    let view = view.map(parse_view).transpose()?;
    let mut options = WaitOptions::new(budget);
    let max_interval = match max_interval {
        Some(max_interval) => seconds("max_interval", max_interval)?,
        // The library's default (10 seconds), unless `poll_interval` is longer.
        None => options.max_interval.max(poll_interval),
    };
    if max_interval < poll_interval {
        return Err(PyValueError::new_err(
            "max_interval must be at least poll_interval",
        ));
    }
    options.poll_interval = poll_interval;
    options.max_interval = max_interval;
    Ok(WaitSettings { options, view })
}

/// How a wait ended, as owned, `Send` data; [`wait_result`] converts it for Python.
pub enum WaitEnd {
    /// The task reached this terminal state.
    State(TesState),
    /// The task ended; this is the task in the view asked for.
    Task(Box<PyTesTask>),
    /// The time ran out; the last polled state, if a poll succeeded.
    Timeout(Option<TesState>),
    /// The wait was stopped locally.
    Interrupted,
    /// A poll or the lookup of the ended task failed.
    Failed(String),
}

/// Reads a state name from a TES response; names that TES 1.1 doesn't define read as
/// `Unknown`.
fn read_state(state: &str) -> TesState {
    serde_json::from_value(Value::String(state.to_owned())).unwrap_or_default()
}

/// The path of a task's lookup.
pub fn task_path(task: &Task) -> String {
    format!("tasks/{}", urlencode(&task.id))
}

/// Waits for the task with the library's waiter until it ends, the time runs out or the
/// settings' operation is cancelled. Never cancels the task.
///
/// This is the sync `Task.wait()`. The future is not `Send`, so `AsyncTask.wait()` uses
/// `tes_async::wait_task_send` instead, which shares [`wait_end`].
pub async fn wait_task(task: &Task, settings: WaitSettings) -> WaitEnd {
    let WaitSettings { options, view } = settings;
    let result = task.wait(&options).await;
    wait_end(result, &options, view, |view| async move {
        let params = serde_json::json!({ "view": view });
        task.transport
            .get(&task_path(task), Some(params))
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

/// Converts the library waiter's result into the end of the wait.
///
/// With a `view` other than the minimal one, `get_view` gets the ended task's JSON in that
/// view, within the deadline of `options`. The future is `Send` if `get_view`'s is.
pub async fn wait_end<G, Fut>(
    result: WaitResult,
    options: &WaitOptions,
    view: Option<TesView>,
    get_view: G,
) -> WaitEnd
where
    G: FnOnce(TesView) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let last_state = result.last_state.as_deref().map(read_state);
    match result.outcome {
        WaitOutcome::Complete
        | WaitOutcome::ExecutorError
        | WaitOutcome::SystemError
        | WaitOutcome::Canceled
        | WaitOutcome::Preempted => {}
        WaitOutcome::Deadline => return WaitEnd::Timeout(last_state),
        WaitOutcome::Interrupted => return WaitEnd::Interrupted,
        WaitOutcome::PollingFailed => {
            let error = result.error.unwrap_or_else(|| "unknown error".to_owned());
            return WaitEnd::Failed(format!("Failed to wait for task: {error}"));
        }
    }
    // The waiter ends only on states that TES 1.1 defines.
    let state = last_state.unwrap_or_default();
    let raw = match (view, result.last_response) {
        (None, _) => return WaitEnd::State(state),
        // The waiter polls the minimal view; no need to ask again.
        (Some(TesView::Minimal), Some(raw)) => Ok(raw),
        (Some(view), _) => match options.operation.run(get_view(view)).await {
            Ok(Ok(body)) => serde_json::from_str(&body).map_err(invalid_response_text),
            Ok(Err(error)) => Err(format!("Failed to get task: {error}")),
            Err(OperationError::Deadline) => return WaitEnd::Timeout(Some(state)),
            Err(OperationError::Cancelled) => return WaitEnd::Interrupted,
        },
    };
    match raw.and_then(|raw| PyTesTask::parse(raw).map_err(invalid_response_text)) {
        Ok(task) => WaitEnd::Task(Box::new(task)),
        Err(error) => WaitEnd::Failed(error),
    }
}

/// Converts the end of a wait for the task `id` into its Python result or exception.
pub fn wait_result(py: Python<'_>, id: &str, end: WaitEnd) -> PyResult<PyObject> {
    match end {
        WaitEnd::State(state) => state_to_python(py, state),
        WaitEnd::Task(task) => Ok((*task).into_py(py)),
        WaitEnd::Timeout(last_state) => {
            let last_state = match last_state {
                Some(state) => state_to_python(py, state)?,
                None => py.None(),
            };
            let message = format!("Timed out waiting for task {id}");
            let error = wait_timeout_error_type(py)?.call1((message,))?;
            error.setattr("id", id)?;
            error.setattr("last_state", last_state)?;
            Err(PyErr::from_value_bound(error))
        }
        // Not expected: the sync wait raises the signal handler's exception instead, and
        // the async one drops the waiter rather than cancelling it.
        WaitEnd::Interrupted => Err(PyRuntimeError::new_err("The wait was stopped locally")),
        WaitEnd::Failed(error) => Err(PyRuntimeError::new_err(error)),
    }
}

/// A TES task on the server, addressed by its ID.
#[pyclass(name = "Task", module = "ga4gh_sdk", subclass)]
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

    /// The task's current state, as a `TesState`.
    pub fn status(&self, py: Python<'_>) -> PyResult<PyObject> {
        let state = io(py, |rt| rt.block_on(task_status(&self.inner)))?;
        state_to_python(py, state)
    }

    /// Asks the server to cancel the task. Returns `None`.
    pub fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        io(py, |rt| rt.block_on(cancel_task(&self.inner)))
    }

    /// Waits until the task ends. Returns its final state as a `TesState`, or, with a
    /// `view` (`"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case), the task in that view as
    /// a `TesTask`.
    ///
    /// A task ends in `COMPLETE`, `EXECUTOR_ERROR`, `SYSTEM_ERROR`, `CANCELED` or
    /// `PREEMPTED`. Polls start `poll_interval` seconds apart; the interval doubles up to
    /// `max_interval` (by default 10 seconds, or `poll_interval` if that's longer).
    /// `timeout` is the time in seconds for the whole wait; `None` waits as long as it
    /// takes. Raises `WaitTimeoutError` when the time runs out, `RuntimeError` if a poll
    /// fails, and `ValueError` for invalid arguments. The wait releases the GIL, and
    /// Ctrl-C stops it with `KeyboardInterrupt`. None of these cancels the task.
    #[pyo3(signature = (timeout=None, *, poll_interval=1.0, max_interval=None, view=None))]
    pub fn wait(
        &self,
        py: Python<'_>,
        timeout: Option<f64>,
        poll_interval: f64,
        max_interval: Option<f64>,
        view: Option<&str>,
    ) -> PyResult<PyObject> {
        let settings = wait_settings(timeout, poll_interval, max_interval, view)?;
        let cancellation = settings.options.operation.cancellation.clone();
        // Python runs signal handlers in the main thread only; other threads just wait.
        let threading = py.import_bound("threading")?;
        // Idents, since `current_thread()` would register threads that Python didn't start.
        let main_thread = threading
            .call_method0("get_ident")?
            .eq(threading.call_method0("main_thread")?.getattr("ident")?)?;
        let (end, interrupt) = io(py, |rt| {
            let waiting = wait_task(&self.inner, settings);
            tokio::pin!(waiting);
            if !main_thread {
                return Ok((rt.block_on(waiting), None));
            }
            loop {
                let slice =
                    async { tokio::time::timeout(SIGNAL_CHECK_INTERVAL, &mut waiting).await };
                if let Ok(end) = rt.block_on(slice) {
                    return Ok((end, None));
                }
                // Outside `block_on`, so that handlers can make sync calls of their own.
                if let Err(error) = Python::with_gil(|py| py.check_signals()) {
                    // Stops the waiter, which then ends at once.
                    cancellation.cancel();
                    return Ok((rt.block_on(waiting), Some(error)));
                }
            }
        })?;
        match interrupt {
            Some(error) => Err(error),
            None => wait_result(py, &self.inner.id, end),
        }
    }
}

/// A TES task: one to create, or one as returned by the server.
///
/// `json()` and `to_dict()` return the task's JSON as given or received, including fields
/// that the TES models don't know (object keys may come back in another order).
#[pyclass(name = "TesTask", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTesTask {
    /// The task parsed into the TES model; derived from `raw`.
    pub inner: TesTask,
    /// The task's JSON as given or received. Build both fields with `from_json` or
    /// `from_value` to keep them in step.
    pub raw: Value,
}

impl PyTesTask {
    fn parse(raw: Value) -> Result<Self, String> {
        if !raw.is_object() {
            return Err("not a JSON object".to_owned());
        }
        let inner = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
        Ok(PyTesTask { inner, raw })
    }

    /// Builds a task from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }

    /// Builds a task from a caller's JSON, keeping the JSON. Errors are `ValueError`s.
    pub fn from_value(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(|e| PyValueError::new_err(format!("Invalid task: {e}")))
    }

    /// Builds a task from a `TesTask`, a JSON-serialisable object or JSON text.
    pub fn from_python(task: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(task) = task.downcast::<PyTesTask>() {
            return Ok(task.borrow().clone());
        }
        if let Ok(mapping) = task.downcast::<PyMapping>() {
            if !task.is_instance_of::<PyDict>() {
                // `json.dumps` takes only `dict`s, so copy other mappings into one.
                let dict = PyDict::new_bound(task.py());
                dict.update(mapping)?;
                return Self::from_value(to_json(&dict, "task")?);
            }
        }
        Self::from_value(to_json(task, "task")?)
    }

    /// Reads a task from a JSON file.
    pub fn from_path(py: Python<'_>, path: &Path) -> PyResult<Self> {
        let text = read_file(py, path)?;
        let raw = serde_json::from_str(&text).map_err(|e| {
            PyValueError::new_err(format!("Invalid task JSON in {}: {e}", path.display()))
        })?;
        Self::from_value(raw)
    }

    fn field(&self, py: Python<'_>, key: &str) -> PyResult<Option<PyObject>> {
        match self.raw.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => to_python(py, value).map(Some),
        }
    }
}

#[pymethods]
impl PyTesTask {
    /// `task` is a `dict` (or other JSON-serialisable object), JSON text or a `TesTask`.
    #[new]
    #[classmethod]
    #[pyo3(signature = (task, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        task: &Bound<'_, PyAny>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::from_python(task)
    }

    /// Builds a task from a mapping, such as the result of `to_dict()`.
    ///
    /// Like `from_json` and `from_file`, it calls `cls(task)`, so a subclass's further
    /// constructor arguments need defaults.
    #[classmethod]
    fn from_dict<'py>(
        cls: &Bound<'py, PyType>,
        task: &Bound<'py, PyMapping>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new_bound(cls.py());
        dict.update(task)?;
        cls.call1((dict,))
    }

    /// Builds a task from JSON text, such as the result of `json()`.
    #[classmethod]
    #[pyo3(name = "from_json")]
    fn py_from_json<'py>(
        cls: &Bound<'py, PyType>,
        text: &Bound<'py, PyString>,
    ) -> PyResult<Bound<'py, PyAny>> {
        cls.call1((text,))
    }

    /// Reads a task from a JSON file. `path` is a `str` or `os.PathLike`.
    #[classmethod]
    fn from_file<'py>(cls: &Bound<'py, PyType>, path: PathBuf) -> PyResult<Bound<'py, PyAny>> {
        let task = Self::from_path(cls.py(), &path)?;
        cls.call1((to_python(cls.py(), &task.raw)?,))
    }

    /// Lets `pickle` and `copy` rebuild the task as `type(task)(task.to_dict())`, with the
    /// attributes that a subclass instance holds in `__dict__`.
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyType>, (PyObject,), Option<Bound<'py, PyAny>>)> {
        let raw = to_python(slf.py(), &slf.borrow().raw)?;
        let state = slf.getattr("__dict__").ok();
        Ok((slf.get_type(), (raw,), state))
    }

    #[getter]
    pub fn id(&self) -> Option<String> {
        self.inner.id.clone()
    }

    /// The task's state as a `TesState`; `UNKNOWN` if the task has none or one that
    /// TES 1.1 doesn't define.
    #[getter]
    pub fn state(&self, py: Python<'_>) -> PyResult<PyObject> {
        state_to_python(py, self.inner.state.unwrap_or_default())
    }

    /// Same as `state`.
    #[getter]
    pub fn status(&self, py: Python<'_>) -> PyResult<PyObject> {
        self.state(py)
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name.clone()
    }

    #[getter]
    fn description(&self) -> Option<String> {
        self.inner.description.clone()
    }

    #[getter]
    fn creation_time(&self) -> Option<String> {
        self.inner.creation_time.clone()
    }

    #[getter]
    fn inputs(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "inputs")
    }

    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "outputs")
    }

    #[getter]
    fn resources(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "resources")
    }

    #[getter]
    fn executors(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "executors")
    }

    #[getter]
    fn volumes(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "volumes")
    }

    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "tags")
    }

    #[getter]
    fn logs(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        self.field(py, "logs")
    }

    /// The task as JSON text, as given or received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The task as a new `dict`, as given or received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let state = slf.borrow().inner.state.unwrap_or_default();
        Ok(format!("{name}(id={id}, state='{state}')"))
    }
}

/// Filters and paging for `TES.list()`.
#[pyclass(name = "ListTasksParams", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyListTasksParams {
    pub inner: ListTasksParams,
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
    fn state(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
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
    pub inner: TesListTasksResponse,
    pub tasks: Vec<PyTesTask>,
    /// The response's JSON as received.
    pub raw: Value,
}

impl PyTesListTasksResponse {
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
    pub fn tasks(&self) -> Vec<PyTesTask> {
        self.tasks.clone()
    }

    #[getter]
    pub fn next_page_token(&self) -> Option<String> {
        self.inner.next_page_token.clone()
    }

    /// The response as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The response as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }
}

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
        Python::with_gil(|py| Self::new_with_gil(py, config))
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
            rt.block_on(list_tasks(&self.inner.transport, &endpoint))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ga4gh_sdk::clients::wait::{RemoteHandle, ServiceKind};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A waiter's result for a task that ended in `state`, with its minimal view.
    fn ended(state: &str) -> WaitResult {
        WaitResult {
            handle: RemoteHandle {
                service: ServiceKind::Tes,
                api_root: "http://localhost/".to_owned(),
                id: "task-1".to_owned(),
            },
            outcome: WaitOutcome::Complete,
            last_state: Some(state.to_owned()),
            last_response: Some(serde_json::json!({ "id": "task-1", "state": state })),
            details: None,
            error: None,
        }
    }

    /// Runs `future` on a runtime of its own.
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn the_minimal_view_reuses_the_last_poll() {
        let options = WaitOptions::new(Duration::from_secs(5));
        let asked = AtomicBool::new(false);
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Minimal),
            |_| async {
                asked.store(true, Ordering::SeqCst);
                Ok(String::new())
            },
        ));
        assert!(matches!(end, WaitEnd::Task(task) if task.raw["id"] == "task-1"));
        assert!(!asked.load(Ordering::SeqCst));
    }

    #[test]
    fn other_views_give_the_task_from_the_view_request() {
        let options = WaitOptions::new(Duration::from_secs(5));
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Full),
            |view| async move {
                assert_eq!(view, TesView::Full);
                Ok(r#"{"id": "task-1", "state": "COMPLETE", "logs": []}"#.to_owned())
            },
        ));
        assert!(matches!(end, WaitEnd::Task(task) if task.raw.get("logs").is_some()));
    }

    #[test]
    fn a_failed_view_request_fails_the_wait() {
        let options = WaitOptions::new(Duration::from_secs(5));
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Full),
            |_| async { Err("HTTP 500".to_owned()) },
        ));
        assert!(matches!(end, WaitEnd::Failed(error) if error == "Failed to get task: HTTP 500"));
    }

    #[test]
    fn the_deadline_during_the_view_request_keeps_the_final_state() {
        let options = WaitOptions::new(Duration::from_millis(10));
        let end = block_on(wait_end(
            ended("EXECUTOR_ERROR"),
            &options,
            Some(TesView::Basic),
            |_| std::future::pending(),
        ));
        assert!(matches!(
            end,
            WaitEnd::Timeout(Some(TesState::ExecutorError))
        ));
    }

    #[test]
    fn cancellation_during_the_view_request_interrupts_the_wait() {
        let options = WaitOptions::new(Duration::from_secs(5));
        options.operation.cancellation.cancel();
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Basic),
            |_| std::future::pending(),
        ));
        assert!(matches!(end, WaitEnd::Interrupted));
    }
}
