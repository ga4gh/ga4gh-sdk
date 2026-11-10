//! Python bindings of the DRS client.
//!
//! `DRS` is the client for one DRS service: it reads objects and access URLs, one at a time
//! or in bulk, and can wait for objects that the server is still preparing. `DrsResolver`
//! turns hostname-based `drs://` URIs into access plans. `DrsObject`, `AccessURL` and
//! `BulkAccessURL` are results that keep the JSON they were built from.

// The `#[pymethods]` expansion of PyO3 0.22 converts each `PyResult` into itself, which
// clippy reports at every method that returns one, as in `tes.rs`.
#![allow(clippy::useless_conversion)]

use crate::utils::convert::{check_extra_args, to_json, to_python};
use crate::utils::runtime::io;
use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::drs::bulk::{BulkKey, BulkPolicy, BulkReport, ObjectOutcome};
use ga4gh_sdk::clients::drs::models::access_method::Type;
use ga4gh_sdk::clients::drs::models::{AccessUrl, BulkAccessUrl, DrsObject};
use ga4gh_sdk::clients::drs::resolution::{
    resolve_until, PendingResolution, Resolution, ResolutionReference, ResolveCause, ResolveError,
};
use ga4gh_sdk::clients::drs::uri::{
    AccessPlan, AccessPreferences, AccessResolution, DrsResolver, DrsUri,
};
use ga4gh_sdk::clients::drs::DRS;
use ga4gh_sdk::utils::operation::{Operation, RetryPolicy};
use ga4gh_sdk::utils::transport::urlencode;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyMapping, PyString, PyTuple, PyType};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

/// Registers the DRS classes in a module.
pub fn register(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDRS>()?;
    m.add_class::<PyDrsObject>()?;
    m.add_class::<PyAccessURL>()?;
    m.add_class::<PyBulkAccessURL>()?;
    m.add_class::<PyPendingResolution>()?;
    m.add_class::<PyBulkPolicy>()?;
    m.add_class::<PyBulkOutcome>()?;
    m.add_class::<PyBulkReport>()?;
    m.add_class::<PyDrsUri>()?;
    m.add_class::<PyDrsResolver>()?;
    m.add_class::<PyAccessPlan>()?;
    Ok(())
}

fn invalid_response(error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(format!("Invalid DRS response: {error}"))
}

/// Parses a server's JSON object into a DRS model.
fn parse_model<T: DeserializeOwned>(raw: &Value) -> Result<T, String> {
    if !raw.is_object() {
        return Err("not a JSON object".to_owned());
    }
    serde_json::from_value(raw.clone()).map_err(|error| model_error(&error))
}

/// Describes a model mismatch without quoting the server's values, which may be
/// credentials such as a signed URL or a bearer token in an access URL.
///
/// serde_json has no structured access to the field of an error, so this relies on its
/// message text; `test_invalid_responses_raise_runtime_error` pins it.
fn model_error(error: &serde_json::Error) -> String {
    let message = error.to_string();
    if message.starts_with("missing field") {
        message
    } else {
        "a field has the wrong type or value".to_owned()
    }
}

/// Describes an error of the library's typed DRS calls, as used by `DrsResolver`,
/// without the response values that their model errors quote.
fn plan_error(error: &(dyn std::error::Error + 'static)) -> String {
    let invalid =
        |error: &serde_json::Error| format!("Invalid DRS response: {}", model_error(error));
    if let Some(error) = error.downcast_ref::<serde_json::Error>() {
        return invalid(error);
    }
    if let Some(ResolveError {
        cause: ResolveCause::Request(cause),
        ..
    }) = error.downcast_ref::<ResolveError>()
    {
        if let Some(cause) = cause.downcast_ref::<serde_json::Error>() {
            return format!("DRS resolution request failed: {}", invalid(cause));
        }
    }
    error.to_string()
}

/// Converts a Python `int` to a count; negative values raise `ValueError`, not the
/// `OverflowError` of a direct conversion.
fn count(value: i64, what: &str) -> PyResult<usize> {
    usize::try_from(value)
        .map_err(|_| PyValueError::new_err(format!("{what} must not be negative")))
}

/// Parses a server's JSON text. Errors are `RuntimeError`s.
fn parse_body(body: &str) -> PyResult<Value> {
    serde_json::from_str(body).map_err(invalid_response)
}

/// A field of a JSON object as a Python object, or `None` if it is missing or `null`.
fn field(py: Python<'_>, raw: &Value, key: &str) -> PyResult<Option<PyObject>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => to_python(py, value).map(Some),
    }
}

/// Rejects IDs that can't be a path segment: empty, `.` or `..`.
fn require_id(id: &str, what: &str) -> PyResult<()> {
    if id.is_empty() {
        return Err(PyValueError::new_err(format!("{what} must not be empty")));
    }
    if matches!(id, "." | "..") {
        return Err(PyValueError::new_err(format!("{what} must not be {id:?}")));
    }
    Ok(())
}

/// The endpoint of an object, below the API root.
fn object_endpoint(object_id: &str) -> PyResult<String> {
    require_id(object_id, "object ID")?;
    Ok(format!("objects/{}", urlencode(object_id)))
}

/// The endpoint of an object's access URL, below the API root.
fn access_endpoint(object_id: &str, access_id: &str) -> PyResult<String> {
    require_id(access_id, "access ID")?;
    Ok(format!(
        "{}/access/{}",
        object_endpoint(object_id)?,
        urlencode(access_id)
    ))
}

/// An operation budget of `timeout` seconds.
fn budget(timeout: f64) -> PyResult<Operation> {
    let invalid = || PyValueError::new_err("timeout must be a positive number of seconds");
    let duration = Duration::try_from_secs_f64(timeout).map_err(|_| invalid())?;
    if duration.is_zero() {
        return Err(invalid());
    }
    // `Operation::new` adds the budget to the current time, which must not overflow.
    if std::time::Instant::now().checked_add(duration).is_none() {
        return Err(PyValueError::new_err("timeout is too long"));
    }
    Ok(Operation::new(duration))
}

/// Checks the number of attempts of a waiting call, as `resolve_until` does.
fn require_attempts(max_attempts: i64) -> PyResult<usize> {
    if !(1..=10_000).contains(&max_attempts) {
        return Err(PyValueError::new_err(
            "max_attempts must be between 1 and 10000",
        ));
    }
    count(max_attempts, "max_attempts")
}

/// A duration in seconds, for `BulkPolicy`.
fn seconds(value: f64, what: &str) -> PyResult<Duration> {
    Duration::try_from_secs_f64(value).map_err(|_| {
        PyValueError::new_err(format!("{what} must be a non-negative number of seconds"))
    })
}

/// Collects the `str` items of an iterable; a `str` itself is not taken as one.
fn strings(values: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec<String>> {
    let error = || PyValueError::new_err(format!("{what} must be an iterable of str"));
    if values.is_instance_of::<PyString>() {
        return Err(error());
    }
    values
        .iter()
        .map_err(|_| error())?
        .map(|item| item?.extract::<String>().map_err(|_| error()))
        .collect()
}

/// A DRS object's metadata, as returned by the server.
///
/// `json()` and `to_dict()` return the object's JSON as received, including fields that the
/// DRS models don't know.
#[pyclass(name = "DrsObject", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyDrsObject {
    /// The object parsed into the DRS model; derived from `raw`.
    pub inner: DrsObject,
    /// The object's JSON as received.
    pub raw: Value,
}

impl PyDrsObject {
    fn parse(raw: Value) -> Result<Self, String> {
        Ok(PyDrsObject {
            inner: parse_model(&raw)?,
            raw,
        })
    }

    /// Builds an object from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }
}

#[pymethods]
impl PyDrsObject {
    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name.clone()
    }

    #[getter]
    fn self_uri(&self) -> String {
        self.inner.self_uri.clone()
    }

    #[getter]
    fn size(&self) -> i64 {
        self.inner.size
    }

    #[getter]
    fn created_time(&self) -> String {
        self.inner.created_time.clone()
    }

    #[getter]
    fn updated_time(&self) -> Option<String> {
        self.inner.updated_time.clone()
    }

    #[getter]
    fn version(&self) -> Option<String> {
        self.inner.version.clone()
    }

    #[getter]
    fn mime_type(&self) -> Option<String> {
        self.inner.mime_type.clone()
    }

    #[getter]
    fn checksums(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "checksums")
    }

    #[getter]
    fn access_methods(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "access_methods")
    }

    #[getter]
    fn contents(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "contents")
    }

    #[getter]
    fn description(&self) -> Option<String> {
        self.inner.description.clone()
    }

    #[getter]
    fn aliases(&self) -> Option<Vec<String>> {
        self.inner.aliases.clone()
    }

    /// The object as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The object as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let size = slf.borrow().inner.size;
        Ok(format!("{name}(id={id}, size={size})"))
    }
}

/// An access URL of a DRS object, as returned by the server.
///
/// The URL and headers may hold credentials, such as a signature or a bearer token, so
/// `repr()` doesn't show them.
#[pyclass(name = "AccessURL", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyAccessURL {
    /// The access URL parsed into the DRS model; derived from `raw`.
    pub inner: AccessUrl,
    /// The access URL's JSON as received.
    pub raw: Value,
}

impl PyAccessURL {
    /// Builds an access URL from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Ok(PyAccessURL {
            inner: parse_model(&raw).map_err(invalid_response)?,
            raw,
        })
    }
}

#[pymethods]
impl PyAccessURL {
    #[getter]
    fn url(&self) -> String {
        self.inner.url.clone()
    }

    /// The headers to send with a request to `url`, as `"Name: value"` strings.
    #[getter]
    fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers.clone()
    }

    /// The access URL as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The access URL as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let headers = slf.borrow().inner.headers.as_ref().map_or(0, Vec::len);
        Ok(format!("{name}(url=<redacted>, headers={headers})"))
    }
}

/// An access URL from `DRS.get_bulk_access_urls()`, with the IDs it belongs to.
///
/// Like `AccessURL`, `repr()` doesn't show the URL or headers.
#[pyclass(name = "BulkAccessURL", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyBulkAccessURL {
    /// The access URL parsed into the DRS model; derived from `raw`.
    pub inner: BulkAccessUrl,
    /// The entry's JSON as received.
    pub raw: Value,
}

impl PyBulkAccessURL {
    fn parse(raw: Value) -> Result<Self, String> {
        Ok(PyBulkAccessURL {
            inner: parse_model(&raw)?,
            raw,
        })
    }
}

#[pymethods]
impl PyBulkAccessURL {
    #[getter]
    fn drs_object_id(&self) -> Option<String> {
        self.inner.drs_object_id.clone()
    }

    #[getter]
    fn drs_access_id(&self) -> Option<String> {
        self.inner.drs_access_id.clone()
    }

    #[getter]
    fn url(&self) -> String {
        self.inner.url.clone()
    }

    /// The headers to send with a request to `url`, as `"Name: value"` strings.
    #[getter]
    fn headers(&self) -> Option<Vec<String>> {
        self.inner.headers.clone()
    }

    /// The entry as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The entry as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let object_id = slf.getattr("drs_object_id")?.repr()?;
        let headers = slf.borrow().inner.headers.as_ref().map_or(0, Vec::len);
        Ok(format!(
            "{name}(drs_object_id={object_id}, url=<redacted>, headers={headers})"
        ))
    }
}

/// The server is still preparing an object or access URL: ask again after `retry_after`
/// seconds.
///
/// `api_root` and `endpoint` say what to ask for; they hold no credentials.
#[pyclass(name = "PendingResolution", module = "GA4GH", subclass)]
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
fn ready_or_pending<T: IntoPy<PyObject>>(
    py: Python<'_>,
    answer: Resolution<String>,
    convert: impl FnOnce(Value) -> PyResult<T>,
) -> PyResult<PyObject> {
    match answer {
        Resolution::Ready(body) => Ok(convert(parse_body(&body)?)?.into_py(py)),
        Resolution::Pending(inner) => Ok(PyPendingResolution { inner }.into_py(py)),
    }
}

/// Limits of `DRS.get_bulk_objects()` and `DRS.get_bulk_access_urls()`.
///
/// The values are checked when a bulk call starts; invalid ones raise `ValueError` there.
#[pyclass(name = "BulkPolicy", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyBulkPolicy {
    pub inner: BulkPolicy,
}

#[pymethods]
impl PyBulkPolicy {
    /// All arguments are optional. Delays are in seconds.
    #[new]
    #[pyo3(signature = (
        chunk_size=100, advertised_limit=None, concurrency=4, max_requests=1000,
        max_split_depth=8, max_attempts=1, initial_delay=0.2, max_delay=10.0
    ))]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chunk_size: i64,
        advertised_limit: Option<i64>,
        concurrency: i64,
        max_requests: i64,
        max_split_depth: i64,
        max_attempts: i64,
        initial_delay: f64,
        max_delay: f64,
    ) -> PyResult<Self> {
        Ok(PyBulkPolicy {
            inner: BulkPolicy {
                chunk_size: count(chunk_size, "chunk_size")?,
                advertised_limit: advertised_limit
                    .map(|limit| count(limit, "advertised_limit"))
                    .transpose()?,
                concurrency: count(concurrency, "concurrency")?,
                max_requests: count(max_requests, "max_requests")?,
                max_split_depth: count(max_split_depth, "max_split_depth")?,
                retry: RetryPolicy {
                    max_attempts: count(max_attempts, "max_attempts")?,
                    initial_delay: seconds(initial_delay, "initial_delay")?,
                    max_delay: seconds(max_delay, "max_delay")?,
                },
                ..Default::default()
            },
        })
    }

    /// A copy of the policy that also keeps to the `maxBulkRequestLength` of a service's
    /// service info (a `dict` or JSON text), if it has one.
    fn with_service_info(&self, service_info: &Bound<'_, PyAny>) -> PyResult<Self> {
        let raw = to_json(service_info, "service info")?;
        let inner = self
            .inner
            .clone()
            .with_service_info(&raw)
            .map_err(|error| PyValueError::new_err(format!("Invalid service info: {error}")))?;
        Ok(PyBulkPolicy { inner })
    }

    /// The most IDs per request.
    #[getter]
    fn chunk_size(&self) -> usize {
        self.inner.chunk_size
    }

    /// The service's most IDs per request, if known; it lowers `chunk_size`.
    #[getter]
    fn advertised_limit(&self) -> Option<usize> {
        self.inner.advertised_limit
    }

    /// The most requests at the same time.
    #[getter]
    fn concurrency(&self) -> usize {
        self.inner.concurrency
    }

    /// The most requests in one call, retries and splits included.
    #[getter]
    fn max_requests(&self) -> usize {
        self.inner.max_requests
    }

    /// How often a request that the server rejects as too large (413) is halved.
    #[getter]
    fn max_split_depth(&self) -> usize {
        self.inner.max_split_depth
    }

    /// The most attempts per request, for network errors, 429 and 503.
    #[getter]
    fn max_attempts(&self) -> usize {
        self.inner.retry.max_attempts
    }

    /// The delay before the first retry, in seconds; it doubles up to `max_delay`.
    #[getter]
    fn initial_delay(&self) -> f64 {
        self.inner.retry.initial_delay.as_secs_f64()
    }

    /// The longest delay between retries, in seconds.
    #[getter]
    fn max_delay(&self) -> f64 {
        self.inner.retry.max_delay.as_secs_f64()
    }
}

/// The outcome for one requested ID (or ID pair) of a bulk call.
///
/// `repr()` shows counts only, as resolved access URLs and error texts may hold
/// credentials.
#[pyclass(name = "BulkOutcome", module = "GA4GH", subclass)]
pub struct PyBulkOutcome {
    pub object_id: String,
    pub access_id: Option<String>,
    /// `DrsObject`s or `BulkAccessURL`s.
    pub resolved: Vec<PyObject>,
    pub failure_codes: Vec<u16>,
    pub pending: Option<PyPendingResolution>,
    pub errors: Vec<String>,
    pub is_success: bool,
}

impl PyBulkOutcome {
    /// Converts a lib outcome, checking each resolved entry against its model as
    /// `get_bulk_objects` and `get_bulk_access_urls` do.
    fn convert<T: IntoPy<PyObject>>(
        py: Python<'_>,
        key: BulkKey,
        outcome: ObjectOutcome<Value>,
        parse: impl Fn(Value) -> Result<T, String>,
    ) -> Self {
        // Rebuilt with the entries that fit the model (as placeholders), so that
        // `is_success` is the lib's own `ObjectOutcome::is_success`.
        let mut checked = ObjectOutcome {
            failure_codes: outcome.failure_codes,
            pending: outcome.pending,
            errors: outcome.errors,
            ..Default::default()
        };
        let mut resolved = Vec::new();
        for value in outcome.resolved {
            match parse(value) {
                Ok(item) => {
                    resolved.push(item.into_py(py));
                    checked.resolved.push(Value::Null);
                }
                Err(error) => checked
                    .errors
                    .push(format!("invalid resolved entry: {error}")),
            }
        }
        PyBulkOutcome {
            object_id: key.object_id,
            access_id: key.access_id,
            resolved,
            is_success: checked.is_success(),
            failure_codes: checked.failure_codes,
            pending: checked.pending.map(|pending| PyPendingResolution {
                inner: PendingResolution {
                    reference: ResolutionReference {
                        api_root: pending.api_root,
                        endpoint: pending.endpoint,
                    },
                    retry_after_secs: pending.retry_after_secs,
                },
            }),
            errors: checked.errors,
        }
    }
}

#[pymethods]
impl PyBulkOutcome {
    #[getter]
    fn object_id(&self) -> String {
        self.object_id.clone()
    }

    /// The access ID of the pair, for `get_bulk_access_urls()`; else `None`.
    #[getter]
    fn access_id(&self) -> Option<String> {
        self.access_id.clone()
    }

    /// The `DrsObject`s or `BulkAccessURL`s that the server returned for the ID; one, if all
    /// went well.
    #[getter]
    fn resolved(&self, py: Python<'_>) -> Vec<PyObject> {
        self.resolved
            .iter()
            .map(|item| item.clone_ref(py))
            .collect()
    }

    /// The HTTP status codes that the server gave for the ID, such as 404.
    #[getter]
    fn failure_codes(&self) -> Vec<u16> {
        self.failure_codes.clone()
    }

    /// The pending state, if the server is still preparing the answer.
    #[getter]
    fn pending(&self) -> Option<PyPendingResolution> {
        self.pending.clone()
    }

    /// What went wrong otherwise, such as a missing or duplicate answer.
    #[getter]
    fn errors(&self) -> Vec<String> {
        self.errors.clone()
    }

    /// Whether the server returned exactly one result and nothing else for the ID.
    #[getter]
    fn is_success(&self) -> bool {
        self.is_success
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let object_id = slf.getattr("object_id")?.repr()?;
        let outcome = slf.borrow();
        Ok(format!(
            "{name}(object_id={object_id}, resolved={}, failure_codes={:?}, pending={}, errors={})",
            outcome.resolved.len(),
            outcome.failure_codes,
            if outcome.pending.is_some() {
                "True"
            } else {
                "False"
            },
            outcome.errors.len()
        ))
    }
}

/// The result of a bulk call: one `BulkOutcome` per distinct requested ID (or ID pair),
/// sorted by ID, and the problems with the server's answers that belong to no ID.
#[pyclass(name = "BulkReport", module = "GA4GH", subclass)]
pub struct PyBulkReport {
    pub outcomes: Vec<Py<PyBulkOutcome>>,
    pub diagnostics: Vec<String>,
}

impl PyBulkReport {
    fn convert<T: IntoPy<PyObject>>(
        py: Python<'_>,
        report: BulkReport<Value>,
        parse: impl Fn(Value) -> Result<T, String> + Copy,
    ) -> PyResult<Self> {
        let outcomes = report
            .outcomes
            .into_iter()
            .map(|(key, outcome)| Py::new(py, PyBulkOutcome::convert(py, key, outcome, parse)))
            .collect::<PyResult<_>>()?;
        Ok(PyBulkReport {
            outcomes,
            diagnostics: report.diagnostics,
        })
    }
}

#[pymethods]
impl PyBulkReport {
    #[getter]
    fn outcomes(&self, py: Python<'_>) -> Vec<Py<PyBulkOutcome>> {
        self.outcomes
            .iter()
            .map(|item| item.clone_ref(py))
            .collect()
    }

    #[getter]
    fn diagnostics(&self) -> Vec<String> {
        self.diagnostics.clone()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let report = slf.borrow();
        Ok(format!(
            "{name}(outcomes={}, diagnostics={})",
            report.outcomes.len(),
            report.diagnostics.len()
        ))
    }
}

/// The `(object_id, access_id)` pairs of `DRS.get_bulk_access_urls()`.
fn access_pairs(pairs: &Bound<'_, PyAny>) -> PyResult<Vec<BulkKey>> {
    const PAIRS: &str = "pairs must be a mapping or an iterable of (object_id, access_id) pairs";
    if pairs.is_instance_of::<PyString>() {
        return Err(PyValueError::new_err(PAIRS));
    }
    let items = match pairs.downcast::<PyMapping>() {
        Ok(mapping) => mapping.items()?.into_any(),
        Err(_) => pairs.clone(),
    };
    items
        .iter()
        .map_err(|_| PyValueError::new_err(PAIRS))?
        .map(|pair| {
            let (object_id, access_id): (String, String) =
                pair?.extract().map_err(|_| PyValueError::new_err(PAIRS))?;
            Ok(BulkKey {
                object_id,
                access_id: Some(access_id),
            })
        })
        .collect()
}

/// A DRS client. Subclass it to add endpoints, using `self.transport`.
///
/// The public `inner` preserves the Rust extension API and is safe to share
/// between Python threads while synchronous requests release the GIL.
#[pyclass(name = "DRS", module = "GA4GH", subclass)]
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
}

/// A hostname-based DRS URI, such as `drs://drs.example.org/object-id`.
#[pyclass(name = "DrsUri", module = "GA4GH", subclass)]
#[derive(Clone)]
pub struct PyDrsUri {
    pub inner: DrsUri,
}

/// Parses a DRS URI; unsupported or malformed ones raise `ValueError`.
fn parse_uri(uri: &str) -> PyResult<DrsUri> {
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

/// How to fetch a DRS object's bytes: an access method, its URL and its headers.
///
/// The URL and headers may hold credentials, so `repr()` shows only the URL's origin.
#[pyclass(name = "AccessPlan", module = "GA4GH", subclass)]
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
        access_type_name(self.inner.method)
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

fn access_type_name(method: Type) -> PyResult<String> {
    match serde_json::to_value(method) {
        Ok(Value::String(name)) => Ok(name),
        _ => Err(PyRuntimeError::new_err("Unknown access method type")),
    }
}

fn parse_access_type(name: &str) -> PyResult<Type> {
    serde_json::from_value(Value::String(name.to_lowercase())).map_err(|_| {
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
#[pyclass(name = "DrsResolver", module = "GA4GH", subclass)]
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
