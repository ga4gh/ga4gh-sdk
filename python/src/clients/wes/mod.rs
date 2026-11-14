//! Python bindings of the WES client.
//!
//! The classes mirror those of TES: `WES` is the client, `Run` addresses a run on the server,
//! `RunRequest` is a run to submit, and `RunLog`, `RunSummary`, `RunListResponse`, `TaskLog`
//! and `TaskListResponse` are results that keep the JSON they were built from.
//!
//! The modules:
//!
//! - `client`: `WES`, the client.
//! - `run`: `Run`.
//! - `request`: `RunRequest` and the conversion of the `attachments` argument of `WES.run()`.
//! - `logs`: `RunLog` and `TaskLog`.
//! - `lists`: `RunSummary`, `RunListResponse` and `TaskListResponse`.
//! - `state`: `WesState`, a `str` enum defined in Python.
//! - `helpers`: the functions that these modules share; they define no Python classes.
//!
//! Helpers that other service bindings can use as they are go to `crate::utils` instead,
//! such as `utils::json` and `utils::validate`.

mod client;
mod helpers;
mod lists;
mod logs;
mod request;
mod run;
mod state;

pub use client::PyWES;
pub use lists::{PyRunListResponse, PyRunSummary, PyTaskListResponse};
pub use logs::{PyRunLog, PyTaskLog};
pub use request::PyRunRequest;
pub use run::PyRun;
pub use state::{state_to_python, wes_state_type};

use pyo3::prelude::*;

/// Registers the WES classes in a module.
pub fn register(py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyWES>()?;
    m.add_class::<PyRun>()?;
    m.add_class::<PyRunRequest>()?;
    m.add_class::<PyRunLog>()?;
    m.add_class::<PyRunSummary>()?;
    m.add_class::<PyRunListResponse>()?;
    m.add_class::<PyTaskLog>()?;
    m.add_class::<PyTaskListResponse>()?;
    m.add("WesState", wes_state_type(py)?)?;
    Ok(())
}
