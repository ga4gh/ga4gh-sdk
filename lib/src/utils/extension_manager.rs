//! Versioned installations, immutable resolution, and fresh registry admission.
use crate::utils::{
    configuration::{Configuration, ServiceExtensionsConfiguration},
    extension::{self, *},
    json_store::{JsonStore, JsonStoreIoError},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Registry {
    registry_version: u32,
    extensions: Vec<InstalledExtension>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            registry_version: 1,
            extensions: vec![],
        }
    }
}
/// Metadata snapshot and caller-owned paths. Runtime ownership lives in instances.
/// Every mutation and instance admission reads the current registry under its lock.
#[derive(Debug, Clone)]
pub struct ExtensionManager {
    registry: PathBuf,
    root: PathBuf,
    snapshot: Registry,
}
// Resolve existing components, including symlinks, without creating missing paths.
fn absolute(path: &Path) -> ExtensionResult<PathBuf> {
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
// JsonStore syncs a replaced file's immediate parent. Artifact publication also
// needs its renamed directory and ancestor entries persisted before registration.
fn sync_artifact_directories(directory: &Path) -> ExtensionResult<()> {
    #[cfg(unix)]
    for path in directory.ancestors() {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(extension::io)?;
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}
impl ExtensionManager {
    /// Opens metadata with installations beside the registry in `extensions/`.
    /// Use `with_paths` to control the two paths independently. Does not create files.
    pub fn open(registry: impl AsRef<Path>) -> ExtensionResult<Self> {
        let registry = absolute(registry.as_ref())?;
        let root = registry
            .parent()
            .unwrap_or(Path::new("."))
            .join("extensions");
        Self::with_paths(registry, root)
    }
    /// Opens metadata using explicit registry and installation-root paths.
    pub fn with_paths(registry: impl AsRef<Path>, root: impl AsRef<Path>) -> ExtensionResult<Self> {
        let registry = absolute(registry.as_ref())?;
        let root = absolute(root.as_ref())?;
        let snapshot = Self::read(&registry)?;
        Ok(Self {
            registry,
            root,
            snapshot,
        })
    }
    fn store(&self) -> JsonStore {
        JsonStore::new(&self.registry, "Extension registry")
    }
    fn read(path: &Path) -> ExtensionResult<Registry> {
        let Some(value) = JsonStore::new(path, "Extension registry")
            .read::<serde_json::Value>()
            .map_err(extension::storage)?
        else {
            return Ok(Registry::default());
        };
        if value.get("registry_version").and_then(|v| v.as_u64()) != Some(1) {
            return Err(ExtensionError::new(ExtensionErrorKind::UnsupportedManifest));
        }
        let registry: Registry = serde_json::from_value(value)
            .map_err(|_| ExtensionError::new(ExtensionErrorKind::InvalidMetadata))?;
        let mut names = std::collections::HashSet::new();
        for record in &registry.extensions {
            extension::validate_name(&record.name)?;
            if !names.insert(&record.name)
                || record.artifacts.is_empty()
                || !record
                    .artifacts
                    .iter()
                    .any(|a| a.manifest.artifact.digest == record.selected)
            {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
            let mut digests = std::collections::HashSet::new();
            for artifact in &record.artifacts {
                artifact.manifest.validate()?;
                if artifact.manifest.artifact.name != record.name
                    || !artifact.library_path.is_absolute()
                    || !digests.insert(&artifact.manifest.artifact.digest)
                {
                    return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
                }
            }
        }
        Ok(registry)
    }
    fn mutate(
        &mut self,
        change: impl FnOnce(&mut Registry) -> ExtensionResult<()>,
    ) -> ExtensionResult<()> {
        let store = self.store();
        let _lock = store.lock().map_err(extension::storage)?;
        let mut latest = Self::read(&self.registry)?;
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
        self.snapshot = Self::read(&self.registry)?;
        Ok(())
    }
    /// Whether this process currently retains a validated module for this name.
    pub fn is_loaded(&self, name: &str) -> bool {
        extension::runtime::is_loaded(name)
    }
    /// Installs local or HTTP(S) bytes into immutable storage and selects them.
    /// Existing registrations keep enablement; a first installation is disabled.
    /// Never executes native code. Unregistered files are retained on write failure.
    pub async fn add_extension(&mut self, path: impl AsRef<Path>) -> ExtensionResult<()> {
        let path = path.as_ref();
        let mut manifest = Extension::from_file(path)?;
        self.check_root(&manifest.artifact.name)?;
        fs::create_dir_all(&self.root).map_err(extension::io)?;
        let root = fs::canonicalize(&self.root).map_err(extension::io)?;
        let staging = tempfile::tempdir_in(&root).map_err(extension::io)?;
        let filename = format!("library{}", std::env::consts::DLL_SUFFIX);
        let prepared = staging.path().join(&filename);
        let mut file = fs::File::create(&prepared).map_err(extension::io)?;
        let parsed_url = url::Url::parse(&manifest.path).ok();
        let digest = if let Some(url) =
            parsed_url.filter(|url| matches!(url.scheme(), "http" | "https"))
        {
            if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
            let mut response = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?
                .get(url)
                .send()
                .await
                .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?
                .error_for_status()
                .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?;
            if !response.status().is_success() {
                return Err(ExtensionError::new(ExtensionErrorKind::Storage));
            }
            extension::artifact_size(response.content_length().unwrap_or(0))?;
            let mut digest = Sha512::new();
            let mut length = 0;
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?
            {
                length += chunk.len() as u64;
                extension::artifact_size(length)?;
                digest.update(&chunk);
                file.write_all(&chunk).map_err(extension::io)?;
            }
            extension::digest_hex(digest.finalize())
        } else {
            if manifest.path.contains("://") || manifest.path.is_empty() {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
            let source = path.parent().unwrap_or(Path::new(".")).join(&manifest.path);
            let source = fs::File::open(source).map_err(extension::io)?;
            extension::artifact_size(source.metadata().map_err(extension::io)?.len())?;
            extension::copy_artifact(source, &mut file)?
        };
        file.sync_all().map_err(extension::io)?;
        drop(file);
        if digest != manifest.artifact.digest {
            return Err(ExtensionError::new(ExtensionErrorKind::DigestMismatch));
        }
        // Installation sources may contain query tokens or private local paths.
        // Persist only the relative filename of the verified managed artifact.
        manifest.path = filename.clone();
        JsonStore::new(staging.path().join("manifest.json"), "Extension manifest")
            .write(&manifest)
            .map_err(extension::storage)?;
        let name = manifest.artifact.name.clone();
        let digest = manifest.artifact.digest.clone();
        let destination = root.join(&name).join(&digest);
        let installation = InstalledArtifact {
            manifest,
            library_path: destination.join(&filename),
        };
        // Publication is coordinated by the shared JsonStore lock mechanism,
        // independently of any registry using this installation root. Hashing an
        // existing artifact must not hold up instance admission on a registry lock.
        {
            let publication = JsonStore::new(
                root.join(&name).join(format!("{digest}.artifact")),
                "Extension artifact",
            );
            let _lock = publication.lock().map_err(extension::storage)?;
            self.check_root(&name)?;
            if destination.is_symlink() {
                return Err(ExtensionError::new(ExtensionErrorKind::Storage));
            }
            if destination.exists() {
                if destination.join(&filename).is_symlink() {
                    return Err(ExtensionError::new(ExtensionErrorKind::Storage));
                }
                let existing_digest = extension::hash_artifact(&destination.join(&filename))?;
                if existing_digest != digest {
                    return Err(ExtensionError::new(ExtensionErrorKind::DigestMismatch));
                }
                let previous = Extension::from_file(destination.join("manifest.json"))?;
                if previous.artifact != installation.manifest.artifact
                    || previous.descriptor() != installation.manifest.descriptor()
                {
                    return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
                }
            } else {
                sync_artifact_directories(staging.path())?;
                fs::rename(staging.path(), &destination).map_err(extension::io)?;
            }
        }
        sync_artifact_directories(&destination)?;
        // Never overwrite published files, or delete artifacts on registry failure.
        let store = self.store();
        let _lock = store.lock().map_err(extension::storage)?;
        let mut latest = Self::read(&self.registry)?;
        if let Some(record) = latest.extensions.iter_mut().find(|e| e.name == name) {
            if let Some(previous) = record
                .artifacts
                .iter()
                .find(|a| a.manifest.artifact.digest == digest)
            {
                if previous.manifest.artifact != installation.manifest.artifact
                    || previous.manifest.descriptor() != installation.manifest.descriptor()
                    || previous.library_path != installation.library_path
                {
                    return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
                }
            } else {
                record.artifacts.push(installation);
            }
            record.selected = digest;
        } else {
            latest.extensions.push(InstalledExtension {
                name,
                enabled: false,
                selected: digest,
                artifacts: vec![installation],
            });
        }
        let result = store.write(&latest);
        if result.is_ok()
            || result.as_ref().err().is_some_and(|e| {
                e.downcast_ref::<JsonStoreIoError>()
                    .is_some_and(JsonStoreIoError::replacement_completed)
            })
        {
            self.snapshot = latest;
        }
        result.map_err(extension::storage)
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
        self.mutate(|r| {
            let index = r
                .extensions
                .iter()
                .position(|e| e.name == name)
                .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
            r.extensions.remove(index);
            Ok(())
        })
    }
    fn status(&mut self, name: &str, enabled: bool) -> ExtensionResult<()> {
        extension::validate_name(name)?;
        self.mutate(|r| {
            r.extensions
                .iter_mut()
                .find(|e| e.name == name)
                .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?
                .enabled = enabled;
            Ok(())
        })
    }
    /// Enables future admissions without loading code.
    pub fn enable_extension(&mut self, name: &str) -> ExtensionResult<()> {
        self.status(name, true)
    }
    /// Blocks future admissions; existing instances remain valid.
    pub fn disable_extension(&mut self, name: &str) -> ExtensionResult<()> {
        self.status(name, false)
    }
    /// Atomically selects an already installed digest without loading code.
    pub fn select_extension(&mut self, name: &str, digest: &str) -> ExtensionResult<()> {
        extension::validate_name(name)?;
        self.mutate(|r| {
            let record = r
                .extensions
                .iter_mut()
                .find(|e| e.name == name)
                .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
            if !record
                .artifacts
                .iter()
                .any(|a| a.manifest.artifact.digest == digest)
            {
                return Err(ExtensionError::new(ExtensionErrorKind::Missing));
            }
            record.selected = digest.to_owned();
            Ok(())
        })
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
        let registry = Self::read(&self.registry)?;
        let record = registry
            .extensions
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
        if !record.enabled {
            return Err(ExtensionError::new(ExtensionErrorKind::Disabled));
        }
        let mut candidates = vec![];
        // Prefer the closest usable artifact: ABI supported, then host target,
        // then capabilities. This classification is independent of install order.
        let mut failures = [false; 3];
        for artifact in &record.artifacts {
            let identity = &artifact.manifest.artifact;
            if digest.is_some_and(|d| d != identity.digest)
                || requirement
                    .as_ref()
                    .is_some_and(|r| !r.matches(&identity.version))
                || (version.is_none() && digest.is_none() && identity.digest != record.selected)
            {
                continue;
            }
            if let Err(e) = artifact.manifest.compatible() {
                failures[usize::from(e.kind == ExtensionErrorKind::TargetMismatch)] = true;
                continue;
            }
            let mut negotiated = NegotiatedCapabilities::new();
            for (cap, acceptable) in required {
                if let Some(revision) = artifact
                    .manifest
                    .capabilities
                    .get(cap)
                    .and_then(|supported| supported.iter().filter(|r| acceptable.contains(r)).max())
                {
                    negotiated.insert(cap.clone(), *revision);
                }
            }
            if negotiated.len() != required.len() {
                failures[2] = true;
                continue;
            }
            candidates.push((artifact, negotiated));
        }
        candidates.sort_by(|a, b| {
            b.0.manifest
                .artifact
                .version
                .cmp_precedence(&a.0.manifest.artifact.version)
        });
        let Some((installation, capabilities)) = candidates.first() else {
            let failure = if failures[2] {
                ExtensionErrorKind::MissingCapability
            } else if failures[1] {
                ExtensionErrorKind::TargetMismatch
            } else if failures[0] {
                ExtensionErrorKind::UnsupportedAbi
            } else {
                ExtensionErrorKind::NoMatchingArtifact
            };
            return Err(ExtensionError::new(failure));
        };
        if candidates.get(1).is_some_and(|other| {
            other
                .0
                .manifest
                .artifact
                .version
                .cmp_precedence(&installation.manifest.artifact.version)
                .is_eq()
        }) {
            return Err(ExtensionError::new(ExtensionErrorKind::AmbiguousArtifact));
        }
        Ok(ResolvedExtension {
            installation: (*installation).clone(),
            capabilities: capabilities.clone(),
            registry: self.registry.clone(),
        })
    }
    fn admit(&self, resolved: &ResolvedExtension) -> ExtensionResult<ResolvedExtension> {
        let store = self.store();
        let _lock = store.lock().map_err(extension::storage)?;
        let registry = Self::read(&self.registry)?;
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
mod tests {
    use super::*;

    #[tokio::test]
    async fn optional_entries_skip_fresh_admission_failures_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager = ExtensionManager::open(dir.path().join("registry.json")).unwrap();
        let source = dir.path().join("source");
        fs::write(&source, b"not native code").unwrap();
        let manifest = dir.path().join("manifest.json");
        let value = serde_json::json!({"manifest_version":1,"name":"optional-race","version":"1.0.0","target":HOST_TARGET,"abi_version":1,"digest":extension::hash_artifact(&source).unwrap(),"path":"source","capabilities":{}});
        fs::write(&manifest, value.to_string()).unwrap();
        manager.add_extension(manifest).await.unwrap();
        for expected in [ExtensionErrorKind::Disabled, ExtensionErrorKind::Missing] {
            manager.enable_extension("optional-race").unwrap();
            let resolved = manager
                .resolve("optional-race", None, None, &Capabilities::new())
                .unwrap();
            // Resolve succeeds; another manager changes metadata before admission.
            let mut concurrent = manager.clone();
            if expected == ExtensionErrorKind::Disabled {
                concurrent.disable_extension("optional-race").unwrap();
            } else {
                concurrent.remove_extension("optional-race").unwrap();
            }
            assert!(
                optional_entry(false, unsafe { manager.create_instance(&resolved, b"{}") })
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                optional_entry(true, unsafe { manager.create_instance(&resolved, b"{}") })
                    .unwrap_err()
                    .kind,
                expected
            );
        }
        assert_eq!(
            optional_entry::<()>(
                false,
                Err(ExtensionError::new(
                    ExtensionErrorKind::InitializationFailed
                ))
            )
            .unwrap_err()
            .kind,
            ExtensionErrorKind::InitializationFailed
        );
    }
}
