//! Experimental native extensions, loaded only through an explicit unsafe call.
//!
//! Installation metadata can be read without executing native code.

use libloading::Library;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

type Init = unsafe extern "Rust" fn(Value) -> Vec<&'static [&'static str]>;
type Method = unsafe extern "Rust" fn(Value) -> Value;
type Deinit = unsafe extern "C" fn();

struct LoadedLibrary {
    library: Library,
    deinit: Option<Deinit>,
}

impl Drop for LoadedLibrary {
    fn drop(&mut self) {
        if let Some(deinit) = self.deinit {
            // SAFETY: load's caller accepts the library's ABI and lifecycle contract.
            unsafe { deinit() };
        }
    }
}

/// A method exported by an extension. Clones keep its library loaded.
#[derive(Clone)]
pub struct ExtensionMethod {
    /// The extension providing this method.
    pub extension_name: String,
    /// The method category, such as `tls-verifier`.
    pub unified_name: String,
    /// The extension's name for the method.
    pub internal_name: String,
    method: Method,
    owner: Arc<LoadedLibrary>,
}

impl std::fmt::Debug for ExtensionMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionMethod")
            .field("extension_name", &self.extension_name)
            .field("unified_name", &self.unified_name)
            .field("internal_name", &self.internal_name)
            .finish_non_exhaustive()
    }
}

impl ExtensionMethod {
    /// Calls the method with extension-specific JSON.
    ///
    /// # Safety
    ///
    /// The caller must meet the extension's input and threading requirements.
    pub unsafe fn call(&self, input: Value) -> Value {
        let _keep_alive = &self.owner;
        // SAFETY: the caller accepts the method's contract; owner pins its code.
        unsafe { (self.method)(input) }
    }
}

/// An installation manifest for a shared library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extension {
    /// A single directory-safe extension name.
    pub name: String,
    /// The extension version.
    pub version: String,
    /// The HTTP or HTTPS URL of the shared library.
    pub path: String,
    /// The library's SHA-512 hex digest.
    pub checksum: String,
    /// An optional description.
    pub description: Option<String>,
}

impl Extension {
    /// Reads a manifest without executing code.
    ///
    /// # Errors
    ///
    /// Returns an I/O or JSON error if the manifest cannot be read.
    pub fn from_file(path: impl AsRef<Path>) -> crate::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }
}

/// An installed extension and its optional runtime state. Serialized records
/// contain metadata only; clones share ownership of a loaded library.
#[derive(Clone, Serialize, Deserialize)]
pub struct InstalledExtension {
    /// The extension name.
    pub name: String,
    /// The installed version.
    pub version: String,
    /// The installed manifest path.
    #[serde(rename = "definition-path")]
    pub definition_path: String,
    /// The installed shared library path.
    #[serde(rename = "library-path")]
    pub library_path: String,
    /// Whether this installation is enabled for explicit loading.
    #[serde(default)]
    pub enabled: bool,
    #[serde(skip)]
    library: Option<Arc<LoadedLibrary>>,
    #[serde(skip)]
    methods: HashMap<String, Vec<ExtensionMethod>>,
}

impl std::fmt::Debug for InstalledExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InstalledExtension")
            .field("name", &self.name)
            .field("version", &self.version)
            .field("enabled", &self.enabled)
            .field("loaded", &self.is_loaded())
            .finish_non_exhaustive()
    }
}

impl InstalledExtension {
    pub(crate) fn new(name: String, version: String, definition: String, library: String) -> Self {
        Self {
            name,
            version,
            definition_path: definition,
            library_path: library,
            enabled: false,
            library: None,
            methods: HashMap::new(),
        }
    }

    /// Reads installation metadata without executing code.
    ///
    /// # Errors
    ///
    /// Returns an I/O or JSON error if the record cannot be read.
    pub fn from_file(path: impl AsRef<Path>) -> crate::Result<Self> {
        Ok(serde_json::from_slice(&std::fs::read(path)?)?)
    }

    /// Enables loading in memory; use the manager to persist the status.
    pub fn enable(&mut self) {
        self.enabled = true;
    }

    /// Disables loading and releases this record's runtime handles.
    pub fn disable(&mut self) {
        self.enabled = false;
        self.unload();
    }

    /// Whether this record owns a loaded library.
    pub fn is_loaded(&self) -> bool {
        self.library.is_some()
    }

    pub(crate) fn methods(&self, category: &str) -> impl Iterator<Item = &ExtensionMethod> {
        self.methods.get(category).into_iter().flatten()
    }

    /// Loads an enabled library and calls `init`. Disabled or loaded records
    /// are left unchanged.
    ///
    /// # Safety
    ///
    /// The library must be trusted and built with the same Rust compiler and
    /// compatible dependency builds as the host. `init` must have type
    /// `unsafe extern "Rust" fn(Value) -> Vec<&'static [&'static str]>`.
    /// Each entry must contain category, internal name and symbol name; every
    /// named method must be `unsafe extern "Rust" fn(Value) -> Value`.
    /// Returned references must remain valid while the library is loaded.
    /// Optional `deinit` must be `unsafe extern "C" fn()` and safe on the thread
    /// dropping the last handle. Initializers and finalizers must not unwind.
    /// The caller must meet any extension-specific threading requirements.
    ///
    /// # Errors
    ///
    /// Returns an error for missing libraries or symbols and invalid descriptors.
    pub unsafe fn load(&mut self, service_config: Value) -> crate::Result<()> {
        if !self.enabled || self.is_loaded() {
            return Ok(());
        }
        // SAFETY: the caller accepts the native code and ABI contract above.
        let library = unsafe { Library::new(&self.library_path) }.map_err(crate::Error::other)?;
        let descriptors = {
            // SAFETY: the caller guarantees the initializer's signature.
            let init = unsafe { library.get::<Init>(b"init") }.map_err(crate::Error::other)?;
            unsafe { init(service_config) }
        };
        // SAFETY: if present, deinit has the signature promised by the caller.
        let deinit = unsafe { library.get::<Deinit>(b"deinit") }
            .ok()
            .map(|symbol| *symbol);
        let owner = Arc::new(LoadedLibrary { library, deinit });
        let mut methods: HashMap<String, Vec<ExtensionMethod>> = HashMap::new();
        for descriptor in descriptors {
            let [category, name, symbol] = descriptor else {
                return Err("Extension method descriptor must contain three strings".into());
            };
            // SAFETY: load's caller guarantees the method ABI; owner pins the code.
            let method = *unsafe { owner.library.get::<Method>(symbol.as_bytes()) }
                .map_err(crate::Error::other)?;
            methods
                .entry(category.to_string())
                .or_default()
                .push(ExtensionMethod {
                    extension_name: self.name.clone(),
                    unified_name: category.to_string(),
                    internal_name: name.to_string(),
                    method,
                    owner: owner.clone(),
                });
        }
        self.methods = methods;
        self.library = Some(owner);
        Ok(())
    }

    /// Releases this record's handles. Retained record or method clones keep the
    /// library alive; optional `deinit` runs when the last owner is dropped.
    pub fn unload(&mut self) {
        self.methods.clear();
        self.library = None;
    }
}
