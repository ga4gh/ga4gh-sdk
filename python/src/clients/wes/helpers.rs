//! Helpers shared by the WES classes: parsing responses and sending GET requests. They
//! don't define Python classes.

use crate::utils::json;
use crate::utils::runtime::io;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::prelude::*;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A `RuntimeError` for a WES response that doesn't fit its models.
pub(super) fn invalid_response(error: impl std::fmt::Display) -> PyErr {
    json::invalid_response("WES", error)
}

/// Parses a server's JSON object into a WES model. Errors are `RuntimeError`s.
pub(super) fn parse_response<T: DeserializeOwned>(raw: &Value) -> PyResult<T> {
    if !raw.is_object() {
        return Err(invalid_response("not a JSON object"));
    }
    serde_json::from_value(raw.clone()).map_err(invalid_response)
}

/// The items of a JSON array field, or none if the field is missing.
pub(super) fn items(raw: &Value, key: &str) -> Vec<Value> {
    match raw.get(key) {
        Some(Value::Array(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// Sends a GET request and parses the response as JSON.
pub(super) fn get_json(
    py: Python<'_>,
    transport: &Transport,
    endpoint: &str,
    what: &str,
) -> PyResult<Value> {
    let body = io(py, |rt| {
        rt.block_on(transport.get(endpoint, None))
            .map_err(|error| format!("Failed to {what}: {error}"))
    })?;
    json::parse_body("WES", &body)
}
