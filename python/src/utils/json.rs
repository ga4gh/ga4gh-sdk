//! Reading the JSON that result classes keep, parsing response bodies, and errors for
//! responses that don't fit.

use crate::utils::convert::to_python;
use pyo3::exceptions::PyRuntimeError;
use pyo3::prelude::*;
use serde_json::Value;

/// A field of a JSON object as a Python object, or `None` if it is missing or `null`.
pub fn field(py: Python<'_>, raw: &Value, key: &str) -> PyResult<Option<Py<PyAny>>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => to_python(py, value).map(Some),
    }
}

/// The message for a response of `api` that doesn't fit its models.
pub fn invalid_response_text(api: &str, error: impl std::fmt::Display) -> String {
    format!("Invalid {api} response: {error}")
}

/// A `RuntimeError` for a response of `api` that doesn't fit its models.
pub fn invalid_response(api: &str, error: impl std::fmt::Display) -> PyErr {
    PyRuntimeError::new_err(invalid_response_text(api, error))
}

/// Parses the JSON text of a response of `api`. Errors are `RuntimeError`s.
pub fn parse_body(api: &str, body: &str) -> PyResult<Value> {
    serde_json::from_str(body).map_err(|error| invalid_response(api, error))
}

/// Adds the read-only `inner()` and `raw()` accessors to a result class that keeps its JSON
/// in `raw` and a model of it in `inner`.
///
/// The fields are deprecated, so that extension crates can't change one without the other;
/// the accessors are the only code that reads them directly. See
/// `docs/state_ownership.md` ("Python views").
macro_rules! view_accessors {
    ($view:ty, $model:ty) => {
        impl $view {
            /// The model of [`raw()`](Self::raw); it describes the same JSON value.
            ///
            /// The class's typed getters read it. To change a value, change a copy of
            /// `raw()` and build a new instance from it.
            #[allow(deprecated)] // The accessor that replaces the field.
            pub fn inner(&self) -> &$model {
                &self.inner
            }

            /// The JSON that the class describes: the value that its getters, `json()` and
            /// `to_dict()` return, including fields that the models don't know.
            #[allow(deprecated)] // The accessor that replaces the field.
            pub fn raw(&self) -> &serde_json::Value {
                &self.raw
            }
        }
    };
}
pub(crate) use view_accessors;

/// Test helpers for the result classes.
///
/// The default `extension-module` feature doesn't link libpython, so `cargo test` can't run
/// code that builds a Python exception. The tests build the classes with the constructors
/// that return `Result<_, String>` (`parse`, `build`), which the public constructors share.
#[cfg(test)]
pub(crate) mod tests {
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use std::fmt::Debug;

    /// Checks that a result class's `inner()`, `raw()` and `json()` describe `expected`.
    pub(crate) fn assert_describes<M: DeserializeOwned + PartialEq + Debug>(
        inner: &M,
        raw: &Value,
        json: &str,
        expected: &Value,
    ) {
        assert_eq!(raw, expected);
        assert_eq!(&serde_json::from_str::<Value>(json).unwrap(), expected);
        assert_eq!(
            inner,
            &serde_json::from_value::<M>(expected.clone()).unwrap()
        );
    }
}
