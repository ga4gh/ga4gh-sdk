//! This module provides Python bindings for the GA4GH SDK using PyO3.
//! It exposes several key structs and their methods to Python, allowing for interaction with GA4GH services.
//!
//! # Classes
//!
//! - `Configuration`: the API root and settings for connecting to a GA4GH service.
//! - `ServiceInfo`: reads a service's service info.
//! - `Transport`: sends requests below the API root (GET, POST, PUT, DELETE, multipart).
//! - `DRS`: a DRS client for objects and access URLs, one at a time or in bulk;
//!   `DrsResolver` turns `drs://` URIs into `AccessPlan`s. `DrsObject`, `AccessURL` and
//!   `BulkAccessURL` are results with their full JSON; `PendingResolution`, `BulkPolicy`,
//!   `BulkReport`, `BulkOutcome` and `DrsUri` go with them.
//! - `TES`: a TES client; `Task`: a TES task addressed by its ID.
//! - `AsyncTES` and `AsyncTask`: the same for `asyncio`; their methods return awaitables.
//! - `TesTask`: a task to create or as returned by the server, with its full JSON
//!   (`PyTesTask` is an alias).
//! - `TesState`: the state of a task, a `str` enum.
//! - `WaitTimeoutError`: raised when `Task.wait()` or `AsyncTask.wait()` runs out of time.
//! - `ListTasksParams` and `ListTasksResponse`: filters and results of `TES.list()`.
//! - `WES`: a WES client; `Run`: a WES run addressed by its ID; `RunRequest`: a run to
//!   start; `WesState`: the state of a run, a `str` enum. `RunLog`, `RunSummary`,
//!   `RunListResponse`, `TaskLog` and `TaskListResponse` are results with their full JSON.
//!
//! All classes except the enums can be subclassed in Python. Rust crates can depend on this
//! crate (`rlib`) to build further bindings on these classes; their `inner` fields hold the
//! library types.
//!
//! # Example
//!
//! ```python
//! from ga4gh_sdk import Configuration, ListTasksParams, TES, TesState, Transport
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
//! - Synchronous network calls release the GIL and share a persistent Tokio runtime. The
//!   async classes run their requests on the same runtime.
//! - Request failures raise `RuntimeError`; invalid arguments raise `ValueError`. A wait
//!   that runs out of time raises `WaitTimeoutError`, a `TimeoutError`.

use ::ga4gh_sdk::clients::ServiceType;
use pyo3::prelude::*;

pub mod clients;
pub mod utils;
use crate::clients::serviceinfo::PyServiceInfo;
use crate::clients::tes::{
    tes_state_type, wait_timeout_error_type, PyListTasksParams, PyTES, PyTask,
    PyTesListTasksResponse, PyTesTask,
};
use crate::clients::tes_async::{PyAsyncTES, PyAsyncTask};
use crate::utils::configuration::PyConfiguration;
use crate::utils::transport::PyTransport;

/// A built-in section of the configuration file, for `Configuration.from_file()`.
/// Other sections are named by a `str`.
#[pyclass(name = "ServiceType", module = "ga4gh_sdk", eq, eq_int)]
#[derive(Clone, PartialEq, Eq)]
pub enum PyServiceType {
    TES,
    DRS,
    TRS,
    AAI,
    WES,
    REGISTRY,
}

impl From<PyServiceType> for ServiceType {
    fn from(py_service_type: PyServiceType) -> Self {
        match py_service_type {
            PyServiceType::TES => ServiceType::TES,
            PyServiceType::DRS => ServiceType::DRS,
            PyServiceType::TRS => ServiceType::TRS,
            PyServiceType::AAI => ServiceType::AAI,
            PyServiceType::WES => ServiceType::WES,
            PyServiceType::REGISTRY => ServiceType::REGISTRY,
        }
    }
}

/// A section of the configuration file: a `ServiceType`, or the name of any other
/// section as a `str`, which becomes `ServiceType::Custom`.
pub enum ServiceSection {
    BuiltIn(PyServiceType),
    Custom(String),
}

impl<'py> FromPyObject<'py> for ServiceSection {
    fn extract_bound(value: &Bound<'py, PyAny>) -> PyResult<Self> {
        if let Ok(service_type) = value.extract::<PyServiceType>() {
            return Ok(ServiceSection::BuiltIn(service_type));
        }
        if let Ok(name) = value.extract::<String>() {
            return Ok(ServiceSection::Custom(name));
        }
        Err(pyo3::exceptions::PyTypeError::new_err(format!(
            "service_type must be a ServiceType or a str, not {}",
            value.get_type().qualname()?
        )))
    }
}

impl From<ServiceSection> for ServiceType {
    fn from(section: ServiceSection) -> Self {
        match section {
            ServiceSection::BuiltIn(service_type) => service_type.into(),
            ServiceSection::Custom(name) => ServiceType::Custom(name),
        }
    }
}

#[pymodule]
fn ga4gh_sdk(py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyServiceType>()?;
    m.add_class::<PyConfiguration>()?;
    m.add_class::<PyServiceInfo>()?;
    m.add_class::<PyTransport>()?;
    clients::wes::register(py, m)?;
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
    clients::drs::register(py, m)?;
    Ok(())
}
