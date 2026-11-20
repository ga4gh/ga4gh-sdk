//! `TesTask`: a TES task to create, or one as returned by the server, with its JSON.
//!
//! The model and the JSON of a task are private, so that an extension crate can't change
//! one without the other (see `docs/state_ownership.md`, "Python views"). The same holds
//! for the other result classes that keep JSON. Assigning a field fails to compile, here
//! for one class of each family:
//!
//! ```compile_fail,E0616
//! use ga4gh::clients::tes::PyTesTask;
//! fn reset(task: &mut PyTesTask) {
//!     task.inner = task.inner().clone();
//! }
//! ```
//!
//! ```compile_fail,E0616
//! use ga4gh::clients::tes::PyTesTask;
//! fn reset(task: &mut PyTesTask) {
//!     task.raw = task.raw().clone();
//! }
//! ```
//!
//! ```compile_fail,E0616
//! use ga4gh::clients::wes::PyRunRequest;
//! fn reset(request: &mut PyRunRequest) {
//!     request.raw = request.raw().clone();
//! }
//! ```
//!
//! ```compile_fail,E0616
//! use ga4gh::clients::wes::PyRunListResponse;
//! fn reset(page: &mut PyRunListResponse) {
//!     page.runs = page.runs();
//! }
//! ```
//!
//! ```compile_fail,E0616
//! use ga4gh::clients::trs::PyTool;
//! fn reset(tool: &mut PyTool) {
//!     tool.inner = tool.inner().clone();
//! }
//! ```
//!
//! The accessors read them (stable rustdoc doesn't check the error code above, so this
//! twin shows that the imports and accessors are valid), and a changed task is built from
//! a changed copy of its JSON:
//!
//! ```no_run
//! use ga4gh::clients::{tes::PyTesTask, trs::PyTool, wes::{PyRunListResponse, PyRunRequest}};
//! fn read(task: &PyTesTask, request: &PyRunRequest, page: &PyRunListResponse, tool: &PyTool) {
//!     let _ = (task.inner(), request.raw(), page.runs(), tool.inner());
//! }
//! fn rename(task: &PyTesTask) -> pyo3::PyResult<PyTesTask> {
//!     let mut raw = task.raw().clone();
//!     raw["name"] = serde_json::json!("changed");
//!     PyTesTask::from_value(raw)
//! }
//! ```

use super::helpers::invalid_response;
use super::state::state_to_python;
use crate::utils::convert::{check_extra_args, read_file, to_json, to_python};
use crate::utils::json::{field, view_accessors};
use ga4gh_sdk::clients::tes::models::TesTask;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyMapping, PyString, PyTuple, PyType};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A TES task: one to create, or one as returned by the server.
///
/// Keeps the task's full JSON, including fields that the TES models don't know. Missing
/// fields are `None`. Nested values, such as executors, come as `dict`s and `list`s.
/// Tasks can be pickled and copied.
#[pyclass(name = "TesTask", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTesTask {
    /// The task parsed into the TES model; derived from `raw`.
    inner: TesTask,
    /// The task's JSON as given or received.
    raw: Value,
}

view_accessors!(PyTesTask, TesTask);

impl PyTesTask {
    pub(super) fn parse(raw: Value) -> Result<Self, String> {
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
        if let Ok(task) = task.cast::<PyTesTask>() {
            return Ok(task.borrow().clone());
        }
        if let Ok(mapping) = task.cast::<PyMapping>() {
            if !task.is_instance_of::<PyDict>() {
                // `json.dumps` takes only `dict`s, so copy other mappings into one.
                let dict = PyDict::new(task.py());
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

    /// Builds a task from a `dict`, as `cls(task)`.
    ///
    /// Args:
    ///     task: The task's JSON as a mapping.
    ///
    /// Returns:
    ///     The task.
    ///
    /// Raises:
    ///     ValueError: The task doesn't fit the TES models.
    #[classmethod]
    fn from_dict<'py>(
        cls: &Bound<'py, PyType>,
        task: &Bound<'py, PyMapping>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(cls.py());
        dict.update(task)?;
        cls.call1((dict,))
    }

    /// Builds a task from JSON text.
    ///
    /// Args:
    ///     text: The task as a JSON object.
    ///
    /// Returns:
    ///     The task.
    ///
    /// Raises:
    ///     ValueError: `text` isn't JSON, or the task doesn't fit the TES models.
    #[classmethod]
    #[pyo3(name = "from_json")]
    fn py_from_json<'py>(
        cls: &Bound<'py, PyType>,
        text: &Bound<'py, PyString>,
    ) -> PyResult<Bound<'py, PyAny>> {
        cls.call1((text,))
    }

    /// Reads a task from a JSON file.
    ///
    /// Args:
    ///     path: The file's path.
    ///
    /// Returns:
    ///     The task.
    ///
    /// Raises:
    ///     OSError: The file can't be read, for example `FileNotFoundError`.
    ///     ValueError: The file isn't JSON, or the task doesn't fit the TES models.
    #[classmethod]
    fn from_file<'py>(cls: &Bound<'py, PyType>, path: PathBuf) -> PyResult<Bound<'py, PyAny>> {
        let task = Self::from_path(cls.py(), &path)?;
        cls.call1((to_python(cls.py(), task.raw())?,))
    }

    /// Lets `pickle` and `copy` rebuild the task as `type(task)(task.to_dict())`, with the
    /// attributes that a subclass instance holds in `__dict__`.
    #[allow(clippy::type_complexity)]
    fn __reduce__<'py>(
        slf: &Bound<'py, Self>,
    ) -> PyResult<(Bound<'py, PyType>, (Py<PyAny>,), Option<Bound<'py, PyAny>>)> {
        let raw = to_python(slf.py(), slf.borrow().raw())?;
        let state = slf.getattr("__dict__").ok();
        Ok((slf.get_type(), (raw,), state))
    }

    /// The task's ID; `None` for a task that isn't created yet.
    #[getter]
    pub fn id(&self) -> Option<String> {
        self.inner().id.clone()
    }

    /// The task's state; `UNKNOWN` if it has none or one that TES 1.1 doesn't define.
    #[getter]
    pub fn state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        state_to_python(py, self.inner().state.unwrap_or_default())
    }

    /// Same as `state`.
    #[getter]
    pub fn status(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.state(py)
    }

    /// The task's name.
    #[getter]
    fn name(&self) -> Option<String> {
        self.inner().name.clone()
    }

    /// The task's description.
    #[getter]
    fn description(&self) -> Option<String> {
        self.inner().description.clone()
    }

    /// When the task was created, as an RFC 3339 timestamp.
    #[getter]
    fn creation_time(&self) -> Option<String> {
        self.inner().creation_time.clone()
    }

    /// The files to put into the task's containers.
    #[getter]
    fn inputs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "inputs")
    }

    /// The files to keep from the task's containers.
    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "outputs")
    }

    /// The resources that the task asks for, such as CPU cores and RAM.
    #[getter]
    fn resources(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "resources")
    }

    /// The commands to run, in order, each with its container image.
    #[getter]
    fn executors(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "executors")
    }

    /// Paths that the executors share.
    #[getter]
    fn volumes(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "volumes")
    }

    /// Key-value tags of the task.
    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "tags")
    }

    /// The task's logs, one per attempt (in the `BASIC` and `FULL` views).
    #[getter]
    fn logs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "logs")
    }

    /// Gives the task as JSON text, as given or received; keys may come in another order.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// Gives the task as a new `dict`, as given or received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let state = slf.borrow().inner().state.unwrap_or_default();
        Ok(format!("{name}(id={id}, state='{state}')"))
    }
}

// The tests call `parse`, which `from_value` and `from_json` share: a Python exception
// needs libpython, which the test binary doesn't link (see `utils::json::tests`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::json::tests::assert_describes;
    use serde_json::json;

    #[test]
    fn a_task_describes_its_json() {
        let raw = json!({
            "name": "hello",
            "description": null,
            "executors": [{"image": "alpine", "command": ["true"]}],
            "x_extension": {"kept": true}
        });
        let task = PyTesTask::parse(raw.clone()).unwrap();
        assert_describes(task.inner(), task.raw(), &task.json(), &raw);
        assert_eq!(task.inner().name.as_deref(), Some("hello"));
        assert_eq!(task.inner().description, None);
        assert_eq!(task.inner().extra["x_extension"], json!({"kept": true}));
    }

    #[test]
    fn a_changed_task_is_built_from_changed_json() {
        let task = PyTesTask::parse(json!({"name": "original"})).unwrap();
        let mut raw = task.raw().clone();
        raw["name"] = json!("changed");
        let changed = PyTesTask::parse(raw.clone()).unwrap();
        assert_describes(changed.inner(), changed.raw(), &changed.json(), &raw);
        assert_eq!(changed.inner().name.as_deref(), Some("changed"));
        assert_eq!(task.inner().name.as_deref(), Some("original"));
    }

    #[test]
    fn json_that_is_no_task_is_rejected() {
        assert_eq!(
            PyTesTask::parse(json!([1])).err().unwrap(),
            "not a JSON object"
        );
        let error = PyTesTask::parse(json!({"name": 1})).err().unwrap();
        assert!(error.starts_with("invalid type: integer `1`"), "{error}");
    }
}
