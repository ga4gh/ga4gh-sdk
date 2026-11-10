//! Python bindings of the WES client.
//!
//! The classes mirror those of TES: `WES` is the client, `Run` addresses a run on the server,
//! `RunRequest` is a run to submit, and `RunLog`, `RunSummary`, `RunListResponse`, `TaskLog`
//! and `TaskListResponse` are results that keep the JSON they were built from.

use crate::utils::convert::{
    check_extra_args, define_class, os_error, read_file, to_json, to_python, with_query,
};
use crate::utils::runtime::io;
use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::wes::attachments::{
    validate_filename, AttachmentSource, StreamingAttachment,
};
use ga4gh_sdk::clients::wes::models::{
    RunListResponse, RunListResponseRunsInner, RunLog, RunRequest, State, TaskListResponse, TaskLog,
};
use ga4gh_sdk::clients::wes::{urlencode, Run, WES};
use ga4gh_sdk::utils::operation::Operation;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::GILOnceCell;
use pyo3::types::{PyDict, PyMapping, PyString, PyTuple, PyType};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The time that `WES.run()` allows for uploading a run and reading the answer, as for
/// `WES::run_workflow_with_attachments`.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// Registers the WES classes in a module.
pub fn register(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWES>()?;
    m.add_class::<PyRun>()?;
    m.add_class::<PyRunRequest>()?;
    m.add_class::<PyRunLog>()?;
    m.add_class::<PyRunSummary>()?;
    m.add_class::<PyRunListResponse>()?;
    m.add_class::<PyTaskLog>()?;
    m.add_class::<PyTaskListResponse>()?;
    m.add("WesState", wes_state_type(py)?)?;
    Ok(())
}

fn invalid_response(error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("Invalid WES response: {error}"))
}

/// Parses a server's JSON object into a WES model. Errors are `RuntimeError`s.
fn parse_response<T: DeserializeOwned>(raw: &Value) -> PyResult<T> {
    if !raw.is_object() {
        return Err(invalid_response("not a JSON object"));
    }
    serde_json::from_value(raw.clone()).map_err(invalid_response)
}

/// A field of a JSON object as a Python object, or `None` if it is missing or `null`.
fn field(py: Python<'_>, raw: &Value, key: &str) -> PyResult<Option<PyObject>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => to_python(py, value).map(Some),
    }
}

/// The items of a JSON array field, or none if the field is missing.
fn items(raw: &Value, key: &str) -> Vec<Value> {
    match raw.get(key) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Sends a GET request and parses the response as JSON.
fn get_json(py: Python<'_>, transport: &Transport, endpoint: &str, what: &str) -> PyResult<Value> {
    let body = io(py, |rt| {
        rt.block_on(transport.get(endpoint, None))
            .map_err(|error| format!("Failed to {what}: {error}"))
    })?;
    serde_json::from_str(&body).map_err(invalid_response)
}

/// Appends the paging parameters of the WES list endpoints.
fn paged(endpoint: &str, page_size: Option<i64>, page_token: Option<String>) -> PyResult<String> {
    with_query(
        endpoint,
        Some(serde_json::json!({ "page_size": page_size, "page_token": page_token })),
    )
}

/// Rejects IDs that can't be a path segment: empty, `.` or `..`.
fn require_id(id: &str, what: &str) -> PyResult<()> {
    if id.is_empty() {
        return Err(PyValueError::new_err(format!("{what} must not be empty")));
    }
    if matches!(id, "." | "..") {
        return Err(PyValueError::new_err(format!("{what} must not be {id:?}")));
    }
    Ok(())
}

/// Python source of the `WesState` enum. Its members must match the variants of `State`.
const WES_STATE_SOURCE: &str = r#"
import enum


class WesState(str, enum.Enum):
    """The state of a WES run.

    Members are strings, so `WesState.RUNNING == "RUNNING"`, and `str()`, f-strings and
    `json.dumps()` give the state's name. `WesState("RUNNING")` looks a state up by name.
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
    CANCELING = "CANCELING"
    PREEMPTED = "PREEMPTED"

    def __str__(self):
        return self.value

    def __format__(self, format_spec):
        return format(self.value, format_spec)
"#;

static WES_STATE: GILOnceCell<Py<PyType>> = GILOnceCell::new();

/// The `WesState` Python enum (a `str` enum, created once per interpreter).
pub fn wes_state_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    WES_STATE
        .get_or_try_init(py, || define_class(py, WES_STATE_SOURCE, "WesState"))
        .map(|cls| cls.bind(py))
}

/// Converts a WES state to the matching `WesState` member.
pub fn state_to_python(py: Python<'_>, state: State) -> PyResult<PyObject> {
    Ok(wes_state_type(py)?
        .getattr(state.to_string().as_str())?
        .unbind())
}

/// A WES run request: a workflow to run, with its parameters.
///
/// `json()` and `to_dict()` return the request's JSON as given or received, including fields
/// that the WES models don't know. `WES.run()` sends the fields of the WES `RunRequest` form.
#[pyclass(name = "RunRequest", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyRunRequest {
    /// The request parsed into the WES model; derived from `raw`.
    pub inner: RunRequest,
    /// The request's JSON as given or received. Build both fields with `from_value` to keep
    /// them in step.
    pub raw: Value,
}

impl PyRunRequest {
    fn parse(raw: Value) -> Result<Self, String> {
        if !raw.is_object() {
            return Err("not a JSON object".to_owned());
        }
        let inner = serde_json::from_value(raw.clone()).map_err(|e| e.to_string())?;
        Ok(PyRunRequest { inner, raw })
    }

    /// Builds a request from a caller's JSON, keeping the JSON. Errors are `ValueError`s.
    pub fn from_value(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(|e| PyValueError::new_err(format!("Invalid run request: {e}")))
    }

    /// Builds a request from a `RunRequest`, a JSON-serialisable object or JSON text.
    pub fn from_python(request: &Bound<'_, PyAny>) -> PyResult<Self> {
        if let Ok(request) = request.downcast::<PyRunRequest>() {
            return Ok(request.borrow().clone());
        }
        if let Ok(mapping) = request.downcast::<PyMapping>() {
            if !request.is_instance_of::<PyDict>() {
                // `json.dumps` takes only `dict`s, so copy other mappings into one.
                let dict = PyDict::new_bound(request.py());
                dict.update(mapping)?;
                return Self::from_value(to_json(&dict, "run request")?);
            }
        }
        Self::from_value(to_json(request, "run request")?)
    }

    /// Reads a request from a JSON file.
    pub fn from_path(py: Python<'_>, path: &Path) -> PyResult<Self> {
        let text = read_file(py, path)?;
        let raw = serde_json::from_str(&text).map_err(|e| {
            PyValueError::new_err(format!(
                "Invalid run request JSON in {}: {e}",
                path.display()
            ))
        })?;
        Self::from_value(raw)
    }

    /// Checks the fields that `WES::run_workflow` requires.
    fn check_complete(&self) -> PyResult<()> {
        let request = &self.inner;
        for (name, value) in [
            ("workflow_type", &request.workflow_type),
            ("workflow_type_version", &request.workflow_type_version),
            ("workflow_url", &request.workflow_url),
        ] {
            if value.is_empty() {
                return Err(PyValueError::new_err(format!(
                    "Invalid run request: {name} is required"
                )));
            }
        }
        if request.workflow_params.as_ref().is_none_or(Value::is_null) {
            return Err(PyValueError::new_err(
                "Invalid run request: workflow_params is required",
            ));
        }
        Ok(())
    }
}

#[pymethods]
impl PyRunRequest {
    /// `request` is a `dict` (or other JSON-serialisable object), JSON text or a `RunRequest`.
    #[new]
    #[classmethod]
    #[pyo3(signature = (request, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        request: &Bound<'_, PyAny>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::from_python(request)
    }

    /// Builds a request from a mapping, such as the result of `to_dict()`.
    ///
    /// Like `from_json` and `from_file`, it calls `cls(request)`, so a subclass's further
    /// constructor arguments need defaults.
    #[classmethod]
    fn from_dict<'py>(
        cls: &Bound<'py, PyType>,
        request: &Bound<'py, PyMapping>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new_bound(cls.py());
        dict.update(request)?;
        cls.call1((dict,))
    }

    /// Builds a request from JSON text, such as the result of `json()`.
    #[classmethod]
    fn from_json<'py>(
        cls: &Bound<'py, PyType>,
        text: &Bound<'py, PyString>,
    ) -> PyResult<Bound<'py, PyAny>> {
        cls.call1((text,))
    }

    /// Reads a request from a JSON file. `path` is a `str` or `os.PathLike`.
    #[classmethod]
    fn from_file<'py>(cls: &Bound<'py, PyType>, path: PathBuf) -> PyResult<Bound<'py, PyAny>> {
        let request = Self::from_path(cls.py(), &path)?;
        cls.call1((to_python(cls.py(), &request.raw)?,))
    }

    /// Lets `pickle` and `copy` rebuild the request as `type(request)(request.to_dict())`,
    /// with the attributes that a subclass instance holds in `__dict__`.
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyType>, (PyObject,), Option<Bound<'py, PyAny>>)> {
        let raw = to_python(slf.py(), &slf.borrow().raw)?;
        let state = slf.getattr("__dict__").ok();
        Ok((slf.get_type(), (raw,), state))
    }

    #[getter]
    fn workflow_type(&self) -> String {
        self.inner.workflow_type.clone()
    }

    #[getter]
    fn workflow_type_version(&self) -> String {
        self.inner.workflow_type_version.clone()
    }

    #[getter]
    fn workflow_url(&self) -> String {
        self.inner.workflow_url.clone()
    }

    #[getter]
    fn workflow_params(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "workflow_params")
    }

    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "tags")
    }

    #[getter]
    fn workflow_engine(&self) -> Option<String> {
        self.inner.workflow_engine.clone()
    }

    #[getter]
    fn workflow_engine_version(&self) -> Option<String> {
        self.inner.workflow_engine_version.clone()
    }

    #[getter]
    fn workflow_engine_parameters(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "workflow_engine_parameters")
    }

    /// The request as JSON text, as given or received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The request as a new `dict`, as given or received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let workflow_type = slf.getattr("workflow_type")?.repr()?;
        let workflow_url = slf.getattr("workflow_url")?.repr()?;
        Ok(format!(
            "{name}(workflow_type={workflow_type}, workflow_url={workflow_url})"
        ))
    }
}

/// A run's details, with its request, state, logs and outputs, as returned by the server.
#[pyclass(name = "RunLog", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyRunLog {
    pub inner: RunLog,
    /// The run's JSON as received.
    pub raw: Value,
}

impl PyRunLog {
    /// Builds a run log from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyRunLog {
            inner: parse_response(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyRunLog {
    #[getter]
    fn run_id(&self) -> Option<String> {
        self.inner.run_id.clone()
    }

    /// The run's request as a `RunRequest`, or `None` if it is missing or doesn't fit.
    #[getter]
    fn request(&self) -> Option<PyRunRequest> {
        self.inner.request.as_ref()?;
        PyRunRequest::parse(self.raw.get("request")?.clone()).ok()
    }

    /// The run's state as a `WesState`; `UNKNOWN` if the run has none.
    #[getter]
    fn state(&self, py: Python<'_>) -> PyResult<PyObject> {
        state_to_python(py, self.inner.state.unwrap_or_default())
    }

    #[getter]
    fn run_log(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "run_log")
    }

    #[getter]
    fn task_logs_url(&self) -> Option<String> {
        self.inner.task_logs_url.clone()
    }

    #[getter]
    fn task_logs(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "task_logs")
    }

    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "outputs")
    }

    /// The run as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The run as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let run_id = slf.getattr("run_id")?.repr()?;
        let state = slf.borrow().inner.state.unwrap_or_default();
        Ok(format!("{name}(run_id={run_id}, state='{state}')"))
    }
}

/// A run in the list of `WES.list()`.
#[pyclass(name = "RunSummary", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyRunSummary {
    pub inner: RunListResponseRunsInner,
    /// The run's JSON as received.
    pub raw: Value,
}

impl PyRunSummary {
    /// Builds a run summary from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyRunSummary {
            inner: parse_response(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyRunSummary {
    #[getter]
    fn run_id(&self) -> String {
        self.inner.run_id.clone()
    }

    /// The run's state as a `WesState`; `UNKNOWN` if the run has none.
    #[getter]
    fn state(&self, py: Python<'_>) -> PyResult<PyObject> {
        state_to_python(py, self.inner.state.unwrap_or_default())
    }

    #[getter]
    fn start_time(&self) -> Option<String> {
        self.inner.start_time.clone()
    }

    #[getter]
    fn end_time(&self) -> Option<String> {
        self.inner.end_time.clone()
    }

    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "tags")
    }

    /// The run as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The run as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let run_id = slf.getattr("run_id")?.repr()?;
        let state = slf.borrow().inner.state.unwrap_or_default();
        Ok(format!("{name}(run_id={run_id}, state='{state}')"))
    }
}

/// One page of runs from `WES.list()`.
#[pyclass(name = "RunListResponse", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyRunListResponse {
    pub inner: RunListResponse,
    pub runs: Vec<PyRunSummary>,
    /// The response's JSON as received; a bare array of runs becomes `{"runs": [...]}`.
    pub raw: Value,
}

impl PyRunListResponse {
    /// Builds a response from a server's JSON, keeping the JSON.
    ///
    /// Accepts a bare array of runs, as WES 0.2.0 servers (GA4GH Starter Kit) send it.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        let raw = match raw {
            Value::Array(runs) => serde_json::json!({ "runs": runs }),
            raw => raw,
        };
        let runs = items(&raw, "runs")
            .into_iter()
            .map(PyRunSummary::from_json)
            .collect::<PyResult<_>>()?;
        Ok(PyRunListResponse {
            inner: parse_response(&raw)?,
            runs,
            raw,
        })
    }
}

#[pymethods]
impl PyRunListResponse {
    #[getter]
    pub fn runs(&self) -> Vec<PyRunSummary> {
        self.runs.clone()
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

/// A task of a run, from `Run.tasks()` or `Run.task()` (WES 1.1).
#[pyclass(name = "TaskLog", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyTaskLog {
    pub inner: TaskLog,
    /// The task's JSON as received.
    pub raw: Value,
}

impl PyTaskLog {
    /// Builds a task log from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyTaskLog {
            inner: parse_response(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyTaskLog {
    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn name(&self) -> String {
        self.inner.name.clone()
    }

    #[getter]
    fn cmd(&self) -> Option<Vec<String>> {
        self.inner.cmd.clone()
    }

    #[getter]
    fn start_time(&self) -> Option<String> {
        self.inner.start_time.clone()
    }

    #[getter]
    fn end_time(&self) -> Option<String> {
        self.inner.end_time.clone()
    }

    #[getter]
    fn stdout(&self) -> Option<String> {
        self.inner.stdout.clone()
    }

    #[getter]
    fn stderr(&self) -> Option<String> {
        self.inner.stderr.clone()
    }

    #[getter]
    fn exit_code(&self) -> Option<i32> {
        self.inner.exit_code
    }

    #[getter]
    fn system_logs(&self) -> Option<Vec<String>> {
        self.inner.system_logs.clone()
    }

    #[getter]
    fn tes_uri(&self) -> Option<String> {
        self.inner.tes_uri.clone()
    }

    /// The task as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The task as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let task_name = slf.getattr("name")?.repr()?;
        Ok(format!("{name}(id={id}, name={task_name})"))
    }
}

/// One page of tasks from `Run.tasks()`.
#[pyclass(name = "TaskListResponse", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyTaskListResponse {
    pub inner: TaskListResponse,
    pub task_logs: Vec<PyTaskLog>,
    /// The response's JSON as received.
    pub raw: Value,
}

impl PyTaskListResponse {
    /// Builds a response from a server's JSON, keeping the JSON.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        let task_logs = items(&raw, "task_logs")
            .into_iter()
            .map(PyTaskLog::from_json)
            .collect::<PyResult<_>>()?;
        Ok(PyTaskListResponse {
            inner: parse_response(&raw)?,
            task_logs,
            raw,
        })
    }
}

#[pymethods]
impl PyTaskListResponse {
    #[getter]
    pub fn task_logs(&self) -> Vec<PyTaskLog> {
        self.task_logs.clone()
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

/// Gets a run's details.
fn get_run_log(py: Python<'_>, transport: &Transport, id: &str) -> PyResult<PyRunLog> {
    require_id(id, "run ID")?;
    let endpoint = format!("runs/{}", urlencode(id));
    PyRunLog::from_json(get_json(py, transport, &endpoint, "get run")?)
}

/// A WES run on the server, addressed by its ID.
#[pyclass(name = "Run", module = "GA4GH", subclass)]
pub struct PyRun {
    pub inner: Run,
}

impl PyRun {
    /// Addresses the run `id` with `transport`; an empty ID raises `ValueError`.
    pub fn new(id: String, transport: &PyTransport) -> PyResult<Self> {
        require_id(&id, "run ID")?;
        Ok(PyRun {
            inner: Run::new(id, transport.inner.clone()),
        })
    }
}

#[pymethods]
impl PyRun {
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
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    /// A transport with the run's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport.clone(),
        }
    }

    /// The run's current state, as a `WesState`.
    pub fn status(&self, py: Python<'_>) -> PyResult<PyObject> {
        let state = io(py, |rt| {
            rt.block_on(self.inner.status())
                .map_err(|e| format!("Failed to get run status: {e}"))
        })?;
        state_to_python(py, state)
    }

    /// Asks the server to cancel the run. Returns `None`.
    pub fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        io(py, |rt| {
            rt.block_on(self.inner.cancel())
                .map(|_| ())
                .map_err(|e| format!("Failed to cancel run: {e}"))
        })
    }

    /// The run's details, with its request, state, logs and outputs, as a `RunLog`.
    pub fn log(&self, py: Python<'_>) -> PyResult<PyRunLog> {
        get_run_log(py, &self.inner.transport, &self.inner.id)
    }

    /// Lists the run's tasks, one page at a time (WES 1.1).
    #[pyo3(signature = (page_size=None, page_token=None))]
    pub fn tasks(
        &self,
        py: Python<'_>,
        page_size: Option<i64>,
        page_token: Option<String>,
    ) -> PyResult<PyTaskListResponse> {
        let endpoint = paged(
            &format!("runs/{}/tasks", urlencode(&self.inner.id)),
            page_size,
            page_token,
        )?;
        PyTaskListResponse::from_json(get_json(
            py,
            &self.inner.transport,
            &endpoint,
            "list run tasks",
        )?)
    }

    /// Gets one of the run's tasks (WES 1.1).
    pub fn task(&self, py: Python<'_>, task_id: &str) -> PyResult<PyTaskLog> {
        require_id(task_id, "task ID")?;
        let endpoint = format!(
            "runs/{}/tasks/{}",
            urlencode(&self.inner.id),
            urlencode(task_id)
        );
        PyTaskLog::from_json(get_json(
            py,
            &self.inner.transport,
            &endpoint,
            "get run task",
        )?)
    }
}

/// The source of an attachment's content: `bytes` (or `bytearray`), or a file path.
fn attachment_source(
    py: Python<'_>,
    filename: &str,
    content: &Bound<'_, PyAny>,
) -> PyResult<AttachmentSource> {
    if let Ok(bytes) = content.extract::<Cow<'_, [u8]>>() {
        return Ok(AttachmentSource::Bytes(bytes.into_owned()));
    }
    if !content.is_instance_of::<PyString>() && content.hasattr("__fspath__")? {
        // Opened here, so that errors are `OSError`s with the path, and the file that was
        // checked is the one that is sent.
        let path: PathBuf = content.extract()?;
        let file = std::fs::File::open(&path).map_err(|error| os_error(py, error, &path))?;
        let metadata = file
            .metadata()
            .map_err(|error| os_error(py, error, &path))?;
        if !metadata.is_file() {
            return Err(PyValueError::new_err(format!(
                "Invalid attachment {filename}: {} is not a file",
                path.display()
            )));
        }
        // `tokio::fs` is enabled by `ga4gh-lib`, which depends on Tokio with all features.
        return Ok(AttachmentSource::Reader {
            reader: Box::pin(tokio::fs::File::from_std(file)),
            length: Some(metadata.len()),
        });
    }
    Err(PyValueError::new_err(format!(
        "Invalid attachment {filename}: content must be bytes or an os.PathLike, not {}",
        content.get_type().qualname()?
    )))
}

const ATTACHMENTS: &str = "attachments must be a mapping or a list of (filename, content) pairs";

/// Converts the `attachments` argument of `WES.run()`, checking the filenames.
fn attachment_list(
    py: Python<'_>,
    attachments: &Bound<'_, PyAny>,
) -> PyResult<Vec<StreamingAttachment>> {
    let pairs = match attachments.downcast::<PyMapping>() {
        Ok(mapping) => mapping.items()?.into_any(),
        Err(_) => attachments.clone(),
    };
    let mut names = HashSet::new();
    let mut result = Vec::new();
    for pair in pairs.iter()? {
        let (filename, content): (String, Bound<'_, PyAny>) = pair?
            .extract()
            .map_err(|_| PyValueError::new_err(ATTACHMENTS))?;
        validate_filename(&filename).map_err(|error| {
            PyValueError::new_err(format!("Invalid attachment {filename:?}: {error}"))
        })?;
        if !names.insert(filename.clone()) {
            return Err(PyValueError::new_err(format!(
                "Invalid attachment {filename:?}: duplicate workflow attachment filename"
            )));
        }
        let source = attachment_source(py, &filename, &content)?;
        result.push(StreamingAttachment { filename, source });
    }
    Ok(result)
}

/// A WES client. Subclass it to add endpoints, using `self.transport`.
///
/// The public `inner` preserves the Rust extension API and is safe to share
/// between Python threads while synchronous requests release the GIL.
#[pyclass(name = "WES", module = "GA4GH", subclass)]
pub struct PyWES {
    pub inner: WES,
}

impl PyWES {
    /// Build a WES client for Rust extension crates while releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::with_gil(|py| Self::new_with_gil(py, config))
    }

    fn new_with_gil(py: Python<'_>, config: &PyConfiguration) -> PyResult<Self> {
        io(py, |rt| {
            rt.block_on(WES::new(&config.inner))
                .map(|inner| Self { inner })
                .map_err(|error| format!("Failed to create WES instance: {error}"))
        })
    }
}

#[pymethods]
impl PyWES {
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

    /// Starts a workflow run and returns a `Run` for it.
    ///
    /// `request` is a `RunRequest`, a `dict` (or other mapping), or the path of a JSON file as
    /// `str` or `os.PathLike`. A `str` is always a path; pass JSON text as
    /// `RunRequest.from_json(text)`. `attachments` maps filenames (relative paths) to
    /// contents as `bytes` or file paths (`os.PathLike`); a list of `(filename, content)`
    /// pairs works too. Files are streamed, not read into memory.
    #[pyo3(signature = (request, attachments=None))]
    pub fn run(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyAny>,
        attachments: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyRun> {
        if let Ok(text) = request.downcast::<PyString>() {
            // Lossy, so that paths with surrogate escapes (non-UTF-8 file names) pass.
            if text.to_string_lossy().trim_start().starts_with('{') {
                return Err(PyValueError::new_err(
                    "WES.run() takes a str as a file path; pass JSON text as \
                     RunRequest.from_json(text), or a path that starts with '{' as a \
                     pathlib.Path",
                ));
            }
        }
        let request = if request.is_instance_of::<PyString>() || request.hasattr("__fspath__")? {
            PyRunRequest::from_path(py, &request.extract::<PathBuf>()?)?
        } else {
            PyRunRequest::from_python(request)?
        };
        request.check_complete()?;
        let attachments = match attachments {
            Some(attachments) => attachment_list(py, attachments)?,
            None => Vec::new(),
        };
        let request = request.inner;
        io(py, |rt| {
            rt.block_on(self.inner.run_workflow_streaming(
                request,
                attachments,
                &Operation::new(RUN_TIMEOUT),
            ))
            .map(|inner| PyRun { inner })
            .map_err(|error| format!("Failed to run workflow: {error}"))
        })
    }

    /// Lists runs, one page at a time.
    #[pyo3(signature = (page_size=None, page_token=None))]
    pub fn list(
        &self,
        py: Python<'_>,
        page_size: Option<i64>,
        page_token: Option<String>,
    ) -> PyResult<PyRunListResponse> {
        let endpoint = paged("runs", page_size, page_token)?;
        PyRunListResponse::from_json(get_json(py, &self.inner.transport, &endpoint, "list runs")?)
    }

    /// Gets a run's details, with its request, state, logs and outputs, as a `RunLog`.
    pub fn get(&self, py: Python<'_>, id: &str) -> PyResult<PyRunLog> {
        get_run_log(py, &self.inner.transport, id)
    }
}
