//! Synchronous Python I/O runs without the GIL on a process-lifetime runtime.
//!
//! Multi-thread runtime drivers remain alive between calls, retaining HTTP pools.
//! Only owned Rust data crosses the GIL boundary. Convert errors to strings there
//! (the current SDK's boxed errors are not Send); create Python exceptions after
//! reacquiring the GIL. No Python callbacks or borrowed Python objects run here.
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Result<Runtime, std::io::Error>> = OnceLock::new();

pub fn io<T: Send>(
    py: Python<'_>,
    operation: impl FnOnce(&Runtime) -> Result<T, String> + Send,
) -> PyResult<T> {
    py.allow_threads(|| {
        let runtime = RUNTIME
            .get_or_init(Runtime::new)
            .as_ref()
            .map_err(|e| format!("Cannot start async runtime: {e}"))?;
        operation(runtime)
    })
    .map_err(PyRuntimeError::new_err)
}
