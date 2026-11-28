//! The only native-code boundary. No registry locks enter this module.
use super::*;
use ga4gh_extension_abi as abi;
use std::{
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex, OnceLock, Weak},
};

/// Loaded library and validated API. Instances retain it independently of managers.
pub struct LoadedModule {
    _library: libloading::Library,
    descriptor: ExtensionDescriptor,
    create: abi::CreateInstance,
    invoke: abi::Invoke,
    destroy: abi::DestroyInstance,
    free: abi::FreeBuffer,
}

impl fmt::Debug for LoadedModule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoadedModule")
            .field("descriptor", &self.descriptor)
            .finish_non_exhaustive()
    }
}

impl LoadedModule {
    /// The validated, immutable exported descriptor.
    pub fn descriptor(&self) -> &ExtensionDescriptor {
        &self.descriptor
    }
}

#[derive(Default)]
struct ModuleSlot {
    pinned_artifact: Option<ArtifactIdentity>,
    module: Weak<LoadedModule>,
}

type ModuleSlots = Mutex<BTreeMap<String, Arc<Mutex<ModuleSlot>>>>;
fn slots() -> &'static ModuleSlots {
    static SLOTS: OnceLock<ModuleSlots> = OnceLock::new();
    SLOTS.get_or_init(Default::default)
}

fn lock_unpoisoned<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn is_loaded(name: &str) -> bool {
    let slot = lock_unpoisoned(slots()).get(name).cloned();
    slot.is_some_and(|slot| lock_unpoisoned(&slot).module.strong_count() > 0)
}
// SAFETY: caller has admitted this exact resolution, and trusts its native code
// to implement the ABI, allocation, lifetime, and cross-thread contract.
pub(crate) unsafe fn load(resolved: &ResolvedExtension) -> ExtensionResult<Arc<LoadedModule>> {
    let manifest = &resolved.installation.manifest;
    let slot = {
        let mut slots = lock_unpoisoned(slots());
        slots
            .entry(manifest.artifact.name.clone())
            .or_default()
            .clone()
    };
    // Only this extension's load gate is held across foreign code. The global
    // map and registry file lock are not held. Other extensions remain independent.
    let mut state = lock_unpoisoned(&slot);
    if let Some(current) = &state.pinned_artifact {
        if current != &manifest.artifact {
            let mut e = ExtensionError::new(ExtensionErrorKind::RestartRequired);
            e.current = Some(Box::new(current.clone()));
            e.requested = Some(Box::new(manifest.artifact.clone()));
            return Err(e);
        }
    }
    manifest.compatible()?;
    if hash_artifact(resolved.library_path())? != manifest.artifact.digest {
        return Err(ExtensionError::new(ExtensionErrorKind::DigestMismatch));
    }
    if let Some(module) = state.module.upgrade() {
        if module.descriptor != manifest.descriptor() {
            return Err(ExtensionError::new(ExtensionErrorKind::InvalidApi));
        }
        return Ok(module);
    }
    // SAFETY: delegated to the explicit loader's caller. Managed paths are absolute.
    let library = unsafe { libloading::Library::new(resolved.library_path()) }.map_err(|e| {
        ExtensionError::caused(ExtensionErrorKind::LoadFailed, crate::Error::other(e))
    })?;
    // Constructors may already have executed. Keep the pin even on later failure.
    state.pinned_artifact = Some(manifest.artifact.clone());
    let module = Arc::new(unsafe { validate_api(library, manifest) }?);
    state.module = Arc::downgrade(&module);
    Ok(module)
}
// SAFETY: the caller retains the process load gate and trusts the complete ABI
// contract. Ownership transfers here so validation failure also releases the library.
unsafe fn validate_api(
    library: libloading::Library,
    manifest: &Extension,
) -> ExtensionResult<LoadedModule> {
    let entry = unsafe { library.get::<abi::GetApiV1>(abi::ENTRY_POINT) }.map_err(|e| {
        ExtensionError::caused(ExtensionErrorKind::LoadFailed, crate::Error::other(e))
    })?;
    let api = unsafe { entry() };
    if api.is_null() {
        return Err(ExtensionError::new(ExtensionErrorKind::InvalidApi));
    }
    // Read only the stable prefix before accessing the complete known table.
    let header = unsafe { ptr::read(api.cast::<abi::Header>()) };
    if header.abi_version != abi::ABI_VERSION {
        return Err(ExtensionError::new(ExtensionErrorKind::UnsupportedAbi));
    }
    if (header.struct_size as usize) < std::mem::size_of::<abi::ApiV1>() {
        return Err(ExtensionError::new(ExtensionErrorKind::InvalidApi));
    }
    let api = unsafe { ptr::read(api.cast::<abi::ApiV1>()) };
    let (create, invoke, destroy, free) = (
        api.create_instance.ok_or_else(invalid_api)?,
        api.invoke.ok_or_else(invalid_api)?,
        api.destroy_instance.ok_or_else(invalid_api)?,
        api.free_buffer.ok_or_else(invalid_api)?,
    );
    let mut descriptor: ExtensionDescriptor =
        serde_json::from_slice(unsafe { borrow_bytes(api.descriptor)? })
            .map_err(|_| invalid_api())?;
    for revisions in descriptor.capabilities.values_mut() {
        revisions.sort_unstable();
    }
    if descriptor != manifest.descriptor() {
        return Err(invalid_api());
    }
    Ok(LoadedModule {
        _library: library,
        descriptor,
        create,
        invoke,
        destroy,
        free,
    })
}

fn invalid_api() -> ExtensionError {
    ExtensionError::new(ExtensionErrorKind::InvalidApi)
}

unsafe fn borrow_bytes<'a>(bytes: abi::Bytes) -> ExtensionResult<&'a [u8]> {
    if bytes.len == 0 {
        return Ok(&[]);
    }
    if bytes.data.is_null() || bytes.len > isize::MAX as usize {
        return Err(ExtensionError::new(ExtensionErrorKind::InvalidApi));
    }
    Ok(unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) })
}
// Always release outputs, including errors and empty buffers, while their module
// and (if any) instance are alive. Caller holds the instance operation lock.
unsafe fn copy_and_free_output(
    module: &LoadedModule,
    instance: *mut c_void,
    bytes: abi::OwnedBytes,
    limit: usize,
) -> ExtensionResult<Vec<u8>> {
    if bytes.len > limit {
        unsafe { (module.free)(instance, bytes) };
        return Err(invalid_api());
    }
    let copied = unsafe {
        borrow_bytes(abi::Bytes {
            data: bytes.data,
            len: bytes.len,
        })
    }
    .map(<[u8]>::to_vec);
    unsafe { (module.free)(instance, bytes) };
    copied
}

struct InstanceHandle(*mut c_void);
// SAFETY: the ABI requires cross-thread use; the enclosing Mutex serializes
// every operation on this instance. Thread-affine plugins do not meet v1.
unsafe impl Send for InstanceHandle {}
struct InstanceInner {
    module: Arc<LoadedModule>,
    instance: Mutex<InstanceHandle>,
    capabilities: NegotiatedCapabilities,
}

impl Drop for InstanceInner {
    fn drop(&mut self) {
        let handle = self.instance.get_mut().unwrap_or_else(|e| e.into_inner());
        // Last Arc: no operations or result releases can still be active.
        unsafe { (self.module.destroy)(handle.0) };
    }
}

/// One independent plugin instance. Clones share and serialize its operations.
#[derive(Clone)]
pub struct ExtensionInstance {
    inner: Arc<InstanceInner>,
}

impl fmt::Debug for ExtensionInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExtensionInstance")
            .field("capabilities", &self.inner.capabilities)
            .finish_non_exhaustive()
    }
}

impl ExtensionInstance {
    pub(crate) fn create(
        module: Arc<LoadedModule>,
        capabilities: NegotiatedCapabilities,
        config: &[u8],
    ) -> ExtensionResult<Self> {
        let mut instance = ptr::null_mut();
        let mut error = abi::OwnedBytes::default();
        // SAFETY: this module passed the explicit native trust boundary. Different
        // creations may overlap, as required by the plugin contract.
        let status = unsafe { (module.create)(abi::Bytes::new(config), &mut instance, &mut error) };
        let error_bytes =
            unsafe { copy_and_free_output(&module, ptr::null_mut(), error, usize::MAX) };
        let empty_error = error_bytes.as_ref().is_ok_and(Vec::is_empty);
        let valid_instance = !instance.is_null();
        if status != abi::OK || !empty_error || !valid_instance {
            if !instance.is_null() {
                unsafe { (module.destroy)(instance) };
            }
            return Err(if status != abi::OK {
                ExtensionError::plugin(
                    ExtensionErrorKind::InitializationFailed,
                    status,
                    error_bytes?,
                )
            } else {
                ExtensionError::new(ExtensionErrorKind::InvalidApi)
            });
        }
        Ok(Self {
            inner: Arc::new(InstanceInner {
                module,
                instance: Mutex::new(InstanceHandle(instance)),
                capabilities,
            }),
        })
    }

    /// Negotiated interfaces available to this instance.
    pub fn capabilities(&self) -> &NegotiatedCapabilities {
        &self.inner.capabilities
    }

    /// Invokes a negotiated capability and copies the plugin result into host memory.
    /// Error payloads are retained explicitly and excluded from default diagnostics.
    pub fn invoke(
        &self,
        capability: &str,
        operation: &str,
        input: &[u8],
    ) -> ExtensionResult<Vec<u8>> {
        self.invoke_bounded(capability, operation, input, usize::MAX)
    }

    /// Invokes a method without copying success or error buffers larger than `limit`.
    pub fn invoke_bounded(
        &self,
        capability: &str,
        operation: &str,
        input: &[u8],
        limit: usize,
    ) -> ExtensionResult<Vec<u8>> {
        let revision = *self
            .inner
            .capabilities
            .get(capability)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::MissingCapability))?;
        let instance = lock_unpoisoned(&self.inner.instance);
        let mut result = abi::OwnedBytes::default();
        let module = &self.inner.module;
        let status = unsafe {
            (module.invoke)(
                instance.0,
                abi::Bytes::new(capability.as_bytes()),
                revision,
                abi::Bytes::new(operation.as_bytes()),
                abi::Bytes::new(input),
                &mut result,
            )
        };
        let bytes = unsafe { copy_and_free_output(module, instance.0, result, limit) }?;
        if status != abi::OK {
            return Err(ExtensionError::plugin(
                ExtensionErrorKind::InvocationFailed,
                status,
                bytes,
            ));
        }
        Ok(bytes)
    }

    /// Retains this instance and binds one negotiated capability and operation.
    pub fn method(&self, capability: &str, operation: &str) -> ExtensionResult<ExtensionMethod> {
        if !self.inner.capabilities.contains_key(capability) {
            return Err(ExtensionError::new(ExtensionErrorKind::MissingCapability));
        }
        Ok(ExtensionMethod {
            instance: self.clone(),
            capability: capability.to_owned(),
            operation: operation.to_owned(),
        })
    }
}

/// Invocation handle that keeps its instance and module alive.
#[derive(Debug, Clone)]
pub struct ExtensionMethod {
    instance: ExtensionInstance,
    capability: String,
    operation: String,
}

impl ExtensionMethod {
    /// Calls the bound operation with opaque input bytes.
    pub fn call(&self, input: &[u8]) -> ExtensionResult<Vec<u8>> {
        self.instance
            .invoke(&self.capability, &self.operation, input)
    }
}
