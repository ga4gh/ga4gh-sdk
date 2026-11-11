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
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::{PyDict, PyMapping, PyString, PyTuple, PyType};
use serde_json::Value;
use std::path::{Path, PathBuf};

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

    __module__ = "GA4GH"

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
        .get_or_try_init(py, || {
            // Runs in a namespace of its own. `PyModule::from_code_bound` would register a
            // module in `sys.modules` and, under the name `GA4GH`, overwrite the package's
            // attributes; the class itself names `GA4GH` as its module.
            let namespace = PyDict::new_bound(py);
            namespace.set_item("__name__", "GA4GH")?;
            py.run_bound(TES_STATE_SOURCE, Some(&namespace), None)?;
            Ok(namespace
                .get_item("TesState")?
                .ok_or_else(|| PyRuntimeError::new_err("TesState is not defined"))?
                .downcast_into::<PyType>()?
                .unbind())
        })
        .map(|cls| cls.bind(py))
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
    // Not `TES::list_tasks`: reqwest's query encoder cannot repeat tag lists.
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

    /// The task's current state, as a `TesState`.
    pub fn status(&self, py: Python<'_>) -> PyResult<PyObject> {
        let state = io(py, |rt| rt.block_on(task_status(&self.inner)))?;
        state_to_python(py, state)
    }

    /// Asks the server to cancel the task. Returns `None`.
    pub fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        io(py, |rt| rt.block_on(cancel_task(&self.inner)))
    }
}

/// A TES task: one to create, or one as returned by the server.
///
/// `json()` and `to_dict()` return the task's JSON as given or received, including fields
/// that the TES models don't know (object keys may come back in another order).
#[pyclass(name = "TesTask", module = "GA4GH", subclass)]
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
#[pyclass(name = "ListTasksParams", module = "GA4GH", subclass)]
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
#[pyclass(name = "ListTasksResponse", module = "GA4GH", subclass)]
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
