//! Versioned installations, immutable resolution, and fresh registry admission.
//!
//! Private modules own installation, registry validation, and artifact selection.
//! This module coordinates public operations and the registry admission lock.
use crate::utils::{
    configuration::{Configuration, ServiceExtensionsConfiguration},
    extension::{
        self, Capabilities, ExtensionError, ExtensionErrorKind, ExtensionInstance, ExtensionResult,
        InstalledExtension, LoadedModule, ResolvedExtension,
    },
    json_store::{JsonStore, JsonStoreIoError},
};
use registry::Registry;

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

mod installation;
mod registry;
mod resolution;

/// Metadata snapshot and caller-owned paths. Runtime ownership lives in instances.
/// Every mutation and instance admission reads the current registry under its lock.
#[derive(Debug, Clone)]
pub struct ExtensionManager {
    registry: PathBuf,
    root: PathBuf,
    snapshot: Registry,
}
// Resolve existing components, including symlinks, without creating missing paths.
fn canonicalize_path(path: &Path) -> ExtensionResult<PathBuf> {
    let absolute = std::path::absolute(path).map_err(extension::io)?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        if component == std::path::Component::ParentDir {
            resolved.pop();
            continue;
        }
        resolved.push(component);
        match fs::canonicalize(&resolved) {
            Ok(path) => resolved = path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // A dangling link must not later be replaced by an atomic write.
                if fs::symlink_metadata(&resolved).is_ok() {
                    return Err(extension::io(e));
                }
            }
            Err(e) => return Err(extension::io(e)),
        }
    }
    Ok(resolved)
}

impl ExtensionManager {
    /// Opens metadata with installations beside the registry in `extensions/`.
    /// Use `with_paths` to control the two paths independently. Does not create files.
    pub fn open(registry: impl AsRef<Path>) -> ExtensionResult<Self> {
        let registry = canonicalize_path(registry.as_ref())?;
        let root = registry
            .parent()
            .unwrap_or(Path::new("."))
            .join("extensions");
        Self::with_paths(registry, root)
    }

    /// Opens metadata using explicit registry and installation-root paths.
    pub fn with_paths(registry: impl AsRef<Path>, root: impl AsRef<Path>) -> ExtensionResult<Self> {
        let registry = canonicalize_path(registry.as_ref())?;
        let root = canonicalize_path(root.as_ref())?;
        let snapshot = Registry::read(&registry)?;
        Ok(Self {
            registry,
            root,
            snapshot,
        })
    }

    fn store(&self) -> JsonStore {
        JsonStore::new(&self.registry, "Extension registry")
    }

    fn mutate(
        &mut self,
        change: impl FnOnce(&mut Registry) -> ExtensionResult<()>,
    ) -> ExtensionResult<()> {
        let store = self.store();
        let _lock = store.lock().map_err(extension::storage)?;
        let mut latest = Registry::read(&self.registry)?;
        change(&mut latest)?;
        match store.write(&latest) {
            Ok(()) => {
                self.snapshot = latest;
                Ok(())
            }
            Err(e) => {
                if e.downcast_ref::<JsonStoreIoError>()
                    .is_some_and(JsonStoreIoError::replacement_completed)
                {
                    self.snapshot = latest;
                }
                Err(extension::storage(e))
            }
        }
    }

    /// Current in-memory metadata snapshot; `refresh` observes later changes.
    pub fn get_extensions(&self) -> &[InstalledExtension] {
        &self.snapshot.extensions
    }

    /// Reloads metadata without executing native code.
    pub fn refresh(&mut self) -> ExtensionResult<()> {
        self.snapshot = Registry::read(&self.registry)?;
        Ok(())
    }

    /// Whether this process currently retains a validated module for this name.
    pub fn is_loaded(&self, name: &str) -> bool {
        extension::runtime::is_loaded(name)
    }

    /// Installs local or HTTP(S) bytes into immutable storage and selects them.
    /// Existing registrations keep enablement; a first installation is disabled.
    /// Never executes native code. Unregistered files are retained on write failure.
    /// The caller must approve the manifest's source path or network destination.
    /// Private repositories are supported; redirects are rejected.
    pub async fn add_extension(&mut self, path: impl AsRef<Path>) -> ExtensionResult<()> {
        let installation = installation::install(self, path.as_ref()).await?;
        self.mutate(|registry| registry.register(installation))
    }

    fn check_root(&self, name: &str) -> ExtensionResult<()> {
        extension::validate_name(name)?;
        if self.root.is_symlink() || self.root.join(name).is_symlink() {
            return Err(ExtensionError::new(ExtensionErrorKind::Storage));
        }
        Ok(())
    }

    /// Unregisters an extension. Artifacts remain on disk and active handles work.
    pub fn remove_extension(&mut self, name: &str) -> ExtensionResult<()> {
        extension::validate_name(name)?;
        self.mutate(|registry| registry.remove(name))
    }

    fn set_enabled(&mut self, name: &str, enabled: bool) -> ExtensionResult<()> {
        extension::validate_name(name)?;
        self.mutate(|registry| registry.set_enabled(name, enabled))
    }

    /// Enables future admissions without loading code.
    pub fn enable_extension(&mut self, name: &str) -> ExtensionResult<()> {
        self.set_enabled(name, true)
    }

    /// Blocks future admissions; existing instances remain valid.
    pub fn disable_extension(&mut self, name: &str) -> ExtensionResult<()> {
        self.set_enabled(name, false)
    }

    /// Atomically selects an already installed digest without loading code.
    pub fn select_extension(&mut self, name: &str, digest: &str) -> ExtensionResult<()> {
        extension::validate_name(name)?;
        self.mutate(|registry| registry.select(name, digest))
    }

    /// Resolves fresh metadata without executing code. Without a requirement, uses the
    /// selected digest; an explicit version requirement chooses the highest compatible
    /// release. Required capabilities map names to acceptable positive revisions.
    pub fn resolve(
        &self,
        name: &str,
        version: Option<&str>,
        digest: Option<&str>,
        required: &Capabilities,
    ) -> ExtensionResult<ResolvedExtension> {
        extension::validate_name(name)?;
        let requirement = version
            .map(semver::VersionReq::parse)
            .transpose()
            .map_err(|_| ExtensionError::new(ExtensionErrorKind::InvalidMetadata))?;
        for (name, revisions) in required {
            extension::validate_name(name)?;
            if revisions.is_empty() || revisions.contains(&0) {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
        }
        let registry = Registry::read(&self.registry)?;
        let record = registry
            .extensions
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
        if !record.enabled {
            return Err(ExtensionError::new(ExtensionErrorKind::Disabled));
        }
        let (installation, capabilities) =
            resolution::select_artifact(record, requirement.as_ref(), digest, required)?;
        Ok(ResolvedExtension {
            installation,
            capabilities,
            registry: self.registry.clone(),
        })
    }

    fn admit(&self, resolved: &ResolvedExtension) -> ExtensionResult<ResolvedExtension> {
        let store = self.store();
        let _lock = store.lock().map_err(extension::storage)?;
        let registry = Registry::read(&self.registry)?;
        let record = registry
            .extensions
            .iter()
            .find(|e| e.name == resolved.artifact().name)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
        if !record.enabled {
            return Err(ExtensionError::new(ExtensionErrorKind::Disabled));
        }
        if resolved.registry != self.registry || !record.artifacts.contains(&resolved.installation)
        {
            return Err(ExtensionError::new(ExtensionErrorKind::StaleResolution));
        }
        // This owned permit fixes admission. The file lock drops on return, before
        // library constructors, API queries, instance creation, or buffer release.
        Ok(resolved.clone())
    }

    /// Loads an admitted artifact without creating an instance. Existing module
    /// handles cannot create instances without another admission through the manager.
    ///
    /// # Safety
    /// The artifact is trusted native code implementing the C ABI v1 header's
    /// pointer, allocation, lifetime, no-unwind, and cross-thread requirements.
    pub unsafe fn load(&self, resolved: &ResolvedExtension) -> ExtensionResult<Arc<LoadedModule>> {
        let permit = self.admit(resolved)?;
        unsafe { extension::runtime::load(&permit) }
    }

    /// Admits and creates one instance from the exact resolution and opaque config.
    ///
    /// # Safety
    /// The artifact must satisfy the trust and ABI contract of `Self::load`.
    pub unsafe fn create_instance(
        &self,
        resolved: &ResolvedExtension,
        configuration: &[u8],
    ) -> ExtensionResult<ExtensionInstance> {
        let permit = self.admit(resolved)?;
        let module = unsafe { extension::runtime::load(&permit) }?;
        ExtensionInstance::create(module, permit.capabilities, configuration)
    }

    /// Resolves and creates configured instances. Only each entry's JSON config
    /// is forwarded. Optional missing/disabled entries are skipped, other failures
    /// propagate. Service credentials never reach plugins.
    ///
    /// # Safety
    /// Every selected artifact must satisfy `Self::load`'s native trust contract.
    pub unsafe fn from_configuration(
        &self,
        config: &Configuration,
    ) -> ExtensionResult<Vec<ExtensionInstance>> {
        let selected = config
            .extra_as::<ServiceExtensionsConfiguration>("extensions")
            .map_err(extension::storage)?
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let mut instances = vec![];
        for entry in selected.entries() {
            if !seen.insert(&entry.extension_name) {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
            let Some(resolved) = optional_entry(
                entry.required,
                self.resolve(
                    &entry.extension_name,
                    entry.version.as_deref(),
                    None,
                    &entry.capabilities,
                ),
            )?
            else {
                continue;
            };
            let config = serde_json::to_vec(&entry.configuration)
                .map_err(|_| ExtensionError::new(ExtensionErrorKind::InvalidMetadata))?;
            if let Some(instance) = optional_entry(entry.required, unsafe {
                self.create_instance(&resolved, &config)
            })? {
                instances.push(instance);
            }
        }
        Ok(instances)
    }
}

// Apply the same optional-entry policy at resolution and at fresh admission.
fn optional_entry<T>(required: bool, result: ExtensionResult<T>) -> ExtensionResult<Option<T>> {
    match result {
        Err(e)
            if !required
                && matches!(
                    e.kind,
                    ExtensionErrorKind::Missing | ExtensionErrorKind::Disabled
                ) =>
        {
            Ok(None)
        }
        other => other.map(Some),
    }
}

#[cfg(test)]
mod tests;
