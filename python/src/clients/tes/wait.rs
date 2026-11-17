//! Waiting for a task to end: the settings and the result of `Task.wait()` and
//! `AsyncTask.wait()`, and `WaitTimeoutError`, a `TimeoutError` defined in Python.

use super::calls::task_path;
use super::helpers::invalid_response_text;
use super::state::{parse_view, read_state, state_to_python};
use super::tes_task::PyTesTask;
use crate::utils::convert::define_class;
use ga4gh_sdk::clients::tes::models::TesState;
use ga4gh_sdk::clients::tes::Task;
use ga4gh_sdk::clients::tes::TesView;
use ga4gh_sdk::clients::wait::{WaitOptions, WaitOutcome, WaitResult};
use ga4gh_sdk::utils::operation::OperationError;
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;
use std::future::Future;
use std::time::Duration;

/// Python source of the `WaitTimeoutError` exception.
const WAIT_TIMEOUT_ERROR_SOURCE: &str = r#"
class WaitTimeoutError(TimeoutError):
    """Raised when `Task.wait()` or `AsyncTask.wait()` runs out of time.

    `id` is the task's ID, and `last_state` its last polled state as a `TesState`, or
    `None` if no poll succeeded. The wait doesn't cancel the task, which may still be
    running (if the time ran out while getting an ended task in a view, `last_state` is
    its final state). Both are `None` on an error raised by other code.
    """

    __module__ = "ga4gh_sdk"

    id = None
    last_state = None
"#;

static WAIT_TIMEOUT_ERROR: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// The `WaitTimeoutError` Python exception, a `TimeoutError` (created once per interpreter).
pub fn wait_timeout_error_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    WAIT_TIMEOUT_ERROR
        .get_or_try_init(py, || {
            define_class(py, WAIT_TIMEOUT_ERROR_SOURCE, "WaitTimeoutError")
        })
        .map(|cls| cls.bind(py))
}

/// How long a sync wait blocks without the GIL before it checks for signals, such as Ctrl-C.
pub(super) const SIGNAL_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// The settings of a wait, from the arguments of `wait()`.
pub struct WaitSettings {
    /// The library waiter's options; their deadline counts from the call of `wait()`.
    pub options: WaitOptions,
    /// The view in which to give the ended task, or `None` to give its state.
    pub view: Option<TesView>,
}

/// A duration in seconds from Python; `name` names the argument in errors.
fn seconds(name: &str, value: f64) -> PyResult<Duration> {
    Duration::try_from_secs_f64(value).map_err(|_| {
        PyValueError::new_err(format!(
            "{name} must be a finite number of seconds, at least 0, not {value}"
        ))
    })
}

/// Checks the arguments of `wait()` and builds its settings; errors are `ValueError`s.
///
/// `timeout` is the time for the whole wait (`None` waits as long as it takes). Polls
/// start `poll_interval` apart, and the interval doubles up to `max_interval` (by default
/// 10 seconds, or `poll_interval` if that's longer).
pub fn wait_settings(
    timeout: Option<f64>,
    poll_interval: f64,
    max_interval: Option<f64>,
    view: Option<&str>,
) -> PyResult<WaitSettings> {
    let budget = timeout
        .map(|timeout| seconds("timeout", timeout))
        .transpose()?
        .unwrap_or(Duration::MAX);
    let poll_interval = seconds("poll_interval", poll_interval)?;
    if poll_interval.is_zero() {
        return Err(PyValueError::new_err("poll_interval must be more than 0"));
    }
    let view = view.map(parse_view).transpose()?;
    let mut options = WaitOptions::new(budget);
    let max_interval = match max_interval {
        Some(max_interval) => seconds("max_interval", max_interval)?,
        // The library's default (10 seconds), unless `poll_interval` is longer.
        None => options.max_interval.max(poll_interval),
    };
    if max_interval < poll_interval {
        return Err(PyValueError::new_err(
            "max_interval must be at least poll_interval",
        ));
    }
    options.poll_interval = poll_interval;
    options.max_interval = max_interval;
    Ok(WaitSettings { options, view })
}

/// How a wait ended, as owned, `Send` data; [`wait_result`] converts it for Python.
pub enum WaitEnd {
    /// The task reached this terminal state.
    State(TesState),
    /// The task ended; this is the task in the view asked for.
    Task(Box<PyTesTask>),
    /// The time ran out; the last polled state, if a poll succeeded.
    Timeout(Option<TesState>),
    /// The wait was stopped locally.
    Interrupted,
    /// A poll or the lookup of the ended task failed.
    Failed(String),
}

/// Waits for the task with the library's waiter until it ends, the time runs out or the
/// settings' operation is cancelled. Never cancels the task.
///
/// This is the sync `Task.wait()`. The future is not `Send`, so `AsyncTask.wait()` uses
/// `tes_async::wait_task_send` instead, which shares [`wait_end`].
pub async fn wait_task(task: &Task, settings: WaitSettings) -> WaitEnd {
    let WaitSettings { options, view } = settings;
    let result = task.wait(&options).await;
    wait_end(result, &options, view, |view| async move {
        let params = serde_json::json!({ "view": view });
        task.transport
            .get(&task_path(task), Some(params))
            .await
            .map_err(|error| error.to_string())
    })
    .await
}

/// Converts the library waiter's result into the end of the wait.
///
/// With a `view` other than the minimal one, `get_view` gets the ended task's JSON in that
/// view, within the deadline of `options`. The future is `Send` if `get_view`'s is.
pub async fn wait_end<G, Fut>(
    result: WaitResult,
    options: &WaitOptions,
    view: Option<TesView>,
    get_view: G,
) -> WaitEnd
where
    G: FnOnce(TesView) -> Fut,
    Fut: Future<Output = Result<String, String>>,
{
    let last_state = result.last_state.as_deref().map(read_state);
    match result.outcome {
        WaitOutcome::Complete
        | WaitOutcome::ExecutorError
        | WaitOutcome::SystemError
        | WaitOutcome::Canceled
        | WaitOutcome::Preempted => {}
        WaitOutcome::Deadline => return WaitEnd::Timeout(last_state),
        WaitOutcome::Interrupted => return WaitEnd::Interrupted,
        WaitOutcome::PollingFailed => {
            let error = result.error.unwrap_or_else(|| "unknown error".to_owned());
            return WaitEnd::Failed(format!("Failed to wait for task: {error}"));
        }
    }
    // The waiter ends only on states that TES 1.1 defines.
    let state = last_state.unwrap_or_default();
    let raw = match (view, result.last_response) {
        (None, _) => return WaitEnd::State(state),
        // The waiter polls the minimal view; no need to ask again.
        (Some(TesView::Minimal), Some(raw)) => Ok(raw),
        (Some(view), _) => match options.operation.run(get_view(view)).await {
            Ok(Ok(body)) => serde_json::from_str(&body).map_err(invalid_response_text),
            Ok(Err(error)) => Err(format!("Failed to get task: {error}")),
            Err(OperationError::Deadline) => return WaitEnd::Timeout(Some(state)),
            Err(OperationError::Cancelled) => return WaitEnd::Interrupted,
        },
    };
    match raw.and_then(|raw| PyTesTask::parse(raw).map_err(invalid_response_text)) {
        Ok(task) => WaitEnd::Task(Box::new(task)),
        Err(error) => WaitEnd::Failed(error),
    }
}

/// Converts the end of a wait for the task `id` into its Python result or exception.
pub fn wait_result(py: Python<'_>, id: &str, end: WaitEnd) -> PyResult<Py<PyAny>> {
    match end {
        WaitEnd::State(state) => state_to_python(py, state),
        WaitEnd::Task(task) => Ok(Py::new(py, *task)?.into_any()),
        WaitEnd::Timeout(last_state) => {
            let last_state = match last_state {
                Some(state) => state_to_python(py, state)?,
                None => py.None(),
            };
            let message = format!("Timed out waiting for task {id}");
            let error = wait_timeout_error_type(py)?.call1((message,))?;
            error.setattr("id", id)?;
            error.setattr("last_state", last_state)?;
            Err(PyErr::from_value(error))
        }
        // Not expected: the sync wait raises the signal handler's exception instead, and
        // the async one drops the waiter rather than cancelling it.
        WaitEnd::Interrupted => Err(PyRuntimeError::new_err("The wait was stopped locally")),
        WaitEnd::Failed(error) => Err(PyRuntimeError::new_err(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ga4gh_sdk::clients::wait::{RemoteHandle, ServiceKind};
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A waiter's result for a task that ended in `state`, with its minimal view.
    fn ended(state: &str) -> WaitResult {
        WaitResult {
            handle: RemoteHandle {
                service: ServiceKind::Tes,
                api_root: "http://localhost/".to_owned(),
                id: "task-1".to_owned(),
            },
            outcome: WaitOutcome::Complete,
            last_state: Some(state.to_owned()),
            last_response: Some(serde_json::json!({ "id": "task-1", "state": state })),
            details: None,
            error: None,
        }
    }

    /// Runs `future` on a runtime of its own.
    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn the_minimal_view_reuses_the_last_poll() {
        let options = WaitOptions::new(Duration::from_secs(5));
        let asked = AtomicBool::new(false);
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Minimal),
            |_| async {
                asked.store(true, Ordering::SeqCst);
                Ok(String::new())
            },
        ));
        assert!(matches!(end, WaitEnd::Task(task) if task.raw["id"] == "task-1"));
        assert!(!asked.load(Ordering::SeqCst));
    }

    #[test]
    fn other_views_give_the_task_from_the_view_request() {
        let options = WaitOptions::new(Duration::from_secs(5));
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Full),
            |view| async move {
                assert_eq!(view, TesView::Full);
                Ok(r#"{"id": "task-1", "state": "COMPLETE", "logs": []}"#.to_owned())
            },
        ));
        assert!(matches!(end, WaitEnd::Task(task) if task.raw.get("logs").is_some()));
    }

    #[test]
    fn a_failed_view_request_fails_the_wait() {
        let options = WaitOptions::new(Duration::from_secs(5));
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Full),
            |_| async { Err("HTTP 500".to_owned()) },
        ));
        assert!(matches!(end, WaitEnd::Failed(error) if error == "Failed to get task: HTTP 500"));
    }

    #[test]
    fn the_deadline_during_the_view_request_keeps_the_final_state() {
        let options = WaitOptions::new(Duration::from_millis(10));
        let end = block_on(wait_end(
            ended("EXECUTOR_ERROR"),
            &options,
            Some(TesView::Basic),
            |_| std::future::pending(),
        ));
        assert!(matches!(
            end,
            WaitEnd::Timeout(Some(TesState::ExecutorError))
        ));
    }

    #[test]
    fn cancellation_during_the_view_request_interrupts_the_wait() {
        let options = WaitOptions::new(Duration::from_secs(5));
        options.operation.cancellation.cancel();
        let end = block_on(wait_end(
            ended("COMPLETE"),
            &options,
            Some(TesView::Basic),
            |_| std::future::pending(),
        ));
        assert!(matches!(end, WaitEnd::Interrupted));
    }
}
