//! `PendingResolution`: the server is still preparing an answer.

use super::helpers::parse_body;
use ga4gh_sdk::clients::drs::resolution::{PendingResolution, Resolution};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::IntoPyObjectExt;
use serde_json::Value;

/// The server is still preparing an object or access URL: ask again after `retry_after`
/// seconds.
///
/// `api_root` and `endpoint` say what to ask for; they hold no credentials.
#[pyclass(name = "PendingResolution", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyPendingResolution {
    pub inner: PendingResolution,
}

#[pymethods]
impl PyPendingResolution {
    /// The API root of the service, without credentials, query or fragment.
    #[getter]
    fn api_root(&self) -> String {
        self.inner.reference.api_root.clone()
    }

    /// The endpoint to ask again, below `api_root`. For a bulk call, it is `objects` or
    /// `objects/access`; call `get_bulk_objects()` or `get_bulk_access_urls()` again.
    #[getter]
    fn endpoint(&self) -> String {
        self.inner.reference.endpoint.clone()
    }

    /// The seconds to wait before asking again (at least 1).
    #[getter]
    fn retry_after(&self) -> u64 {
        self.inner.retry_after_secs
    }

    /// The pending state as a new `dict` with `api_root`, `endpoint` and `retry_after`.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        dict.set_item("api_root", self.api_root())?;
        dict.set_item("endpoint", self.endpoint())?;
        dict.set_item("retry_after", self.retry_after())?;
        Ok(dict)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let endpoint = slf.getattr("endpoint")?.repr()?;
        let retry_after = slf.borrow().inner.retry_after_secs;
        Ok(format!(
            "{name}(endpoint={endpoint}, retry_after={retry_after})"
        ))
    }
}

/// Converts the answer of one request to a result or the pending state.
pub(super) fn ready_or_pending<'py, T>(
    py: Python<'py>,
    answer: Resolution<String>,
    convert: impl FnOnce(Value) -> PyResult<T>,
) -> PyResult<Py<PyAny>>
where
    T: IntoPyObject<'py>,
{
    match answer {
        Resolution::Ready(body) => convert(parse_body(&body)?)?.into_py_any(py),
        Resolution::Pending(inner) => PyPendingResolution { inner }.into_py_any(py),
    }
}
