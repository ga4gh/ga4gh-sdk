//! `DrsResolver` and `AccessPlan`: plan access to the objects of `drs://` URIs.

// The `#[pymethods]` expansion of PyO3 0.22 converts each `PyResult` into itself, which
// clippy reports at every method that returns one, as in `tes.rs`.
#![allow(clippy::useless_conversion)]

use super::helpers::{budget, count, plan_error, require_attempts};
use super::pending::PyPendingResolution;
use super::uri::parse_uri;
use crate::utils::convert::check_extra_args;
use crate::utils::runtime::io;
use crate::PyTransport;
use ga4gh_sdk::clients::drs::models::access_method::Type;
use ga4gh_sdk::clients::drs::uri::{AccessPlan, AccessPreferences, AccessResolution, DrsResolver};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use serde_json::Value;

/// How to fetch a DRS object's bytes: an access method, its URL and its headers.
///
/// The URL and headers may hold credentials, so `repr()` shows only the URL's origin.
#[pyclass(name = "AccessPlan", module = "ga4gh_sdk", subclass)]
pub struct PyAccessPlan {
    pub inner: AccessPlan,
}

#[pymethods]
impl PyAccessPlan {
    #[getter]
    fn object_id(&self) -> String {
        self.inner.object_id.clone()
    }

    /// The type of the access method, such as `"https"` or `"s3"`.
    #[getter]
    fn method(&self) -> PyResult<String> {
        access_type_name(&self.inner.method)
    }

    #[getter]
    fn region(&self) -> Option<String> {
        self.inner.region.clone()
    }

    #[getter]
    fn url(&self) -> String {
        self.inner.url.to_string()
    }

    /// The headers to send with a request to `url`, as `(name, value)` pairs (unlike the
    /// `"Name: value"` strings of `AccessURL`).
    #[getter]
    fn headers(&self) -> Vec<(String, String)> {
        self.inner
            .headers
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    String::from_utf8_lossy(value.as_bytes()).into_owned(),
                )
            })
            .collect()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let object_id = slf.getattr("object_id")?.repr()?;
        let method = slf.getattr("method")?.repr()?;
        let origin = slf.borrow().inner.url.origin().ascii_serialization();
        Ok(format!(
            "{name}(object_id={object_id}, method={method}, origin='{origin}')"
        ))
    }
}

pub(super) fn access_type_name(method: &Type) -> PyResult<String> {
    match serde_json::to_value(method) {
        Ok(Value::String(name)) => Ok(name),
        _ => Err(PyRuntimeError::new_err("Unknown access method type")),
    }
}

pub(super) fn parse_access_type(name: &str) -> PyResult<Type> {
    serde_json::from_value::<Type>(Value::String(name.to_lowercase()))
        .ok()
        .filter(|method| !matches!(method, Type::Unrecognized(_)))
        .ok_or_else(|| {
            PyValueError::new_err(format!(
                "Invalid access method type: {name:?}; expected s3, gs, ftp, gsiftp, globus, \
             htsget, https or file"
            ))
        })
}

/// Turns hostname-based DRS URIs into `AccessPlan`s.
///
/// By default, a URI's host is asked anonymously at `https://<host>/ga4gh/drs/v1`.
/// `with_service()` maps a host to a `Transport` with its own API root and settings;
/// settings are never sent to other hosts. It never downloads object bytes.
#[pyclass(name = "DrsResolver", module = "ga4gh_sdk", subclass)]
pub struct PyDrsResolver {
    pub inner: DrsResolver,
}

impl PyDrsResolver {
    fn preferences(
        protocols: Option<Vec<String>>,
        regions: Option<Vec<String>>,
        method_index: Option<i64>,
    ) -> PyResult<AccessPreferences> {
        Ok(AccessPreferences {
            protocols: protocols
                .unwrap_or_default()
                .iter()
                .map(|name| parse_access_type(name))
                .collect::<PyResult<_>>()?,
            regions: regions.unwrap_or_default(),
            method_index: method_index
                .map(|index| count(index, "method_index"))
                .transpose()?,
        })
    }
}

#[pymethods]
impl PyDrsResolver {
    #[new]
    #[classmethod]
    #[pyo3(signature = (*args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Ok(PyDrsResolver {
            inner: DrsResolver::default(),
        })
    }

    /// Asks the DRS service of `transport` for URIs with host `authority`. Returns the
    /// resolver itself, for chaining.
    fn with_service<'py>(
        slf: Bound<'py, Self>,
        authority: &str,
        transport: &PyTransport,
    ) -> PyResult<Bound<'py, Self>> {
        // Checked first: `DrsResolver::with_service` drops the resolver on an error.
        parse_uri(&format!("drs://{authority}/placeholder"))
            .map_err(|_| PyValueError::new_err(format!("Invalid DRS host: {authority:?}")))?;
        {
            // A call on another thread holds the resolver while it waits for the network.
            let mut resolver = slf
                .try_borrow_mut()
                .map_err(|_| PyRuntimeError::new_err("The resolver is in use by another thread"))?;
            let inner = std::mem::take(&mut resolver.inner);
            resolver.inner = inner
                .with_service(authority, transport.inner.clone())
                .map_err(|error| PyValueError::new_err(error.to_string()))?;
        }
        Ok(slf)
    }

    /// Asks once for the object of `uri` and, if needed, its access URL. Returns an
    /// `AccessPlan`, or a `PendingResolution` if the server is still preparing an answer.
    ///
    /// `protocols` and `regions` list the access method types and regions to choose from,
    /// in order of preference; `method_index` picks one of the object's access methods
    /// instead.
    #[pyo3(signature = (uri, protocols=None, regions=None, method_index=None, timeout=60.0))]
    pub fn access_plan_once(
        &self,
        py: Python<'_>,
        uri: &str,
        protocols: Option<Vec<String>>,
        regions: Option<Vec<String>>,
        method_index: Option<i64>,
        timeout: f64,
    ) -> PyResult<PyObject> {
        parse_uri(uri)?;
        let preferences = Self::preferences(protocols, regions, method_index)?;
        let operation = budget(timeout)?;
        let answer = io(py, |rt| {
            rt.block_on(self.inner.access_plan_once(uri, &preferences, &operation))
                .map_err(|error| {
                    format!("Failed to plan DRS access: {}", plan_error(error.as_ref()))
                })
        })?;
        Ok(match answer {
            AccessResolution::Ready(inner) => PyAccessPlan { inner }.into_py(py),
            AccessResolution::Pending(inner) => PyPendingResolution { inner }.into_py(py),
        })
    }

    /// Like `access_plan_once()`, but waits as the server asks while it prepares an
    /// answer. Gives up after `max_attempts` requests per step or `timeout` seconds in
    /// all, with `RuntimeError`.
    #[pyo3(signature = (
        uri, protocols=None, regions=None, method_index=None, timeout=60.0, max_attempts=5
    ))]
    #[allow(clippy::too_many_arguments)]
    pub fn access_plan(
        &self,
        py: Python<'_>,
        uri: &str,
        protocols: Option<Vec<String>>,
        regions: Option<Vec<String>>,
        method_index: Option<i64>,
        timeout: f64,
        max_attempts: i64,
    ) -> PyResult<PyAccessPlan> {
        parse_uri(uri)?;
        let max_attempts = require_attempts(max_attempts)?;
        let preferences = Self::preferences(protocols, regions, method_index)?;
        let operation = budget(timeout)?;
        io(py, |rt| {
            rt.block_on(
                self.inner
                    .access_plan(uri, &preferences, &operation, max_attempts),
            )
            .map(|inner| PyAccessPlan { inner })
            .map_err(|error| format!("Failed to plan DRS access: {}", plan_error(error.as_ref())))
        })
    }
}
