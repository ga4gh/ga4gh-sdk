//! Helpers shared by the DRS classes: parsing and checking responses and arguments,
//! endpoints, and time budgets. They don't define Python classes.

use crate::utils::convert::to_python;
use crate::utils::json;
use crate::utils::validate::require_id;
use ga4gh_sdk::clients::drs::resolution::{ResolveCause, ResolveError};
use ga4gh_sdk::utils::operation::Operation;
use ga4gh_sdk::utils::transport::urlencode;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

/// A `RuntimeError` for a DRS response that doesn't fit its models.
pub(super) fn invalid_response(error: impl std::fmt::Display) -> PyErr {
    json::invalid_response("DRS", error)
}

/// Parses a server's JSON object into a DRS model.
pub(super) fn parse_model<T: DeserializeOwned>(raw: &Value) -> Result<T, String> {
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
pub(super) fn model_error(error: &serde_json::Error) -> String {
    let message = error.to_string();
    if message.starts_with("missing field") {
        message
    } else {
        "a field has the wrong type or value".to_owned()
    }
}

/// Describes an error of the library's typed DRS calls, as used by `DrsResolver`,
/// without the response values that their model errors quote.
pub(super) fn plan_error(error: &(dyn std::error::Error + 'static)) -> String {
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
pub(super) fn count(value: i64, what: &str) -> PyResult<usize> {
    usize::try_from(value)
        .map_err(|_| PyValueError::new_err(format!("{what} must not be negative")))
}

/// Parses a server's JSON text. Errors are `RuntimeError`s.
pub(super) fn parse_body(body: &str) -> PyResult<Value> {
    json::parse_body("DRS", body)
}

pub(super) fn response_object(py: Python<'_>, raw: &Value) -> PyResult<Py<PyAny>> {
    if !raw.is_object() {
        return Err(invalid_response("not a JSON object"));
    }
    to_python(py, raw)
}

pub(super) fn raw_json(py: Python<'_>, raw: Value) -> Result<Py<PyAny>, String> {
    response_object(py, &raw).map_err(|_| "could not convert JSON object to Python".to_owned())
}

/// The endpoint of an object, below the API root.
pub(super) fn object_endpoint(object_id: &str) -> PyResult<String> {
    require_id(object_id, "object ID")?;
    Ok(format!("objects/{}", urlencode(object_id)))
}

/// The endpoint of an object's access URL, below the API root.
pub(super) fn access_endpoint(object_id: &str, access_id: &str) -> PyResult<String> {
    require_id(access_id, "access ID")?;
    Ok(format!(
        "{}/access/{}",
        object_endpoint(object_id)?,
        urlencode(access_id)
    ))
}

/// An operation budget of `timeout` seconds.
pub(super) fn budget(timeout: f64) -> PyResult<Operation> {
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
pub(super) fn require_attempts(max_attempts: i64) -> PyResult<usize> {
    if !(1..=10_000).contains(&max_attempts) {
        return Err(PyValueError::new_err(
            "max_attempts must be between 1 and 10000",
        ));
    }
    count(max_attempts, "max_attempts")
}

/// A duration in seconds, for `BulkPolicy`.
pub(super) fn seconds(value: f64, what: &str) -> PyResult<Duration> {
    Duration::try_from_secs_f64(value).map_err(|_| {
        PyValueError::new_err(format!("{what} must be a non-negative number of seconds"))
    })
}

/// Collects the `str` items of an iterable; a `str` itself is not taken as one.
pub(super) fn strings(values: &Bound<'_, PyAny>, what: &str) -> PyResult<Vec<String>> {
    let error = || PyValueError::new_err(format!("{what} must be an iterable of str"));
    if values.is_instance_of::<PyString>() {
        return Err(error());
    }
    values
        .try_iter()
        .map_err(|_| error())?
        .map(|item| item?.extract::<String>().map_err(|_| error()))
        .collect()
}
