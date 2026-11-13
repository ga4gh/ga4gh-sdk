//! `DRS`: the client for one DRS service.

// The `#[pymethods]` expansion of PyO3 0.22 converts each `PyResult` into itself, which
// clippy reports at every method that returns one, as in `tes.rs`.
#![allow(clippy::useless_conversion)]

use super::bulk::{access_pairs, PyBulkPolicy, PyBulkReport};
use super::helpers::{
    access_endpoint, budget, object_endpoint, parse_body, raw_json, require_attempts,
    response_object, strings,
};
use super::objects::{PyAccessURL, PyBulkAccessURL, PyDrsObject};
use super::pending::ready_or_pending;
use crate::utils::convert::check_extra_args;
use crate::utils::runtime::io;
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::drs::bulk::{BulkKey, BulkReport};
use ga4gh_sdk::clients::drs::resolution::{resolve_until, Resolution};
use ga4gh_sdk::clients::drs::DRS;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use serde_json::Value;

/// A DRS client. Subclass it to add endpoints, using `self.transport`.
///
/// The public `inner` preserves the Rust extension API and is safe to share
/// between Python threads while synchronous requests release the GIL.
#[pyclass(name = "DRS", module = "ga4gh_sdk", subclass)]
pub struct PyDRS {
    pub inner: DRS,
}

impl PyDRS {
    /// Build a DRS client for Rust extension crates while releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::with_gil(|py| Self::new_with_gil(py, config))
    }

    fn new_with_gil(py: Python<'_>, config: &PyConfiguration) -> PyResult<Self> {
        io(py, |rt| {
            rt.block_on(DRS::new(&config.inner))
                .map(|inner| Self { inner })
                .map_err(|error| format!("Failed to create DRS instance: {error}"))
        })
    }

    /// Sends one request for `endpoint`; a pending answer is not an error.
    fn once(&self, py: Python<'_>, endpoint: String, what: &str) -> PyResult<Resolution<String>> {
        let reference = self.inner.resolution_reference(endpoint);
        io(py, |rt| {
            rt.block_on(self.inner.resolve_reference_raw(&reference))
                .map_err(|error| format!("Failed to get {what}: {error}"))
        })
    }

    /// Sends one request for `endpoint`; a pending answer raises `RuntimeError`.
    fn ready(&self, py: Python<'_>, endpoint: String, what: &str) -> PyResult<Value> {
        match self.once(py, endpoint, what)? {
            Resolution::Ready(body) => parse_body(&body),
            Resolution::Pending(pending) => Err(PyRuntimeError::new_err(format!(
                "Failed to get {what}: {pending}"
            ))),
        }
    }

    /// Asks for `endpoint` until the server stops answering that it is pending.
    fn wait(
        &self,
        py: Python<'_>,
        endpoint: String,
        timeout: f64,
        max_attempts: i64,
        what: &str,
    ) -> PyResult<Value> {
        let max_attempts = require_attempts(max_attempts)?;
        let operation = budget(timeout)?;
        let reference = self.inner.resolution_reference(endpoint);
        let body = io(py, |rt| {
            rt.block_on(resolve_until(
                reference.clone(),
                &operation,
                max_attempts,
                || self.inner.resolve_reference_raw(&reference),
            ))
            .map_err(|error| {
                let mut message = format!("Failed to resolve {what}: {error}");
                if let Some(pending) = error.last_pending {
                    message.push_str(&format!(
                        " (last answer: retry after {} seconds)",
                        pending.retry_after_secs
                    ));
                }
                message
            })
        })?;
        parse_body(&body)
    }

    /// Runs a bulk call. Invalid IDs and policies raise `ValueError`.
    fn bulk(
        &self,
        py: Python<'_>,
        keys: Vec<BulkKey>,
        access: bool,
        passports: Option<Vec<String>>,
        policy: Option<PyBulkPolicy>,
        timeout: f64,
    ) -> PyResult<BulkReport<Value>> {
        let operation = budget(timeout)?;
        let passports = passports.unwrap_or_default();
        let policy = policy.map(|policy| policy.inner).unwrap_or_default();
        let answer: Result<BulkReport<Value>, String> = io(py, |rt| {
            Ok(rt
                .block_on(async {
                    if access {
                        self.inner
                            .get_bulk_access_urls_raw(&keys, &passports, &policy, &operation)
                            .await
                    } else {
                        let ids = keys
                            .into_iter()
                            .map(|key| key.object_id)
                            .collect::<Vec<_>>();
                        self.inner
                            .get_bulk_objects_raw(&ids, &passports, &policy, &operation)
                            .await
                    }
                })
                .map_err(|error| error.to_string()))
        })?;
        // The bulk calls report request failures per ID; an error means invalid input.
        answer.map_err(|error| PyValueError::new_err(format!("Invalid bulk request: {error}")))
    }
}

#[pymethods]
impl PyDRS {
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

    /// Gets an object's metadata as a `DrsObject`.
    ///
    /// If the server is still preparing the object, this raises `RuntimeError`; use
    /// `get_object_once()` or `resolve_object()` to handle that case.
    pub fn get_object(&self, py: Python<'_>, object_id: &str) -> PyResult<PyDrsObject> {
        let raw = self.ready(py, object_endpoint(object_id)?, "DRS object")?;
        PyDrsObject::from_json(raw)
    }

    /// Gets the object's complete JSON as a `dict`, without model validation.
    /// This also accepts provider extensions such as `created_time: null`.
    pub fn get_object_raw(&self, py: Python<'_>, object_id: &str) -> PyResult<PyObject> {
        let raw = self.ready(py, object_endpoint(object_id)?, "DRS object")?;
        response_object(py, &raw)
    }

    /// Asks once for an object: returns a `DrsObject`, or a `PendingResolution` if the
    /// server is still preparing it.
    pub fn get_object_once(&self, py: Python<'_>, object_id: &str) -> PyResult<PyObject> {
        let answer = self.once(py, object_endpoint(object_id)?, "DRS object")?;
        ready_or_pending(py, answer, PyDrsObject::from_json)
    }

    /// Gets an object, waiting as the server asks while it prepares the object.
    ///
    /// Gives up after `max_attempts` requests or `timeout` seconds, with `RuntimeError`.
    #[pyo3(signature = (object_id, timeout=60.0, max_attempts=5))]
    pub fn resolve_object(
        &self,
        py: Python<'_>,
        object_id: &str,
        timeout: f64,
        max_attempts: i64,
    ) -> PyResult<PyDrsObject> {
        let endpoint = object_endpoint(object_id)?;
        let raw = self.wait(py, endpoint, timeout, max_attempts, "DRS object")?;
        PyDrsObject::from_json(raw)
    }

    /// Gets an access URL of an object as an `AccessURL`. `access_id` is the `access_id` of
    /// one of the object's access methods.
    ///
    /// If the server is still preparing the URL, this raises `RuntimeError`; use
    /// `get_access_url_once()` or `resolve_access_url()` to handle that case.
    pub fn get_access_url(
        &self,
        py: Python<'_>,
        object_id: &str,
        access_id: &str,
    ) -> PyResult<PyAccessURL> {
        let raw = self.ready(py, access_endpoint(object_id, access_id)?, "access URL")?;
        PyAccessURL::from_json(raw)
    }

    /// Gets the access URL's complete JSON as a `dict`, without model validation.
    pub fn get_access_url_raw(
        &self,
        py: Python<'_>,
        object_id: &str,
        access_id: &str,
    ) -> PyResult<PyObject> {
        let raw = self.ready(py, access_endpoint(object_id, access_id)?, "access URL")?;
        response_object(py, &raw)
    }

    /// Asks once for an access URL: returns an `AccessURL`, or a `PendingResolution` if the
    /// server is still preparing it.
    pub fn get_access_url_once(
        &self,
        py: Python<'_>,
        object_id: &str,
        access_id: &str,
    ) -> PyResult<PyObject> {
        let answer = self.once(py, access_endpoint(object_id, access_id)?, "access URL")?;
        ready_or_pending(py, answer, PyAccessURL::from_json)
    }

    /// Gets an access URL, waiting as the server asks while it prepares the URL.
    ///
    /// Gives up after `max_attempts` requests or `timeout` seconds, with `RuntimeError`.
    #[pyo3(signature = (object_id, access_id, timeout=60.0, max_attempts=5))]
    pub fn resolve_access_url(
        &self,
        py: Python<'_>,
        object_id: &str,
        access_id: &str,
        timeout: f64,
        max_attempts: i64,
    ) -> PyResult<PyAccessURL> {
        let endpoint = access_endpoint(object_id, access_id)?;
        let raw = self.wait(py, endpoint, timeout, max_attempts, "access URL")?;
        PyAccessURL::from_json(raw)
    }

    /// Gets the metadata of several objects with `POST objects` (DRS 1.4).
    ///
    /// `ids` is an iterable of object IDs; repeated IDs are asked for once. Returns a
    /// `BulkReport` whose outcomes hold `DrsObject`s. Failures of single IDs or requests
    /// are in the outcomes; invalid IDs or policies raise `ValueError`.
    #[pyo3(signature = (ids, passports=None, policy=None, timeout=60.0))]
    pub fn get_bulk_objects(
        &self,
        py: Python<'_>,
        ids: &Bound<'_, PyAny>,
        passports: Option<&Bound<'_, PyAny>>,
        policy: Option<PyBulkPolicy>,
        timeout: f64,
    ) -> PyResult<PyBulkReport> {
        let keys = strings(ids, "ids")?
            .into_iter()
            .map(|object_id| BulkKey {
                object_id,
                access_id: None,
            })
            .collect();
        let passports = passports.map(|p| strings(p, "passports")).transpose()?;
        let report = self.bulk(py, keys, false, passports, policy, timeout)?;
        PyBulkReport::convert(py, report, PyDrsObject::parse)
    }

    /// Like `get_bulk_objects()`, but each resolved entry is a complete JSON `dict`.
    /// The library still correlates entries by ID and records reply dialects.
    #[pyo3(signature = (ids, passports=None, policy=None, timeout=60.0))]
    pub fn get_bulk_objects_raw(
        &self,
        py: Python<'_>,
        ids: &Bound<'_, PyAny>,
        passports: Option<&Bound<'_, PyAny>>,
        policy: Option<PyBulkPolicy>,
        timeout: f64,
    ) -> PyResult<PyBulkReport> {
        let keys = strings(ids, "ids")?
            .into_iter()
            .map(|object_id| BulkKey {
                object_id,
                access_id: None,
            })
            .collect();
        let passports = passports.map(|p| strings(p, "passports")).transpose()?;
        let report = self.bulk(py, keys, false, passports, policy, timeout)?;
        PyBulkReport::convert(py, report, |raw| raw_json(py, raw))
    }

    /// Gets access URLs of several objects with `POST objects/access` (DRS 1.4).
    ///
    /// `pairs` maps object IDs to access IDs, or is an iterable of `(object_id, access_id)`
    /// pairs; each object may have one access ID per call. Returns a `BulkReport` whose
    /// outcomes hold `BulkAccessURL`s.
    #[pyo3(signature = (pairs, passports=None, policy=None, timeout=60.0))]
    pub fn get_bulk_access_urls(
        &self,
        py: Python<'_>,
        pairs: &Bound<'_, PyAny>,
        passports: Option<&Bound<'_, PyAny>>,
        policy: Option<PyBulkPolicy>,
        timeout: f64,
    ) -> PyResult<PyBulkReport> {
        let keys = access_pairs(pairs)?;
        let passports = passports.map(|p| strings(p, "passports")).transpose()?;
        let report = self.bulk(py, keys, true, passports, policy, timeout)?;
        PyBulkReport::convert(py, report, PyBulkAccessURL::parse)
    }

    /// Like `get_bulk_access_urls()`, but each resolved entry is a complete JSON `dict`.
    #[pyo3(signature = (pairs, passports=None, policy=None, timeout=60.0))]
    pub fn get_bulk_access_urls_raw(
        &self,
        py: Python<'_>,
        pairs: &Bound<'_, PyAny>,
        passports: Option<&Bound<'_, PyAny>>,
        policy: Option<PyBulkPolicy>,
        timeout: f64,
    ) -> PyResult<PyBulkReport> {
        let keys = access_pairs(pairs)?;
        let passports = passports.map(|p| strings(p, "passports")).transpose()?;
        let report = self.bulk(py, keys, true, passports, policy, timeout)?;
        PyBulkReport::convert(py, report, |raw| raw_json(py, raw))
    }
}
