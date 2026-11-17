//! Argument and model helpers shared by the TRS binding; no Python classes.

use crate::utils::json;
use crate::utils::validate::require_id;
use pyo3::exceptions::{PyOverflowError, PyValueError};
use pyo3::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// A model as JSON. The models serialize their unknown fields too.
pub(super) fn to_raw(value: &impl Serialize) -> PyResult<Value> {
    serde_json::to_value(value).map_err(|error| json::invalid_response("TRS", error))
}

/// A descriptor type, such as `CWL` (`DescriptorType`) or `PLAIN_CWL`
/// (`DescriptorTypeWithPlain`), as given: names that TRS doesn't define are sent unchanged.
pub(super) fn parse_descriptor_type<T: DeserializeOwned>(name: &str) -> PyResult<T> {
    require_id(name, "descriptor type")?;
    serde_json::from_value(Value::String(name.to_owned()))
        .map_err(|error| PyValueError::new_err(format!("Invalid descriptor type: {error}")))
}

/// An `int` argument between `min` and `max`. Other `int`s, also those too large for an
/// `i64`, raise `ValueError` with `message`; other types raise `TypeError`.
pub(super) fn bounded_int(
    value: &Bound<'_, PyAny>,
    min: i64,
    max: i64,
    message: &str,
) -> PyResult<i64> {
    match value.extract::<i64>() {
        Ok(number) if (min..=max).contains(&number) => Ok(number),
        Ok(_) => Err(PyValueError::new_err(message.to_owned())),
        Err(error) if error.is_instance_of::<PyOverflowError>(value.py()) => {
            Err(PyValueError::new_err(message.to_owned()))
        }
        Err(error) => Err(error),
    }
}

/// Checks a tool ID and a version ID with [`require_id`].
///
/// The Rust client rejects such IDs too, but as a request failure; this raises
/// `ValueError` before any request.
pub(super) fn require_ids(id: &str, version_id: &str) -> PyResult<()> {
    require_id(id, "tool ID")?;
    require_id(version_id, "version ID")
}
