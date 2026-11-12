//! TRS profiles and deployment capability options.
use crate::compatibility::{
    CapabilityOverride, CapabilitySupport, SelectionSource, VersionPolicy, VersionSelection,
};
use std::fmt;

/// Wire contracts currently tested by the TRS client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TrsVersion {
    /// GA4GH TRS 2.0.1 response contract; tag filtering is not specified.
    V2_0_1,
    /// GA4GH TRS 2.0.2 response contract; tag filtering is specified.
    V2_0_2,
}

impl fmt::Display for TrsVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::V2_0_1 => "2.0.1",
            Self::V2_0_2 => "2.0.2",
        })
    }
}

/// Selection policy for a TRS connection. Auto uses the 2.0.1 read contract
/// for unknown declarations and reports that choice as an unverified baseline.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct TrsOptions {
    /// Select from discovery, require a recognized declaration, or override it.
    pub version: VersionPolicy<TrsVersion>,
    /// Override only with independent knowledge of a provider's tag filtering.
    pub tag_filter: CapabilityOverride,
}

impl TrsOptions {
    /// Select a version policy for this connection.
    pub fn with_version(mut self, version: VersionPolicy<TrsVersion>) -> Self {
        self.version = version;
        self
    }

    /// Set deployment-specific knowledge of tag-filter support.
    pub fn with_tag_filter(mut self, tag_filter: CapabilityOverride) -> Self {
        self.tag_filter = tag_filter;
        self
    }
}

/// Read-only compatibility state for one TRS client.
#[derive(Clone, Debug)]
pub struct TrsCompatibility {
    selection: VersionSelection<TrsVersion>,
    tag_filter: CapabilitySupport,
}

impl TrsCompatibility {
    pub(super) fn without_discovery(profile: TrsVersion) -> Self {
        Self::from_selection(
            VersionSelection::explicit(profile),
            CapabilityOverride::Auto,
        )
    }

    fn from_selection(selection: VersionSelection<TrsVersion>, tags: CapabilityOverride) -> Self {
        let mut result = Self {
            selection,
            tag_filter: CapabilitySupport::Unknown,
        };
        result.override_tags(tags);
        result
    }

    pub(super) fn select(
        declared: Option<&str>,
        options: TrsOptions,
    ) -> Result<Self, crate::compatibility::CompatibilityError> {
        let selection = VersionSelection::resolve(
            "TRS",
            declared,
            options.version,
            &[("2.0.1", TrsVersion::V2_0_1), ("2.0.2", TrsVersion::V2_0_2)],
            TrsVersion::V2_0_1,
        )?;
        Ok(Self::from_selection(selection, options.tag_filter))
    }

    /// Declared version, selected profile, and the source of selection.
    pub fn selection(&self) -> &VersionSelection<TrsVersion> {
        &self.selection
    }
    /// Support assumed from the selected contract or asserted by the caller.
    pub fn tag_filter(&self) -> CapabilitySupport {
        self.tag_filter
    }
    pub(super) fn override_tags(&mut self, value: CapabilityOverride) {
        let base = match (self.selection.source(), self.selection.profile()) {
            (SelectionSource::Baseline, _) => CapabilitySupport::Unknown,
            (_, TrsVersion::V2_0_1) => CapabilitySupport::NotSpecified,
            (_, TrsVersion::V2_0_2) => CapabilitySupport::Specified,
        };
        self.tag_filter = base.with_override(value);
    }
}
