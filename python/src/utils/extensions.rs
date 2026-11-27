//! Native extension bindings. Only explicit load/create executes trusted code.
use crate::utils::{convert::to_python, runtime};
use ::ga4gh_sdk::utils::{
    extension::{
        Capabilities, ExtensionError as NativeError, ExtensionInstance, ExtensionMethod,
        LoadedModule, NegotiatedCapabilities as CapabilitiesMap, ResolvedExtension,
    },
    extension_manager::ExtensionManager as Manager,
};
use pyo3::{exceptions::PyRuntimeError, prelude::*, types::PyBytes};
use std::sync::Arc;
pyo3::create_exception!(
    ga4gh_sdk,
    ExtensionError,
    PyRuntimeError,
    "A classified native extension failure with redacted default diagnostics."
);
fn error(py: Python<'_>, native: NativeError) -> PyErr {
    let error = ExtensionError::new_err(native.to_string());
    let value = error.value(py);
    let _ = value.setattr("kind", format!("{:?}", native.kind));
    let _ = value.setattr("status", native.status);
    let _ = value.setattr(
        "private_payload",
        PyBytes::new(py, native.private_payload()),
    );
    for (key, identity) in [
        ("current", &native.current),
        ("requested", &native.requested),
    ] {
        if let Ok(data) = to_python(
            py,
            &serde_json::to_value(identity).expect("identity serializes"),
        ) {
            let _ = value.setattr(key, data);
        }
    }
    error
}
// Release native resources without the GIL, including on ordinary Python GC.
fn release<T: Send>(value: T) {
    let _ = Python::try_attach(|py| py.detach(|| drop(value)));
}

/// Manage native installations with explicit registry and artifact paths.
#[pyclass(name = "ExtensionManager", module = "ga4gh_sdk", subclass)]
pub struct PyExtensionManager {
    pub inner: Manager,
}

#[pymethods]
impl PyExtensionManager {
    /// Return this loader's full target triple for installation manifests.
    #[staticmethod]
    fn host_target() -> &'static str {
        ::ga4gh_sdk::utils::extension::HOST_TARGET
    }

    /// Open metadata without loading code.
    ///
    /// Args:
    ///     registry_path: Path to the JSON registry.
    ///     installation_root: Directory for immutable artifacts.
    #[new]
    fn new(py: Python<'_>, registry_path: String, installation_root: String) -> PyResult<Self> {
        Manager::with_paths(registry_path, installation_root)
            .map(|inner| Self { inner })
            .map_err(|e| error(py, e))
    }

    /// Install and select local or downloaded bytes without executing them.
    ///
    /// Args:
    ///     manifest: Path to a v1 JSON manifest.
    fn install(&mut self, py: Python<'_>, manifest: String) -> PyResult<()> {
        let result = runtime::io(py, |rt| Ok(rt.block_on(self.inner.add_extension(manifest))))?;
        result.map_err(|e| error(py, e))
    }

    /// Read the latest installation metadata without loading code.
    fn list(&mut self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        self.inner.refresh().map_err(|e| error(py, e))?;
        to_python(
            py,
            &serde_json::to_value(self.inner.get_extensions()).expect("metadata serializes"),
        )
    }

    /// Enable future instance creation without loading code.
    ///
    /// Args:
    ///     name: Registered extension name.
    fn enable(&mut self, py: Python<'_>, name: String) -> PyResult<()> {
        py.detach(|| self.inner.enable_extension(&name))
            .map_err(|e| error(py, e))
    }

    /// Block new instances while existing handles continue working.
    ///
    /// Args:
    ///     name: Registered extension name.
    fn disable(&mut self, py: Python<'_>, name: String) -> PyResult<()> {
        py.detach(|| self.inner.disable_extension(&name))
            .map_err(|e| error(py, e))
    }

    /// Remove registration; retain artifact files and their disk usage.
    ///
    /// Args:
    ///     name: Registered extension name.
    fn remove(&mut self, py: Python<'_>, name: String) -> PyResult<()> {
        py.detach(|| self.inner.remove_extension(&name))
            .map_err(|e| error(py, e))
    }

    /// Select installed bytes without loading or switching existing clients.
    ///
    /// Args:
    ///     name: Registered extension name.
    ///     digest: Exact SHA-512 artifact digest.
    fn select(&mut self, py: Python<'_>, name: String, digest: String) -> PyResult<()> {
        py.detach(|| self.inner.select_extension(&name, &digest))
            .map_err(|e| error(py, e))
    }

    /// Resolve one exact artifact and negotiate required interfaces.
    ///
    /// Args:
    ///     name: Registered extension name.
    ///     version: Release requirement; None uses the selected artifact.
    ///     digest: Optional exact SHA-512 pin.
    ///     capabilities: Required names and acceptable interface revisions.
    #[pyo3(signature=(name, version=None, digest=None, capabilities=None))]
    fn resolve(
        &self,
        py: Python<'_>,
        name: String,
        version: Option<String>,
        digest: Option<String>,
        capabilities: Option<Capabilities>,
    ) -> PyResult<PyResolvedExtension> {
        let capabilities = capabilities.unwrap_or_default();
        py.detach(|| {
            self.inner
                .resolve(&name, version.as_deref(), digest.as_deref(), &capabilities)
        })
        .map(|inner| PyResolvedExtension { inner })
        .map_err(|e| error(py, e))
    }

    /// Explicitly load trusted native code without creating an instance.
    ///
    /// Args:
    ///     resolved: Exact artifact implementing the C ABI v1 trust contract.
    fn load(
        &self,
        py: Python<'_>,
        resolved: PyRef<'_, PyResolvedExtension>,
    ) -> PyResult<PyExtensionModule> {
        let resolved = resolved.inner.clone();
        unsafe { py.detach(|| self.inner.load(&resolved)) }
            .map(|inner| PyExtensionModule { inner: Some(inner) })
            .map_err(|e| error(py, e))
    }

    /// Create an independent instance of trusted native code without the GIL.
    ///
    /// Args:
    ///     resolved: Exact artifact implementing the C ABI v1 trust contract.
    ///     configuration: Opaque instance configuration bytes.
    fn create_instance(
        &self,
        py: Python<'_>,
        resolved: PyRef<'_, PyResolvedExtension>,
        configuration: Vec<u8>,
    ) -> PyResult<PyExtensionInstance> {
        let resolved = resolved.inner.clone();
        unsafe { py.detach(|| self.inner.create_instance(&resolved, &configuration)) }
            .map(|inner| PyExtensionInstance { inner: Some(inner) })
            .map_err(|e| error(py, e))
    }
}

/// An immutable artifact resolution and negotiated capability revisions.
#[pyclass(name = "ResolvedExtension", module = "ga4gh_sdk", subclass)]
pub struct PyResolvedExtension {
    pub inner: ResolvedExtension,
}

#[pymethods]
impl PyResolvedExtension {
    /// Return a copy of the exact version, target, digest, and ABI identity.
    fn artifact(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(
            py,
            &serde_json::to_value(self.inner.artifact()).expect("identity serializes"),
        )
    }

    /// Return a copy of negotiated capability interface revisions.
    fn capabilities(&self, _py: Python<'_>) -> CapabilitiesMap {
        self.inner.capabilities().clone()
    }
}

/// A loaded native module retained independently of its manager.
#[pyclass(name = "ExtensionModule", module = "ga4gh_sdk", subclass)]
pub struct PyExtensionModule {
    inner: Option<Arc<LoadedModule>>,
}

impl PyExtensionModule {
    /// Wraps a valid SDK handle for downstream bindings.
    pub fn from_inner(inner: Arc<LoadedModule>) -> Self {
        Self { inner: Some(inner) }
    }

    /// Borrows the SDK handle without exposing the wrapper's release state.
    pub fn inner(&self) -> &Arc<LoadedModule> {
        self.inner.as_ref().expect("live native wrapper")
    }
}

#[pymethods]
impl PyExtensionModule {
    /// Return a copy of the validated exported API descriptor.
    fn descriptor(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(
            py,
            &serde_json::to_value(self.inner.as_ref().expect("live module").descriptor())
                .expect("descriptor serializes"),
        )
    }
}

impl Drop for PyExtensionModule {
    fn drop(&mut self) {
        release(self.inner.take());
    }
}

/// An independent native instance whose operations are serialized across threads.
#[pyclass(name = "ExtensionInstance", module = "ga4gh_sdk", subclass)]
pub struct PyExtensionInstance {
    inner: Option<ExtensionInstance>,
}

impl PyExtensionInstance {
    /// Wraps a valid SDK handle for downstream bindings.
    pub fn from_inner(inner: ExtensionInstance) -> Self {
        Self { inner: Some(inner) }
    }

    /// Borrows the SDK handle without exposing the wrapper's release state.
    pub fn inner(&self) -> &ExtensionInstance {
        self.inner.as_ref().expect("live native wrapper")
    }
}

#[pymethods]
impl PyExtensionInstance {
    /// Return a copy of the instance capability interface revisions.
    fn capabilities(&self, _py: Python<'_>) -> CapabilitiesMap {
        self.inner
            .as_ref()
            .expect("live instance")
            .capabilities()
            .clone()
    }

    /// Invoke a negotiated interface without the GIL and copy its result.
    ///
    /// Args:
    ///     capability: Negotiated capability name.
    ///     operation: Adapter-defined operation name.
    ///     input: Opaque operation input bytes.
    fn invoke(
        &self,
        py: Python<'_>,
        capability: String,
        operation: String,
        input: Vec<u8>,
    ) -> PyResult<Py<PyBytes>> {
        let result = py
            .detach(|| {
                self.inner
                    .as_ref()
                    .expect("live instance")
                    .invoke(&capability, &operation, &input)
            })
            .map_err(|e| error(py, e))?;
        Ok(PyBytes::new(py, &result).unbind())
    }

    /// Retain this instance in a bound invocation handle.
    ///
    /// Args:
    ///     capability: Negotiated capability name.
    ///     operation: Adapter-defined operation name.
    fn method(
        &self,
        py: Python<'_>,
        capability: String,
        operation: String,
    ) -> PyResult<PyExtensionMethod> {
        self.inner
            .as_ref()
            .expect("live instance")
            .method(&capability, &operation)
            .map(|inner| PyExtensionMethod { inner: Some(inner) })
            .map_err(|e| error(py, e))
    }
}

impl Drop for PyExtensionInstance {
    fn drop(&mut self) {
        release(self.inner.take());
    }
}

/// A bound operation retaining its native instance and module.
#[pyclass(name = "ExtensionMethod", module = "ga4gh_sdk", subclass)]
pub struct PyExtensionMethod {
    inner: Option<ExtensionMethod>,
}

impl PyExtensionMethod {
    /// Wraps a valid SDK handle for downstream bindings.
    pub fn from_inner(inner: ExtensionMethod) -> Self {
        Self { inner: Some(inner) }
    }

    /// Borrows the SDK handle without exposing the wrapper's release state.
    pub fn inner(&self) -> &ExtensionMethod {
        self.inner.as_ref().expect("live native wrapper")
    }
}

#[pymethods]
impl PyExtensionMethod {
    /// Invoke the bound operation without the GIL.
    ///
    /// Args:
    ///     input: Opaque operation input bytes.
    fn call(&self, py: Python<'_>, input: Vec<u8>) -> PyResult<Py<PyBytes>> {
        let result = py
            .detach(|| self.inner.as_ref().expect("live method").call(&input))
            .map_err(|e| error(py, e))?;
        Ok(PyBytes::new(py, &result).unbind())
    }
}

impl Drop for PyExtensionMethod {
    fn drop(&mut self) {
        release(self.inner.take());
    }
}

pub fn register(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    let error = py.get_type::<ExtensionError>();
    error.setattr("kind", "")?;
    error.setattr("status", py.None())?;
    error.setattr("current", py.None())?;
    error.setattr("requested", py.None())?;
    error.setattr("private_payload", PyBytes::new(py, &[]))?;
    m.add("ExtensionError", error)?;
    m.add_class::<PyExtensionManager>()?;
    m.add_class::<PyResolvedExtension>()?;
    m.add_class::<PyExtensionModule>()?;
    m.add_class::<PyExtensionInstance>()?;
    m.add_class::<PyExtensionMethod>()?;
    Ok(())
}
