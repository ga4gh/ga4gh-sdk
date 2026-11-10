//! Conversions between Python objects, JSON values and query strings.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;
use serde_json::Value;
use std::future::Future;

/// Runs a future to completion on a new Tokio runtime.
pub fn block_on<F: Future>(future: F) -> PyResult<F::Output> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| PyRuntimeError::new_err(format!("Cannot start async runtime: {error}")))?;
    Ok(runtime.block_on(future))
}

/// Converts a Python argument to JSON.
///
/// A `str` is parsed as JSON text. Any other object is serialised with Python's `json` module,
/// so `dict`, `list`, numbers, booleans and `None` work as expected.
pub fn to_json(value: &Bound<'_, PyAny>, what: &str) -> PyResult<Value> {
    let text = if let Ok(text) = value.downcast::<PyString>() {
        text.to_cow()?.into_owned()
    } else {
        let json = value.py().import_bound("json")?;
        json.call_method1("dumps", (value,))
            .and_then(|text| text.extract::<String>())
            .map_err(|error| {
                PyValueError::new_err(format!("Invalid {what}: not JSON-serialisable: {error}"))
            })?
    };
    serde_json::from_str(&text)
        .map_err(|error| PyValueError::new_err(format!("Invalid {what} JSON: {error}")))
}

/// Converts an optional Python argument to JSON, see [`to_json`].
pub fn to_json_opt(value: Option<&Bound<'_, PyAny>>, what: &str) -> PyResult<Option<Value>> {
    value.map(|value| to_json(value, what)).transpose()
}

/// Converts a JSON value to the equivalent Python object (`dict`, `list`, `str`, ...).
pub fn to_python(py: Python<'_>, value: &Value) -> PyResult<PyObject> {
    let json = py.import_bound("json")?;
    Ok(json.call_method1("loads", (value.to_string(),))?.unbind())
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// Appends query parameters to an endpoint.
///
/// `params` must be a JSON object. Its values may be strings, numbers, booleans, `null`
/// (skipped) or arrays of these (one parameter per item, as in `tag_key=a&tag_key=b`).
pub fn with_query(endpoint: &str, params: Option<Value>) -> PyResult<String> {
    let Some(params) = params else {
        return Ok(endpoint.to_owned());
    };
    let Value::Object(map) = params else {
        return Err(PyValueError::new_err("params must be a JSON object"));
    };
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in &map {
        let items = match value {
            Value::Null => continue,
            Value::Array(items) => items.iter().filter(|item| !item.is_null()).collect(),
            other => vec![other],
        };
        for item in items {
            let text = scalar(item).ok_or_else(|| {
                PyValueError::new_err(format!(
                    "params value for '{key}' must be a string, number, boolean or a list of these"
                ))
            })?;
            query.append_pair(key, &text);
        }
    }
    let query = query.finish();
    if query.is_empty() {
        return Ok(endpoint.to_owned());
    }
    let (path, fragment) = match endpoint.split_once('#') {
        Some((path, fragment)) => (path, Some(fragment)),
        None => (endpoint, None),
    };
    let separator = match path.find('?') {
        None => "?",
        Some(index) if index + 1 == path.len() || path.ends_with('&') => "",
        Some(_) => "&",
    };
    let mut result = format!("{path}{separator}{query}");
    if let Some(fragment) = fragment {
        result.push('#');
        result.push_str(fragment);
    }
    Ok(result)
}
