//! Checks of arguments before a request is sent.

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Rejects IDs that can't be a path segment: empty, `.` or `..`. Callers must still
/// percent-encode the ID (for example with `urlencode`) before they put it in a path.
pub fn require_id(id: &str, what: &str) -> PyResult<()> {
    if id.is_empty() {
        return Err(PyValueError::new_err(format!("{what} must not be empty")));
    }
    if matches!(id, "." | "..") {
        return Err(PyValueError::new_err(format!("{what} must not be {id:?}")));
    }
    Ok(())
}
