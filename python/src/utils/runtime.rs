//! Python I/O runs without the GIL on a process-lifetime runtime.
//!
//! Multi-thread runtime drivers remain alive between calls, retaining HTTP pools.
//! Only owned Rust data crosses the GIL boundary. Convert errors to strings there
//! (the current SDK's boxed errors are not Send); create Python exceptions after
//! reacquiring the GIL. No Python callbacks or borrowed Python objects run here, except
//! token providers, which take the GIL on a blocking thread of the runtime (see
//! `Configuration.set_token_provider()`).
//!
//! Synchronous calls block on the runtime with [`io`]. Asynchronous calls run on the same
//! runtime as `asyncio` futures, with [`future_into_py`].
use pyo3::{exceptions::PyRuntimeError, prelude::*};
use std::future::Future;
use std::sync::{Arc, OnceLock};
use tokio::runtime::Runtime;
use tokio::sync::{oneshot, Semaphore};

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

/// How many async calls run at once; further calls wait for a slot.
pub const ASYNC_CALL_THREADS: usize = 256;

/// The name of the threads that run async calls.
const ASYNC_CALL_THREAD_NAME: &str = "ga4gh-async-call";

/// The pool of threads for [`spawn_io`]: the blocking pool of a runtime of its own.
///
/// It is separate from the blocking pool of [`runtime`], which the SDK's requests need
/// themselves, for example for DNS lookups: calls that wait there can't starve them.
fn call_threads() -> Result<&'static Runtime, String> {
    static CALLS: OnceLock<Result<Runtime, std::io::Error>> = OnceLock::new();
    CALLS
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .max_blocking_threads(ASYNC_CALL_THREADS)
                .thread_name(ASYNC_CALL_THREAD_NAME)
                .build()
        })
        .as_ref()
        .map_err(|e| format!("Cannot start async call threads: {e}"))
}

fn call_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(ASYNC_CALL_THREADS)))
        .clone()
}

async fn run_until_cancelled<F, Fut, T>(
    make: F,
    cancelled: oneshot::Receiver<()>,
) -> Option<Result<T, String>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    tokio::select! {
        biased;
        _ = cancelled => None,
        // Construct the SDK future only after the cancellation branch is checked.
        result = async move { make().await } => Some(result),
    }
}

/// Runs the future that `make` creates on the runtime and gives its result.
///
/// The SDK's futures are not `Send` (they hold boxed errors across awaits), so they can't
/// be spawned as tasks. Each one runs in a thread of its own pool instead (up to
/// [`ASYNC_CALL_THREADS`] at once), which blocks on it on the runtime. Further calls wait
/// for a slot before entering the blocking pool. The future that this function returns is
/// `Send`; dropping it drops the SDK's future, which cancels its request, or skips the call
/// if it hasn't started yet.
pub async fn spawn_io<F, Fut, T>(make: F) -> Result<T, String>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, String>> + 'static,
    T: Send + 'static,
{
    spawn_io_with_slots(make, call_slots()).await
}

async fn spawn_io_with_slots<F, Fut, T>(make: F, slots: Arc<Semaphore>) -> Result<T, String>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, String>> + 'static,
    T: Send + 'static,
{
    let runtime = runtime()?;
    // A cancelled waiter leaves this queue without scheduling a blocking task.
    let slot = slots
        .acquire_owned()
        .await
        .map_err(|_| "The async call pool is closed".to_owned())?;
    // Dropped with the returned future, which ends the `select!` below.
    let (_cancel, cancelled) = oneshot::channel::<()>();
    let task = call_threads()?.spawn_blocking(move || {
        let _slot = slot;
        runtime.block_on(run_until_cancelled(make, cancelled))
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
    fn spawn_io_leaves_the_blocking_pool_of_the_runtime_to_requests() {
        let names = runtime().unwrap().block_on(spawn_io(|| async {
            let call = std::thread::current().name().map(str::to_owned);
            // Such as the DNS lookups of the SDK's requests.
            let request =
                tokio::task::spawn_blocking(|| std::thread::current().name().map(str::to_owned))
                    .await
                    .map_err(|e| e.to_string())?;
            Ok((call, request))
        }));
        let (call, request) = names.unwrap();
        assert_eq!(call.as_deref(), Some(ASYNC_CALL_THREAD_NAME));
        assert_ne!(request.as_deref(), Some(ASYNC_CALL_THREAD_NAME));
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
        let (started, started_rx) = oneshot::channel::<()>();
        let call = spawn_io(move || {
            let guard = SetOnDrop(flag);
            async move {
                let _guard = guard;
                let _ = started.send(());
                std::future::pending::<Result<(), String>>().await
            }
        });
        runtime().unwrap().block_on(async {
            tokio::pin!(call);
            tokio::select! {
                _ = &mut call => panic!("the pending future finished"),
                _ = started_rx => {}
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

    #[test]
    fn cancellation_before_start_does_not_construct_the_call() {
        let constructed = Arc::new(AtomicBool::new(false));
        let flag = constructed.clone();
        let (cancel, cancelled) = oneshot::channel::<()>();
        drop(cancel);
        let result = runtime().unwrap().block_on(run_until_cancelled(
            move || {
                flag.store(true, Ordering::SeqCst);
                async { Ok::<(), String>(()) }
            },
            cancelled,
        ));
        assert!(result.is_none());
        assert!(!constructed.load(Ordering::SeqCst));
    }

    #[test]
    fn cancellation_while_waiting_for_a_slot_does_not_queue_a_call() {
        let slots = Arc::new(Semaphore::new(1));
        let constructed = Arc::new(AtomicBool::new(false));
        let flag = constructed.clone();
        runtime().unwrap().block_on(async {
            let slot = slots.clone().acquire_owned().await.unwrap();
            let mut call = Box::pin(spawn_io_with_slots(
                move || {
                    flag.store(true, Ordering::SeqCst);
                    async { Ok::<(), String>(()) }
                },
                slots,
            ));
            tokio::select! {
                biased;
                result = &mut call => panic!("call ran without a slot: {result:?}"),
                _ = tokio::task::yield_now() => {}
            }
            drop(call);
            drop(slot);
            tokio::task::yield_now().await;
        });
        assert!(!constructed.load(Ordering::SeqCst));
    }
}
