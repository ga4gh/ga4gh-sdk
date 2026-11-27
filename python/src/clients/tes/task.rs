//! `Task`: a TES task on the server, addressed by its ID.

use super::calls::{cancel_task, task_status};
use super::state::state_to_python;
use super::wait::{wait_result, wait_settings, wait_task, SIGNAL_CHECK_INTERVAL};
use crate::utils::convert::check_extra_args;
use crate::utils::runtime::io;
use crate::PyTransport;
use ga4gh_sdk::clients::tes::Task;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

/// A TES task on the server, addressed by its ID.
#[pyclass(name = "Task", module = "ga4gh_sdk", subclass)]
pub struct PyTask {
    pub inner: Task,
}

impl PyTask {
    pub fn new(id: String, transport: &PyTransport) -> PyResult<Self> {
        Ok(PyTask {
            inner: Task::new(id, transport.inner.clone()),
        })
    }
}

#[pymethods]
impl PyTask {
    #[new]
    #[classmethod]
    #[pyo3(signature = (id, transport, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        id: String,
        transport: &PyTransport,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::new(id, transport)
    }

    /// The task's ID.
    #[getter]
    fn id(&self) -> PyResult<String> {
        Ok(self.inner.id.clone())
    }

    /// A transport with the task's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport.clone(),
        }
    }

    /// Gets the task's current state.
    ///
    /// Returns:
    ///     The state.
    ///
    /// Raises:
    ///     RuntimeError: The request failed or the response has no state.
    pub fn status(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let state = io(py, |rt| rt.block_on(task_status(&self.inner)))?;
        state_to_python(py, state)
    }

    /// Asks the server to cancel the task.
    ///
    /// Raises:
    ///     RuntimeError: The request failed.
    pub fn cancel(&self, py: Python<'_>) -> PyResult<()> {
        io(py, |rt| rt.block_on(cancel_task(&self.inner)))
    }

    /// Waits until the task ends; gives its final state, or the task in `view`.
    ///
    /// A task ends in `COMPLETE`, `EXECUTOR_ERROR`, `SYSTEM_ERROR`, `CANCELED` or
    /// `PREEMPTED`. Polls ask for the minimal view and start `poll_interval` seconds
    /// apart; the interval doubles after each poll. The wait releases the GIL; in the main
    /// thread, Ctrl-C stops it with `KeyboardInterrupt`. Nothing here cancels the task:
    /// call `cancel()` for that.
    ///
    /// Args:
    ///     timeout: The most seconds for the whole wait; `None` waits as long as it takes.
    ///     poll_interval: The seconds between the first polls.
    ///     max_interval: The longest interval between polls; by default 10 seconds, or
    ///         `poll_interval` if that's longer.
    ///     view: `"MINIMAL"`, `"BASIC"` or `"FULL"`, in any case, to get the ended task in
    ///         that view instead of its state.
    ///
    /// Returns:
    ///     The final state, or the ended task in `view`.
    ///
    /// Raises:
    ///     WaitTimeoutError: The time ran out; the task goes on running.
    ///     RuntimeError: A poll failed; polls are not retried.
    ///     ValueError: An argument is invalid, such as a negative `timeout`, a
    ///         `poll_interval` of 0 or a `max_interval` below `poll_interval`.
    ///
    /// Example:
    ///     ```python
    ///     try:
    ///         state = task.wait(timeout=3600)
    ///     except WaitTimeoutError as error:
    ///         print(error.id, "still", error.last_state)
    ///     ```
    #[pyo3(signature = (timeout=None, *, poll_interval=1.0, max_interval=None, view=None))]
    pub fn wait(
        &self,
        py: Python<'_>,
        timeout: Option<f64>,
        poll_interval: f64,
        max_interval: Option<f64>,
        view: Option<&str>,
    ) -> PyResult<Py<PyAny>> {
        let settings = wait_settings(timeout, poll_interval, max_interval, view)?;
        let cancellation = settings.options.operation.cancellation.clone();
        // Python runs signal handlers in the main thread only; other threads just wait.
        let threading = py.import("threading")?;
        // Idents, since `current_thread()` would register threads that Python didn't start.
        let main_thread = threading
            .call_method0("get_ident")?
            .eq(threading.call_method0("main_thread")?.getattr("ident")?)?;
        let (end, interrupt) = io(py, |rt| {
            let waiting = wait_task(&self.inner, settings);
            tokio::pin!(waiting);
            if !main_thread {
                return Ok((rt.block_on(waiting), None));
            }
            loop {
                let slice =
                    async { tokio::time::timeout(SIGNAL_CHECK_INTERVAL, &mut waiting).await };
                if let Ok(end) = rt.block_on(slice) {
                    return Ok((end, None));
                }
                // Outside `block_on`, so that handlers can make sync calls of their own.
                if let Err(error) = Python::attach(|py| py.check_signals()) {
                    // Stops the waiter, which then ends at once.
                    cancellation.cancel();
                    return Ok((rt.block_on(waiting), Some(error)));
                }
            }
        })?;
        match interrupt {
            Some(error) => Err(error),
            None => wait_result(py, &self.inner.id, end),
        }
    }
}
