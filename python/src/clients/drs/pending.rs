//! `PendingResolution`: the server is still preparing an answer.

#![allow(clippy::useless_conversion)]

use super::helpers::parse_body;
use ga4gh_sdk::clients::drs::resolution::{PendingResolution, Resolution};
use pyo3::prelude::*;
use pyo3::types::PyDict;
use serde_json::Value;

/// ask every `retry_after` seconds for objet or access URL from server
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

    /// The endpoint to ask for bulk call, it is `objects` or `objects/access`
    #[getter]
    fn endpoint(&self) -> String {
        self.inner.reference.endpoint.clone()
    }

    #[getter]
    fn retry_after(&self) -> u64 {
        self.inner.retry_after_secs
    }

    /// The pending state as a new `dict` with `api_root`, `endpoint` and `retry_after`.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new_bound(py);
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
pub(super) fn ready_or_pending<T: IntoPy<PyObject>>(
    py: Python<'_>,
    answer: Resolution<String>,
    convert: impl FnOnce(Value) -> PyResult<T>,
) -> PyResult<PyObject> {
    match answer {
        Resolution::Ready(body) => Ok(convert(parse_body(&body)?)?.into_py(py)),
        Resolution::Pending(inner) => Ok(PyPendingResolution { inner }.into_py(py)),
    }
}
