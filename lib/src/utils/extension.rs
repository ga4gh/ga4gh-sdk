//! Versioned extension metadata and explicit C ABI loading.
pub(crate) mod runtime;
pub use runtime::{ExtensionInstance, ExtensionMethod, LoadedModule};

use crate::utils::json_store::JsonStore;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// This build's full target triple, including its platform ABI.
pub const HOST_TARGET: &str = env!("GA4GH_EXTENSION_TARGET");
/// Capability name to explicit supported interface revisions.
pub type Capabilities = BTreeMap<String, Vec<u32>>;
/// Negotiated capability name to one interface revision.
pub type NegotiatedCapabilities = BTreeMap<String, u32>;

/// Machine-readable failure classification; default diagnostics contain no payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ExtensionErrorKind {
    /// Invalid name, version requirement, or metadata.
    InvalidMetadata,
    /// Unsupported or legacy manifest/registry format; convert or reinstall explicitly.
    UnsupportedManifest,
    /// Missing extension registration or compatible release.
    Missing,
    /// No installed artifact matches an explicit release or digest request.
    NoMatchingArtifact,
    /// Extension is registered but disabled.
    Disabled,
    /// Unsupported native ABI.
    UnsupportedAbi,
    /// OS, architecture, or platform ABI does not match.
    TargetMismatch,
    /// Library bytes differ from the recorded digest.
    DigestMismatch,
    /// A required capability or interface revision is unavailable.
    MissingCapability,
    /// Multiple matching artifacts require an exact digest selection.
    AmbiguousArtifact,
    /// An earlier resolution is no longer registered or belongs to another registry.
    StaleResolution,
    /// The process has already loaded another artifact of this extension.
    RestartRequired,
    /// OS library loading or symbol lookup failed.
    LoadFailed,
    /// Exported API is malformed or disagrees with the manifest.
    InvalidApi,
    /// Plugin instance creation failed.
    InitializationFailed,
    /// Plugin operation failed.
    InvocationFailed,
    /// Registry, installation, or coordination I/O failed.
    Storage,
}
/// Inspectable failure with redacted default formatting and explicit private details.
pub struct ExtensionError {
    /// Failure classification.
    pub kind: ExtensionErrorKind,
    /// Loaded artifact when a restart is required.
    pub current: Option<Box<ArtifactIdentity>>,
    /// Requested artifact when a restart is required.
    pub requested: Option<Box<ArtifactIdentity>>,
    /// Plugin status, when applicable.
    pub status: Option<i32>,
    payload: Vec<u8>,
    cause: Option<crate::Error>,
}
impl ExtensionError {
    pub(crate) fn new(kind: ExtensionErrorKind) -> Self {
        Self {
            kind,
            current: None,
            requested: None,
            status: None,
            payload: vec![],
            cause: None,
        }
    }
    pub(crate) fn caused(kind: ExtensionErrorKind, cause: impl Into<crate::Error>) -> Self {
        let mut e = Self::new(kind);
        e.cause = Some(cause.into());
        e
    }
    pub(crate) fn plugin(kind: ExtensionErrorKind, status: i32, payload: Vec<u8>) -> Self {
        let mut e = Self::new(kind);
        e.status = Some(status);
        e.payload = payload;
        e
    }
    /// Explicit access to plugin error bytes, which may include secrets.
    pub fn private_payload(&self) -> &[u8] {
        &self.payload
    }
}
impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Extension error: {:?}", self.kind)
    }
}
impl fmt::Debug for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionError")
            .field("kind", &self.kind)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}
impl std::error::Error for ExtensionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.cause.as_ref().map(|x| x as _)
    }
}
/// Result of extension management, resolution, and execution.
pub type ExtensionResult<T> = Result<T, ExtensionError>;

/// Exact binary identity. A release number alone cannot identify a loaded artifact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactIdentity {
    /// Logical extension identity, shared across registries in one process.
    pub name: String,
    /// Extension release version.
    pub version: semver::Version,
    /// Full target triple.
    pub target: String,
    /// Lowercase SHA-512 hex digest (the v1 digest algorithm).
    pub digest: String,
    /// Native ABI revision.
    pub abi_version: u32,
}
/// Installation manifest. Reading it never executes native code.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Extension {
    /// Manifest schema revision; v1 accepts 1 only.
    pub manifest_version: u32,
    /// Exact binary identity; fields are flattened into JSON.
    #[serde(flatten)]
    pub artifact: ArtifactIdentity,
    /// Advertised capability revisions.
    pub capabilities: Capabilities,
    /// HTTP(S) URL or local path relative to this manifest.
    pub path: String,
    /// Optional descriptive metadata.
    #[serde(default)]
    pub description: Option<String>,
}
/// Exported descriptor, encoded as UTF-8 JSON in the C API table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExtensionDescriptor {
    /// Logical extension name.
    pub name: String,
    /// Extension release version.
    pub version: semver::Version,
    /// Full target triple.
    pub target: String,
    /// Native ABI revision.
    pub abi_version: u32,
    /// Advertised capability revisions.
    pub capabilities: Capabilities,
}
impl Extension {
    /// Reads and validates metadata without loading code. Legacy manifests need reinstalling.
    pub fn from_file(path: impl AsRef<Path>) -> ExtensionResult<Self> {
        let value: serde_json::Value = JsonStore::new(path.as_ref(), "Extension manifest")
            .read()
            .map_err(storage)?
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
        if value.get("manifest_version").and_then(|v| v.as_u64()) != Some(1) {
            return Err(ExtensionError::new(ExtensionErrorKind::UnsupportedManifest));
        }
        let manifest: Self = serde_json::from_value(value)
            .map_err(|_| ExtensionError::new(ExtensionErrorKind::InvalidMetadata))?;
        manifest.validate()?;
        Ok(manifest)
    }
    pub(crate) fn validate(&self) -> ExtensionResult<()> {
        if self.manifest_version != 1 {
            return Err(ExtensionError::new(ExtensionErrorKind::UnsupportedManifest));
        }
        validate_name(&self.artifact.name)?;
        if self.artifact.digest.len() != 128
            || !self
                .artifact
                .digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || self.artifact.target.is_empty()
        {
            return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
        }
        for (name, revisions) in &self.capabilities {
            validate_name(name)?;
            let unique: std::collections::BTreeSet<_> = revisions.iter().collect();
            if revisions.is_empty() || revisions.contains(&0) || unique.len() != revisions.len() {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
        }
        Ok(())
    }
    pub(crate) fn compatible(&self) -> ExtensionResult<()> {
        if self.artifact.abi_version != ga4gh_extension_abi::ABI_VERSION {
            return Err(ExtensionError::new(ExtensionErrorKind::UnsupportedAbi));
        }
        if self.artifact.target != HOST_TARGET {
            return Err(ExtensionError::new(ExtensionErrorKind::TargetMismatch));
        }
        Ok(())
    }
    pub(crate) fn descriptor(&self) -> ExtensionDescriptor {
        let mut capabilities = self.capabilities.clone();
        for revisions in capabilities.values_mut() {
            revisions.sort_unstable();
        }
        ExtensionDescriptor {
            name: self.artifact.name.clone(),
            version: self.artifact.version.clone(),
            target: self.artifact.target.clone(),
            abi_version: self.artifact.abi_version,
            capabilities,
        }
    }
}
/// One immutable artifact installation. Contains no runtime state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledArtifact {
    /// Verified installation metadata.
    pub manifest: Extension,
    /// Managed library path.
    pub library_path: PathBuf,
}
/// Logical registration, with an explicit selected digest and multiple artifacts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledExtension {
    /// Logical extension name.
    pub name: String,
    /// Whether new instances may be admitted.
    pub enabled: bool,
    /// Selected artifact digest.
    pub selected: String,
    /// All registered artifacts of this extension.
    pub artifacts: Vec<InstalledArtifact>,
}
/// Exact, immutable resolver output; only the manager can construct it.
#[derive(Debug, Clone)]
pub struct ResolvedExtension {
    pub(crate) installation: InstalledArtifact,
    pub(crate) capabilities: NegotiatedCapabilities,
    pub(crate) registry: PathBuf,
}
impl ResolvedExtension {
    /// Exact name, release, target, digest, and ABI.
    pub fn artifact(&self) -> &ArtifactIdentity {
        &self.installation.manifest.artifact
    }
    /// Agreed interface revisions for requested capabilities.
    pub fn capabilities(&self) -> &NegotiatedCapabilities {
        &self.capabilities
    }
    /// Concrete managed library path.
    pub fn library_path(&self) -> &Path {
        &self.installation.library_path
    }
}
pub(crate) fn validate_name(name: &str) -> ExtensionResult<()> {
    if matches!(name, "" | "." | "..")
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
    }
    Ok(())
}
pub(crate) fn storage(e: crate::Error) -> ExtensionError {
    ExtensionError::caused(ExtensionErrorKind::Storage, e)
}
pub(crate) fn io(e: std::io::Error) -> ExtensionError {
    ExtensionError::caused(ExtensionErrorKind::Storage, e)
}
