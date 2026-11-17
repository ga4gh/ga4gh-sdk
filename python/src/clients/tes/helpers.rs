//! Helpers shared by the TES classes: errors for responses that don't fit the TES models.
//! They don't define Python classes.

use crate::utils::json;
use pyo3::prelude::*;

/// The message for a TES response that doesn't fit its models.
pub(super) fn invalid_response_text(error: impl std::fmt::Display) -> String {
    json::invalid_response_text("TES", error)
}

/// A `RuntimeError` for a TES response that doesn't fit its models.
pub(super) fn invalid_response(error: impl std::fmt::Display) -> PyErr {
    json::invalid_response("TES", error)
}
