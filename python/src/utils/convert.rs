//! Conversions between Python objects, JSON values and query strings.

use ga4gh_sdk::utils::query::encode_query;
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyString, PyTuple, PyType};
use pyo3::PyTypeInfo;
use serde_json::Value;
use std::ffi::CString;
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
    let object_init = cls.py().get_type::<PyAny>().getattr("__init__")?;
    if cls.is(&T::type_object(cls.py())) || cls.getattr("__init__")?.is(&object_init) {
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
    let bytes = std::fs::read(path).map_err(|error| os_error(py, error, path))?;
    String::from_utf8(bytes).map_err(|_| {
        PyValueError::new_err(format!(
            "Invalid file {}: not UTF-8 text",
            path.to_string_lossy()
        ))
    })
}

/// Converts an I/O error for `path` to the matching `OSError` subclass (such as
/// `FileNotFoundError`), with the system's message and the path.
pub fn os_error(py: Python<'_>, error: std::io::Error, path: &Path) -> PyErr {
    match error.raw_os_error() {
        Some(errno) => match py
            .import("os")
            .and_then(|os| os.call_method1("strerror", (errno,)))
            .and_then(|message| message.extract::<String>())
        {
            Ok(message) => {
                PyOSError::new_err((errno, message, path.to_string_lossy().into_owned()))
            }
            Err(error) => error,
        },
        None => PyErr::from(error),
    }
}

/// Defines a Python class, such as a `str` enum, from Python source.
///
/// The source runs in a namespace of its own whose `__name__` is `ga4gh_sdk`.
/// `PyModule::from_code_bound` would register a module in `sys.modules` and, under the
/// name `ga4gh_sdk`, overwrite the package's attributes.
pub fn define_class(py: Python<'_>, source: &str, name: &str) -> PyResult<Py<PyType>> {
    let namespace = PyDict::new(py);
    namespace.set_item("__name__", "ga4gh_sdk")?;
    let c_source =
        CString::new(source).map_err(|e| PyValueError::new_err(format!("Invalid source: {e}")))?;
    py.run(&c_source, Some(&namespace), None)?;
    Ok(namespace
        .get_item(name)?
        .ok_or_else(|| PyRuntimeError::new_err(format!("{name} is not defined")))?
        .downcast_into::<PyType>()?
        .unbind())
}

/// Converts a Python argument to JSON.
///
/// A `str` is parsed as JSON text. Any other object is serialised with Python's `json` module,
/// so `dict`, `list`, numbers, booleans and `None` work as expected.
pub fn to_json(value: &Bound<'_, PyAny>, what: &str) -> PyResult<Value> {
    let text = if let Ok(text) = value.downcast::<PyString>() {
        text.to_cow()?.into_owned()
    } else {
        let json = value.py().import("json")?;
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
    let json = py.import("json")?;
    Ok(json.call_method1("loads", (value.to_string(),))?.unbind())
}

/// Appends query parameters to an endpoint.
///
/// `params` must be a JSON object, encoded by the Rust transport's policy in
/// [`ga4gh_sdk::utils::query`]: strings, numbers and booleans give one parameter,
/// `null` none, and a list of strings, numbers and booleans one per item, as in
/// `tag_key=a&tag_key=b`. Other values, including `null` in a list, raise `ValueError`.
/// The parameters follow any query already in `endpoint`.
pub fn with_query(endpoint: &str, params: Option<Value>) -> PyResult<String> {
    let Some(params) = params else {
        return Ok(endpoint.to_owned());
    };
    let query = encode_query(&params)
        .map_err(|error| PyValueError::new_err(format!("Invalid params: {error}")))?;
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
