//! `TesTask`: a TES task to create, or one as returned by the server, with its JSON.

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
/// `json()` and `to_dict()` return the task's JSON as given or received, including fields
/// that the TES models don't know (object keys may come back in another order).
#[pyclass(name = "TesTask", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTesTask {
    /// The task parsed into the TES model; derived from `raw`.
    #[deprecated(note = "read `inner()`; build a changed task from JSON with `from_value`")]
    pub inner: TesTask,
    /// The task's JSON as given or received.
    #[deprecated(note = "read `raw()`; build a changed task from JSON with `from_value`")]
    pub raw: Value,
}

view_accessors!(PyTesTask, TesTask);

impl PyTesTask {
    #[allow(deprecated)] // Builds the fields that the accessors read.
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

    /// Builds a task from a mapping, such as the result of `to_dict()`.
    ///
    /// Like `from_json` and `from_file`, it calls `cls(task)`, so a subclass's further
    /// constructor arguments need defaults.
    #[classmethod]
    fn from_dict<'py>(
        cls: &Bound<'py, PyType>,
        task: &Bound<'py, PyMapping>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let dict = PyDict::new(cls.py());
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

    #[getter]
    pub fn id(&self) -> Option<String> {
        self.inner().id.clone()
    }

    /// The task's state as a `TesState`; `UNKNOWN` if the task has none or one that
    /// TES 1.1 doesn't define.
    #[getter]
    pub fn state(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        state_to_python(py, self.inner().state.unwrap_or_default())
    }

    /// Same as `state`.
    #[getter]
    pub fn status(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.state(py)
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner().name.clone()
    }

    #[getter]
    fn description(&self) -> Option<String> {
        self.inner().description.clone()
    }

    #[getter]
    fn creation_time(&self) -> Option<String> {
        self.inner().creation_time.clone()
    }

    #[getter]
    fn inputs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "inputs")
    }

    #[getter]
    fn outputs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "outputs")
    }

    #[getter]
    fn resources(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "resources")
    }

    #[getter]
    fn executors(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "executors")
    }

    #[getter]
    fn volumes(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "volumes")
    }

    #[getter]
    fn tags(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "tags")
    }

    #[getter]
    fn logs(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "logs")
    }

    /// The task as JSON text, as given or received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// The task as a new `dict`, as given or received.
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
}
