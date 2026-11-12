//! Checks of arguments before a request is sent.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Rejects IDs that can't be a path segment: empty, `.` or `..`.
pub fn require_id(id: &str, what: &str) -> PyResult<()> {
    if id.is_empty() {
        return Err(PyValueError::new_err(format!("{what} must not be empty")));
    }
    if matches!(id, "." | "..") {
        return Err(PyValueError::new_err(format!("{what} must not be {id:?}")));
    }
    Ok(())
}
