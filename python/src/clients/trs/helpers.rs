//! Argument and model helpers shared by the TRS binding; no Python classes.

use crate::utils::json;
use crate::utils::validate::require_id;
use ga4gh_sdk::clients::trs::TrsVersion;
use ga4gh_sdk::compatibility::{
    CapabilityOverride, CapabilitySupport, SelectionSource, VersionPolicy,
};
use pyo3::exceptions::{PyOverflowError, PyValueError};
use pyo3::prelude::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

/// A model as JSON. The models serialize their unknown fields too.
pub(super) fn to_raw(value: &impl Serialize) -> PyResult<Value> {
    serde_json::to_value(value).map_err(|error| json::invalid_response("TRS", error))
}

/// Rejects IDs and paths that can't be one URL path segment: empty, `.` or `..`.
///
/// The Rust client rejects them too, but as a request failure; this raises `ValueError`
/// before any request. Other values are sent unchanged, as one percent-encoded segment.
pub(super) fn require_segment(value: &str, what: &str) -> PyResult<()> {
    require_id(value, what)
}

/// A descriptor type, such as `CWL` (`DescriptorType`) or `PLAIN_CWL`
/// (`DescriptorTypeWithPlain`), as given: names that TRS doesn't define are sent unchanged.
pub(super) fn parse_descriptor_type<T: DeserializeOwned>(name: &str) -> PyResult<T> {
    require_segment(name, "descriptor type")?;
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

/// Checks a tool ID and a version ID, see [`require_segment`].
pub(super) fn require_ids(id: &str, version_id: &str) -> PyResult<()> {
    require_segment(id, "tool ID")?;
    require_segment(version_id, "version ID")
}

pub(super) fn version_policy(api_version: &str) -> PyResult<VersionPolicy<TrsVersion>> {
    match api_version {
        "auto" => Ok(VersionPolicy::Auto),
        "strict" => Ok(VersionPolicy::RequireKnown),
        "2.0.1" => Ok(VersionPolicy::Explicit(TrsVersion::V2_0_1)),
        "2.0.2" => Ok(VersionPolicy::Explicit(TrsVersion::V2_0_2)),
        other => Err(PyValueError::new_err(format!(
            "Invalid api_version {other:?}: use 'auto', 'strict', '2.0.1' or '2.0.2'"
        ))),
    }
}

pub(super) fn capability_override(tag_filter: &str) -> PyResult<CapabilityOverride> {
    match tag_filter {
        "auto" => Ok(CapabilityOverride::Auto),
        "enabled" => Ok(CapabilityOverride::Enabled),
        "disabled" => Ok(CapabilityOverride::Disabled),
        other => Err(PyValueError::new_err(format!(
            "Invalid tag_filter {other:?}: use 'auto', 'enabled' or 'disabled'"
        ))),
    }
}

pub(super) fn source_name(source: SelectionSource) -> &'static str {
    match source {
        SelectionSource::DeclaredVersion => "declared",
        SelectionSource::Caller => "caller",
        SelectionSource::Baseline => "baseline",
        _ => "unknown",
    }
}

pub(super) fn support_name(support: CapabilitySupport) -> &'static str {
    match support {
        CapabilitySupport::Specified => "specified",
        CapabilitySupport::NotSpecified => "not_specified",
        CapabilitySupport::CallerEnabled => "caller_enabled",
        CapabilitySupport::CallerDisabled => "caller_disabled",
        _ => "unknown",
    }
}
