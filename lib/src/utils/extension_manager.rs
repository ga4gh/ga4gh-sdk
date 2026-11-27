//! Installation records and explicit loading of experimental native extensions.

use crate::utils::configuration::{Configuration, ServiceExtensionsConfiguration};
use crate::utils::extension::{Extension, ExtensionMethod, InstalledExtension};
use crate::utils::json_store::{JsonStore, JsonStoreIoError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// A registry snapshot. Mutations update both the snapshot and the file.
/// Mutations reload the registry under its shared writer lock. Reopen to observe
/// other processes' changes without making a change yourself.
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ExtensionManager {
    extensions: Vec<InstalledExtension>,
    #[serde(skip)]
    config_path: PathBuf,
}

fn validate_name(name: &str) -> crate::Result<()> {
    if matches!(name, "" | "." | "..")
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(
            "Extension name must be one nonempty ASCII filename, excluding '.' and '..'".into(),
        );
    }
    Ok(())
}

fn replacement_completed(error: &crate::Error) -> bool {
    error
        .downcast_ref::<JsonStoreIoError>()
        .is_some_and(JsonStoreIoError::replacement_completed)
}

impl ExtensionManager {
    /// Opens metadata without loading code or creating files. A missing registry
    /// represents an empty installation.
    ///
    /// # Errors
    ///
    /// Rejects unreadable or malformed registries and duplicate or invalid names.
    pub fn open(path: impl AsRef<Path>) -> crate::Result<Self> {
        let path = path.as_ref();
        let mut manager: Self = JsonStore::new(path, "Extension registry")
            .read()?
            .unwrap_or_default();
        let mut names = std::collections::HashSet::new();
        for extension in &manager.extensions {
            validate_name(&extension.name)?;
            if !names.insert(&extension.name) {
                return Err("Duplicate extension name in registry".into());
            }
        }
        manager.config_path = path.to_owned();
        Ok(manager)
    }

    /// Opens the registry and loads enabled extensions selected by a service.
    /// Unselected installations are never initialized. Required extensions must
    /// be installed and enabled; absent optional extensions are skipped.
    ///
    /// # Safety
    ///
    /// Each selected library must meet [`InstalledExtension::load`]'s contract.
    ///
    /// # Errors
    ///
    /// Fails for invalid metadata, unavailable required extensions, or load errors.
    pub unsafe fn init(
        path: impl AsRef<Path>,
        service_config: Option<ServiceExtensionsConfiguration>,
    ) -> crate::Result<Self> {
        let mut manager = Self::open(path)?;
        if let Some(config) = service_config {
            let mut selected_names = std::collections::HashSet::new();
            for selected in config.entries() {
                validate_name(&selected.extension_name)?;
                if !selected_names.insert(&selected.extension_name) {
                    return Err("Duplicate extension in service configuration".into());
                }
                let extension = manager.extensions.iter_mut().find(|extension| {
                    extension.name == selected.extension_name && extension.enabled
                });
                match extension {
                    Some(extension) => {
                        // SAFETY: delegated to init's caller for each selected library.
                        unsafe { extension.load(selected.configuration.clone()) }?;
                    }
                    None if selected.required => {
                        return Err(format!(
                            "Required extension '{}' is not installed and enabled",
                            selected.extension_name
                        )
                        .into())
                    }
                    None => {}
                }
            }
        }
        Ok(manager)
    }

    /// Reads an `extensions` array from a current SDK configuration's `extra`
    /// map and loads it using an explicit registry path. Other settings and
    /// credentials are not passed to native initializers.
    ///
    /// # Safety
    ///
    /// Each selected library must meet [`InstalledExtension::load`]'s contract.
    ///
    /// # Errors
    ///
    /// Fails as [`Self::init`] does, or if the service array is malformed.
    pub unsafe fn from_configuration(
        path: impl AsRef<Path>,
        config: &Configuration,
    ) -> crate::Result<Self> {
        let selected = config.extra_as::<ServiceExtensionsConfiguration>("extensions")?;
        // SAFETY: delegated to this function's caller.
        unsafe { Self::init(path, selected) }
    }

    /// The installed records, including their current loading status.
    pub fn get_extensions(&self) -> &[InstalledExtension] {
        &self.extensions
    }

    /// Finds enabled methods in a category. A cloned method keeps its library
    /// alive even after the manager is dropped or its record is unloaded.
    pub fn lookup_extension_methods(&self, category: &str) -> Vec<&ExtensionMethod> {
        self.extensions
            .iter()
            .filter(|extension| extension.enabled)
            .flat_map(|extension| extension.methods(category))
            .collect()
    }

    fn directory(&self) -> crate::Result<PathBuf> {
        if self.config_path.as_os_str().is_empty() {
            return Err("Open an extension registry before changing installations".into());
        }
        Ok(self
            .config_path
            .parent()
            .unwrap_or(Path::new(""))
            .join("extensions"))
    }

    fn extension_directory(&self, name: &str) -> crate::Result<PathBuf> {
        validate_name(name)?;
        let root = self.directory()?;
        if root.is_symlink() || root.join(name).is_symlink() {
            return Err("Extension directories must not be symbolic links".into());
        }
        Ok(root.join(name))
    }

    fn store(&self) -> crate::Result<JsonStore> {
        self.directory()?;
        Ok(JsonStore::new(&self.config_path, "Extension registry"))
    }

    // Called under the store lock, retaining loaded code only for unchanged
    // installation records. A registry reload must not initialize native code.
    fn reload(&mut self, store: &JsonStore) -> crate::Result<()> {
        let mut latest = Self::open(store.path())?;
        for extension in &mut latest.extensions {
            if let Some(previous) = self.extensions.iter().find(|previous| {
                previous.name == extension.name
                    && previous.version == extension.version
                    && previous.definition_path == extension.definition_path
                    && previous.library_path == extension.library_path
                    && previous.enabled == extension.enabled
            }) {
                *extension = previous.clone();
            }
        }
        *self = latest;
        Ok(())
    }

    /// Downloads a manifest's library, verifies SHA-512, then installs it disabled
    /// beside the registry in `extensions/<name>/`. Never executes the library.
    ///
    /// # Errors
    ///
    /// Rejects invalid names or URLs, duplicates, HTTP failures, checksum
    /// mismatches and I/O errors. Failed downloads leave existing installs intact.
    pub async fn add_extension(&mut self, manifest: impl AsRef<Path>) -> crate::Result<()> {
        let extension = Extension::from_file(manifest)?;
        let destination = self.extension_directory(&extension.name)?;
        if destination.exists() {
            return Err("Extension is already installed or its directory already exists".into());
        }
        if extension.checksum.len() != 128
            || !extension.checksum.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err("Extension checksum must be a SHA-512 hex digest".into());
        }
        let url = url::Url::parse(&extension.path)?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("Extension library URL must use HTTP or HTTPS without userinfo".into());
        }
        let filename = url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .ok_or("Extension URL must end in a library filename")?;
        validate_name(filename)?;
        let root = self.directory()?;
        fs::create_dir_all(&root)?;
        let destination = fs::canonicalize(&root)?.join(&extension.name);
        let staging = tempfile::tempdir_in(&root)?;
        // LoadLibrary on Windows appends .dll to extensionless paths.
        let library_name = format!("library{}", std::env::consts::DLL_SUFFIX);
        let mut response = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()?
            .get(url)
            .send()
            .await?
            .error_for_status()?;
        let mut file = fs::File::create(staging.path().join(&library_name))?;
        let mut digest = Sha512::new();
        while let Some(chunk) = response.chunk().await? {
            digest.update(&chunk);
            file.write_all(&chunk)?;
        }
        drop(file);
        let checksum: String = digest
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if checksum != extension.checksum.to_ascii_lowercase() {
            return Err("Extension library checksum mismatch".into());
        }
        let definition = format!("{}.ga4gh-sdk-extension.json", extension.name);
        JsonStore::new(staging.path().join(&definition), "Extension manifest").write(&extension)?;
        let store = self.store()?;
        let _lock = store.lock()?;
        self.reload(&store)?;
        self.extension_directory(&extension.name)?;
        if self
            .extensions
            .iter()
            .any(|installed| installed.name == extension.name)
            || destination.exists()
        {
            return Err("Extension is already installed or its directory already exists".into());
        }
        let record = InstalledExtension::new(
            extension.name,
            extension.version,
            destination
                .join(definition)
                .to_str()
                .ok_or("Extension path is not Unicode")?
                .to_owned(),
            destination
                .join(library_name)
                .to_str()
                .ok_or("Extension path is not Unicode")?
                .to_owned(),
        );
        fs::rename(staging.path(), &destination)?;
        self.extensions.push(record);
        let result = store.write(self);
        if let Err(error) = &result {
            if !replacement_completed(error) {
                self.extensions.pop();
                fs::remove_dir_all(destination)?;
            }
        }
        result
    }

    /// Removes a registered extension and its installation directory.
    ///
    /// # Errors
    ///
    /// Rejects invalid or unknown names and symbolic links; returns I/O errors.
    pub fn remove_extension(&mut self, name: &str) -> crate::Result<()> {
        validate_name(name)?;
        let store = self.store()?;
        let _lock = store.lock()?;
        self.reload(&store)?;
        let directory = self.extension_directory(name)?;
        let index = self
            .extensions
            .iter()
            .position(|extension| extension.name == name)
            .ok_or("Extension is not installed")?;
        let root = self.directory()?;
        fs::create_dir_all(&root)?;
        let removed = tempfile::tempdir_in(root)?;
        let backup = removed.path().join("installation");
        let exists = directory.exists();
        if exists {
            fs::rename(&directory, &backup)?;
        }
        let record = self.extensions.remove(index);
        let result = store.write(self);
        if let Err(error) = &result {
            if !replacement_completed(error) {
                self.extensions.insert(index, record);
                if exists {
                    if let Err(restore_error) = fs::rename(backup, directory) {
                        // Preserve the backup if restoring the original path also fails.
                        let _ = removed.keep();
                        return Err(restore_error.into());
                    }
                }
                return result;
            }
        }
        drop(record);
        let cleanup = removed.close().map_err(crate::Error::from);
        result.and(cleanup)
    }

    fn update_extension_status(&mut self, name: &str, enabled: bool) -> crate::Result<()> {
        validate_name(name)?;
        let store = self.store()?;
        let _lock = store.lock()?;
        self.reload(&store)?;
        let index = self
            .extensions
            .iter()
            .position(|extension| extension.name == name)
            .ok_or("Extension is not installed")?;
        let previous = self.extensions[index].enabled;
        self.extensions[index].enabled = enabled;
        let result = store.write(self);
        if let Err(error) = &result {
            if !replacement_completed(error) {
                self.extensions[index].enabled = previous;
                return result;
            }
        }
        if !enabled {
            self.extensions[index].unload();
        }
        result
    }

    /// Persists enabled status without loading native code.
    ///
    /// # Errors
    ///
    /// Fails for invalid or unknown names or an unwritable registry.
    pub fn enable_extension(&mut self, name: &str) -> crate::Result<()> {
        self.update_extension_status(name, true)
    }

    /// Persists disabled status and releases this manager's runtime handles.
    ///
    /// # Errors
    ///
    /// Fails for invalid or unknown names or an unwritable registry.
    pub fn disable_extension(&mut self, name: &str) -> crate::Result<()> {
        self.update_extension_status(name, false)
    }
}
