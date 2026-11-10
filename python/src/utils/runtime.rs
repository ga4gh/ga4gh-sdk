//! Python I/O runs without the GIL on a process-lifetime runtime.
//!
//! Multi-thread runtime drivers remain alive between calls, retaining HTTP pools.
//! Only owned Rust data crosses the GIL boundary. Convert errors to strings there
//! (the current SDK's boxed errors are not Send); create Python exceptions after
//! reacquiring the GIL. No Python callbacks or borrowed Python objects run here.
//!
//! Synchronous calls block on the runtime with [`io`]. Asynchronous calls run on the same
//! runtime as `asyncio` futures, with [`future_into_py`].
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::future::Future;
use std::sync::OnceLock;
use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Result<Runtime, std::io::Error>> = OnceLock::new();

/// The process-lifetime runtime, started on first use.
fn runtime() -> Result<&'static Runtime, String> {
    RUNTIME
        .get_or_init(Runtime::new)
        .as_ref()
        .map_err(|e| format!("Cannot start async runtime: {e}"))
}

/// The runtime, also handed to `pyo3-async-runtimes` for [`future_into_py`].
fn shared_runtime() -> Result<&'static Runtime, String> {
    static SHARED: OnceLock<()> = OnceLock::new();
    let runtime = runtime()?;
    SHARED.get_or_init(|| {
        // Fails only if `pyo3-async-runtimes` already has a runtime: no call in this crate
        // starts one before this.
        let _ = pyo3_async_runtimes::tokio::init_with_runtime(runtime);
    });
    Ok(runtime)
}

pub fn io<T: Send>(
    py: Python<'_>,
    operation: impl FnOnce(&Runtime) -> Result<T, String> + Send,
) -> PyResult<T> {
    py.allow_threads(|| operation(runtime()?))
        .map_err(PyRuntimeError::new_err)
}

/// Runs the future that `make` creates on the runtime and gives its result.
///
/// The SDK's futures are not `Send` (they hold boxed errors across awaits), so they can't
/// be spawned as tasks. Each one runs in a thread of the runtime's blocking pool instead,
/// which blocks on it there. The future that this function returns is `Send`; dropping it
/// drops the SDK's future, which cancels its request.
pub async fn spawn_io<F, Fut, T>(make: F) -> Result<T, String>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, String>> + 'static,
    T: Send + 'static,
{
    let runtime = runtime()?;
    // Dropped with the returned future, which ends the `select!` below.
    let (_cancel, cancelled) = tokio::sync::oneshot::channel::<()>();
    let task = runtime.spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(async move {
            tokio::select! {
                result = make() => Some(result),
                _ = cancelled => None,
            }
        })
    });
    match task.await {
        Ok(Some(result)) => result,
        Ok(None) => Err("The request was cancelled".to_owned()),
        Err(error) => Err(format!("The request failed: {error}")),
    }
}

/// Runs `future` on the runtime and returns an `asyncio.Future` of the running event loop.
///
/// The future runs without the GIL. Cancelling the `asyncio.Future` drops it. Raises
/// `RuntimeError` if no event loop is running.
pub fn future_into_py<F, T>(py: Python<'_>, future: F) -> PyResult<Bound<'_, PyAny>>
where
    F: Future<Output = PyResult<T>> + Send + 'static,
    T: IntoPy<PyObject> + Send + 'static,
{
    shared_runtime().map_err(PyRuntimeError::new_err)?;
    pyo3_async_runtimes::tokio::future_into_py(py, future)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn spawn_io_runs_futures_that_are_not_send() {
        let result = runtime().unwrap().block_on(spawn_io(|| async {
            let value = Rc::new(5); // Not `Send`, and held across the await.
            tokio::task::yield_now().await;
            Ok(*value)
        }));
        assert_eq!(result, Ok(5));
    }

    #[test]
    fn dropping_spawn_io_drops_its_future() {
        struct SetOnDrop(Arc<AtomicBool>);
        impl Drop for SetOnDrop {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let flag = dropped.clone();
        let call = spawn_io(move || {
            let guard = SetOnDrop(flag);
            async move {
                let _guard = guard;
                std::future::pending::<Result<(), String>>().await
            }
        });
        runtime().unwrap().block_on(async {
            tokio::select! {
                _ = call => panic!("the pending future finished"),
                _ = tokio::task::yield_now() => {}
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dropped.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the future is still running");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn async_calls_share_the_runtime_of_sync_calls() {
        let runtime = shared_runtime().unwrap();
        assert!(std::ptr::eq(
            pyo3_async_runtimes::tokio::get_runtime(),
            runtime
        ));
    }
}
