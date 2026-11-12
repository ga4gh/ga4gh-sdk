//! Reading JSON results classes keep, and reporting responses that don't fit.

use crate::utils::convert::to_python;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use serde_json::Value;

/// A field of a JSON object as a Python object
pub fn field(py: Python<'_>, raw: &Value, key: &str) -> PyResult<Option<PyObject>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => to_python(py, value).map(Some),
    }
}

/// The message for a response of `api` that doesn't fit its models.
pub fn invalid_response_text(api: &str, error: impl std::fmt::Display) -> String {
    format!("Invalid {api} response: {error}")
}

/// A `RuntimeError` for a response of `api` that doesn't fit its models.
pub fn invalid_response(api: &str, error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(invalid_response_text(api, error))
}
