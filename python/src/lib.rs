#![allow(non_snake_case)] // allow the Python module to be named as GA4GH instead of ga4gh; should be the first line.
//! This module provides Python bindings for the GA4GH SDK using PyO3.
//! It exposes several key structs and their methods to Python, allowing for interaction with GA4GH services.
//!
//! # Classes
//!
//! - `Configuration`: the API root and settings for connecting to a GA4GH service.
//! - `ServiceInfo`: reads a service's service info.
//! - `Transport`: sends requests below the API root (GET, POST, PUT, DELETE, multipart).
//! - `TES`: a TES client; `Task`: a TES task addressed by its ID.
//! - `TesTask`: a task to create or as returned by the server, with its full JSON
//!   (`PyTesTask` is an alias).
//! - `TesState`: the state of a task, a `str` enum.
//! - `ListTasksParams` and `ListTasksResponse`: filters and results of `TES.list()`.
//!
//! All classes except the enums can be subclassed in Python. Rust crates can depend on this
//! crate (`rlib`) to build further bindings on these classes; their `inner` fields hold the
//! library types.
//!
//! # Example
//!
//! ```python
//! from GA4GH import Configuration, ListTasksParams, TES, TesState, Transport
//!
//! config = Configuration("http://localhost:8000/ga4gh/tes/v1")
//! tes = TES(config)
//! task = tes.create({"executors": [{"image": "alpine", "command": ["echo", "hello"]}]})
//! if task.status() is TesState.QUEUED:
//!     task.cancel()
//! print(tes.get("FULL", task.id).to_dict())
//! for task in tes.list(ListTasksParams(state=TesState.RUNNING)).tasks:
//!     print(task.id)
//!
//! response = Transport(config).get("tasks", {"view": "MINIMAL"})
//! ```
//!
//! # Notes
//!
//! - Synchronous network calls release the GIL and share a persistent Tokio runtime.
//! - Request failures raise `RuntimeError`; invalid arguments raise `ValueError`.

use ga4gh_sdk::clients::ServiceType;
use pyo3::prelude::*;

pub mod clients;
pub mod utils;
use crate::clients::serviceinfo::PyServiceInfo;
use crate::clients::tes::{
    tes_state_type, PyListTasksParams, PyTES, PyTask, PyTesListTasksResponse, PyTesTask,
};
use crate::utils::configuration::PyConfiguration;
use crate::utils::transport::PyTransport;

#[pyclass(name = "ServiceType", module = "GA4GH", eq, eq_int)]
#[derive(Clone, PartialEq, Eq)]
pub enum PyServiceType {
    TES,
    DRS,
    TRS,
    AAI,
}

impl From<PyServiceType> for ServiceType {
    fn from(py_service_type: PyServiceType) -> Self {
        match py_service_type {
            PyServiceType::TES => ServiceType::TES,
            PyServiceType::DRS => ServiceType::DRS,
            PyServiceType::TRS => ServiceType::TRS,
            PyServiceType::AAI => ServiceType::AAI,
        }
    }
}

#[pymodule]
fn GA4GH(py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyServiceType>()?;
    m.add_class::<PyConfiguration>()?;
    m.add_class::<PyServiceInfo>()?;
    m.add_class::<PyTransport>()?;
    m.add_class::<PyTES>()?;
    m.add_class::<PyTesTask>()?;
    // Previous name of `TesTask`.
    m.add("PyTesTask", m.getattr("TesTask")?)?;
    m.add_class::<PyListTasksParams>()?;
    m.add_class::<PyTesListTasksResponse>()?;
    m.add_class::<PyTask>()?;
    m.add("TesState", tes_state_type(py)?)?;
    Ok(())
}
