//! Conversions between Python objects, JSON values and query strings.

use pyo3::exceptions::{PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple, PyType};
use pyo3::PyTypeInfo;
use serde_json::Value;
use std::path::Path;

/// Lets subclasses take further constructor arguments.
///
/// Python passes all constructor arguments to `__new__` as well as `__init__`. So `__new__`
/// accepts and ignores arguments beyond its own when `cls` is a subclass that defines
/// `__init__(self, <base arguments>, extra, ...)` to take them. Otherwise, as for `T` itself,
/// they are an error.
pub fn check_extra_args<T: PyTypeInfo>(
    cls: &Bound<'_, PyType>,
    args: &Bound<'_, PyTuple>,
    kwargs: Option<&Bound<'_, PyDict>>,
) -> PyResult<()> {
    let has_extra = !args.is_empty() || kwargs.is_some_and(|kwargs| !kwargs.is_empty());
    if !has_extra {
        return Ok(());
    }
    let object_init = cls.py().get_type_bound::<PyAny>().getattr("__init__")?;
    if cls.is(&T::type_object_bound(cls.py())) || cls.getattr("__init__")?.is(&object_init) {
        return Err(PyTypeError::new_err(format!(
            "{}() got unexpected arguments",
            T::NAME
        )));
    }
    Ok(())
}

/// Reads a UTF-8 text file.
///
/// Failures to read raise the matching `OSError` subclass (such as `FileNotFoundError`) with
/// the system's message and the path; text that isn't UTF-8 raises `ValueError`.
pub fn read_file(py: Python<'_>, path: &Path) -> PyResult<String> {
    let filename = path.to_string_lossy().into_owned();
    let bytes = std::fs::read(path).map_err(|error| match error.raw_os_error() {
        Some(errno) => match py
            .import_bound("os")
            .and_then(|os| os.call_method1("strerror", (errno,)))
            .and_then(|message| message.extract::<String>())
        {
            Ok(message) => PyOSError::new_err((errno, message, filename.clone())),
            Err(error) => error,
        },
        None => PyErr::from(error),
    })?;
    String::from_utf8(bytes)
        .map_err(|_| PyValueError::new_err(format!("Invalid file {filename}: not UTF-8 text")))
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
