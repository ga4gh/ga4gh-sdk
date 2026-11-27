//! Python bindings of the TES client.
//!
//! `TES` is the client and `Task` addresses a task on the server; `AsyncTES` and
//! `AsyncTask` in `clients::tes_async` are their `asyncio` variants and use the same calls.
//! `TesTask` is a task to create or as returned, which keeps its JSON; `ListTasksParams`
//! and `ListTasksResponse` are the filters and results of `TES.list()`.
//!
//! The modules:
//!
//! - `client`: `TES`, the client.
//! - `task`: `Task`.
//! - `tes_task`: `TesTask`.
//! - `list`: `ListTasksParams`, `ListTasksResponse` and the calls that list tasks.
//! - `state`: `TesState`, a `str` enum defined in Python, and the parsing of state and view
//!   names.
//! - `wait`: the waits of `Task.wait()` and `AsyncTask.wait()`, and `WaitTimeoutError`.
//! - `calls`: the other TES calls that the sync and the async classes share (connect,
//!   create, get, status, cancel).
//! - `helpers`: the functions that only these modules share; they define no Python classes.
//!
//! The calls and functions that `clients::tes_async` and Rust extension crates use are
//! public and re-exported here. Helpers that other service bindings can use as they are go
//! to `crate::utils` instead, such as `utils::json` and `utils::convert::define_class`.

mod calls;
mod client;
mod helpers;
mod list;
mod state;
mod task;
mod tes_task;
mod wait;

pub use calls::{
    cancel_task, connect, connect_with, create_task, get_task, task_path, task_status,
    task_to_create,
};
pub use client::PyTES;
pub use list::{list_endpoint, list_tasks, PyListTasksParams, PyTesListTasksResponse};
pub use state::{parse_view, state_to_python, tes_state_type};
pub use task::PyTask;
pub use tes_task::PyTesTask;
pub use wait::{
    wait_end, wait_result, wait_settings, wait_task, wait_timeout_error_type, WaitEnd, WaitSettings,
};

use crate::clients::tes_async::{PyAsyncTES, PyAsyncTask};
use pyo3::prelude::*;

/// Registers the TES classes in a module: the sync and the async ones, the `PyTesTask`
/// alias of `TesTask`, the `TesState` enum and `WaitTimeoutError`.
pub fn register(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyTES>()?;
    m.add_class::<PyTesTask>()?;
    // Previous name of `TesTask`.
    m.add("PyTesTask", m.getattr("TesTask")?)?;
    m.add_class::<PyListTasksParams>()?;
    m.add_class::<PyTesListTasksResponse>()?;
    m.add_class::<PyTask>()?;
    m.add_class::<PyAsyncTES>()?;
    m.add_class::<PyAsyncTask>()?;
    m.add("TesState", tes_state_type(py)?)?;
    m.add("WaitTimeoutError", wait_timeout_error_type(py)?)?;
    Ok(())
}
