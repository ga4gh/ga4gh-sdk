//! Shared policy for clients that support several protocol contracts.
//!
//! Each service supplies its supported versions and a deliberate fallback.
//! Selection does not negotiate with the server, validate a response, or retry
//! a request. Currently TRS uses this mechanism; other APIs retain their own
//! behavior until they adopt it. A fallback must not silently remove requested
//! semantics or enable operations that the selected contract cannot represent.

use std::{error::Error, fmt};

/// How to select a service's protocol profile.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum VersionPolicy<P> {
    /// Use a recognized declaration, otherwise the service's documented baseline.
    #[default]
    Auto,
    /// Reject absent or unrecognized declarations instead of using a baseline.
    RequireKnown,
    /// Interpret responses using this profile, retaining the server declaration.
    Explicit(P),
}

/// Evidence used to select the local protocol profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectionSource {
    /// The declaration matched an explicitly supported version.
    DeclaredVersion,
    /// The caller selected the profile.
    Caller,
    /// The declaration was absent or unknown; compatibility is unverified.
    Baseline,
}

/// Immutable selection provenance owned by one client connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionSelection<P> {
    declared: Option<String>,
    profile: P,
    source: SelectionSource,
}

impl<P: Copy> VersionSelection<P> {
    /// Select a profile without discovery. The server identity is not verified.
    pub fn explicit(profile: P) -> Self {
        Self {
            declared: None,
            profile,
            source: SelectionSource::Caller,
        }
    }
    /// Resolve a profile against exact advertised version strings.
    ///
    /// The service must document which operations may use `baseline`. This
    /// method never infers compatibility from a version number's ordering.
    pub fn resolve(
        api: &'static str,
        declared: Option<&str>,
        policy: VersionPolicy<P>,
        supported: &[(&str, P)],
        baseline: P,
    ) -> Result<Self, CompatibilityError> {
        let (profile, source) = match policy {
            VersionPolicy::Explicit(profile) => (profile, SelectionSource::Caller),
            VersionPolicy::Auto | VersionPolicy::RequireKnown => {
                match supported
                    .iter()
                    .find(|(version, _)| Some(*version) == declared)
                {
                    Some((_, profile)) => (*profile, SelectionSource::DeclaredVersion),
                    None if matches!(policy, VersionPolicy::Auto) => {
                        (baseline, SelectionSource::Baseline)
                    }
                    None => {
                        return Err(CompatibilityError::UnknownVersion {
                            api,
                            declared: declared.map(str::to_owned),
                        })
                    }
                }
            }
        };
        Ok(Self {
            declared: declared.map(str::to_owned),
            profile,
            source,
        })
    }

    /// Version declared by the server; an override does not rewrite it.
    pub fn declared_version(&self) -> Option<&str> {
        self.declared.as_deref()
    }
    /// Local interpretation selected for this connection.
    pub fn profile(&self) -> P {
        self.profile
    }
    /// Whether the choice came from discovery, the caller, or a fallback.
    pub fn source(&self) -> SelectionSource {
        self.source
    }
}

/// Caller knowledge of a particular deployment's capability.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilityOverride {
    /// Use the selected specification's capability contract.
    #[default]
    Auto,
    /// The caller has established support, including a provider backport.
    Enabled,
    /// The deployment does not implement this capability.
    Disabled,
}

/// The reason an operation's capability is, or is not, available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CapabilitySupport {
    /// Defined by the selected contract; actual server conformance is not proven.
    Specified,
    /// The selected contract does not define it; a provider may still implement it.
    NotSpecified,
    /// The caller explicitly enabled it for this deployment.
    CallerEnabled,
    /// The caller explicitly disabled it for this deployment.
    CallerDisabled,
    /// No support evidence is available.
    Unknown,
}

impl CapabilitySupport {
    /// Apply caller knowledge without changing the selected protocol version.
    pub fn with_override(self, override_: CapabilityOverride) -> Self {
        match override_ {
            CapabilityOverride::Auto => self,
            CapabilityOverride::Enabled => Self::CallerEnabled,
            CapabilityOverride::Disabled => Self::CallerDisabled,
        }
    }

    /// Reject a requested capability before sending a request if support is absent
    /// or unestablished. Callers may supply an explicit deployment override.
    pub fn require(
        self,
        api: &'static str,
        capability: &'static str,
    ) -> Result<(), CompatibilityError> {
        match self {
            Self::Specified | Self::CallerEnabled => Ok(()),
            _ => Err(CompatibilityError::Capability {
                api,
                capability,
                support: self,
            }),
        }
    }
}

/// An inspectable version or capability selection failure.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompatibilityError {
    /// Discovery did not identify a supported version under a strict policy.
    UnknownVersion {
        api: &'static str,
        declared: Option<String>,
    },
    /// The requested feature is unavailable or its support is unknown.
    Capability {
        api: &'static str,
        capability: &'static str,
        support: CapabilitySupport,
    },
}

impl fmt::Display for CompatibilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVersion { api, declared } => write!(f, "{api} API version {:?} is not recognized; select an explicit compatibility profile", declared.as_deref()),
            Self::Capability { api, capability, support } => write!(f, "{api} capability '{capability}' is not established ({support:?}); use an explicit capability override only for a deployment known to support it"),
        }
    }
}

impl Error for CompatibilityError {}
