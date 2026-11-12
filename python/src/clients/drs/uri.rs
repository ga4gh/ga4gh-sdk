//! A hostname-based DRS URI, such as `drs://drs.example.org/object-id`.

// The `#[pymethods]` expansion of PyO3 0.22 converts each `PyResult` into itself, which
// clippy reports at every method that returns one, as in `tes.rs`.
#![allow(clippy::useless_conversion)]

use crate::utils::convert::check_extra_args;
use ga4gh_sdk::clients::drs::uri::DrsUri;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

/// A hostname-based DRS URI, such as `drs://drs.example.org/object-id`.
#[pyclass(name = "DrsUri", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyDrsUri {
    pub inner: DrsUri,
}

/// Parses a DRS URI; unsupported or malformed ones raise `ValueError`.
pub(super) fn parse_uri(uri: &str) -> PyResult<DrsUri> {
    DrsUri::parse(uri).map_err(|error| PyValueError::new_err(format!("Invalid DRS URI: {error}")))
}

#[pymethods]
impl PyDrsUri {
    /// Compact identifiers (`drs://prefix:accession`) are not supported.
    #[new]
    #[classmethod]
    #[pyo3(signature = (uri, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        uri: &str,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Ok(PyDrsUri {
            inner: parse_uri(uri)?,
        })
    }

    /// The host name, in lowercase.
    #[getter]
    fn authority(&self) -> String {
        self.inner.authority.clone()
    }

    /// The object ID, decoded.
    #[getter]
    fn object_id(&self) -> String {
        self.inner.object_id.clone()
    }

    /// The API root that the URI implies, `https://<authority>/ga4gh/drs/v1`.
    fn default_api_root(&self) -> String {
        self.inner.default_api_root().to_string()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let authority = slf.getattr("authority")?.repr()?;
        let object_id = slf.getattr("object_id")?.repr()?;
        Ok(format!(
            "{name}(authority={authority}, object_id={object_id})"
        ))
    }
}
