//! Version and capability selection of the TRS client: the `api_version` and `tag_filter`
//! arguments, how the client connects with them, and the names of the selection's results.

use crate::utils::runtime::io;
use crate::PyConfiguration;
use ga4gh_sdk::clients::trs::{TrsOptions, TrsVersion, TRS};
use ga4gh_sdk::compatibility::{
    CapabilityOverride, CapabilitySupport, SelectionSource, VersionPolicy,
};
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// Builds the Rust client with `options`; with `discovery`, it reads the service info first,
/// releasing the GIL. Without discovery, `options.version` must be explicit.
pub(super) fn connect(
    py: Python<'_>,
    config: &PyConfiguration,
    options: TrsOptions,
    discovery: bool,
) -> PyResult<TRS> {
    if !discovery {
        let VersionPolicy::Explicit(version) = options.version else {
            return Err(PyValueError::new_err(
                "discovery=False needs an explicit api_version, '2.0.1' or '2.0.2'",
            ));
        };
        return Ok(
            TRS::without_discovery(Transport::new(&config.inner), version)
                .with_tag_filter_override(options.tag_filter),
        );
    }
    io(py, |rt| {
        rt.block_on(TRS::with_options(&config.inner, options))
            .map_err(|error| format!("Failed to create TRS instance: {error}"))
    })
}

/// Checks that the client may send the `tags` filter, if `tags` isn't empty.
pub(super) fn filter_tags(trs: &TRS, tags: &[String]) -> PyResult<()> {
    if tags.is_empty() {
        return Ok(());
    }
    trs.compatibility()
        .tag_filter()
        .require("TRS", "tags")
        .map_err(|error| PyValueError::new_err(error.to_string()))
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
