#![allow(non_snake_case)] // allow the Python module to be named as GA4GH instead of ga4gh; should be the first line.
//! This module provides Python bindings for the GA4GH SDK using PyO3.
//! It exposes several key structs and their methods to Python, allowing for interaction with GA4GH services.
//!
//! # Classes
//!
//! - `Task`: Represents a task in the GA4GH TES service.
//! - `TES`: Represents the GA4GH TES service.
//! - `Configuration`: Represents the configuration for connecting to GA4GH services.
//! - `ServiceInfo`: Provides information about the GA4GH service.
//! - `Transport`: Handles HTTP transport for GA4GH services.
//! - `PyTesTask`: Represents a TES task.
//! - `ListTasksParams`: Represents parameters for listing TES tasks.
//!
//! # Example
//!
//! ```python
//! from ga4gh import Configuration, TES, Task, ServiceInfo, Transport
//!
//! # Create a configuration
//! config = Configuration("http://example.com")
//!
//! # Create a TES instance
//! tes = TES(config)
//!
//! # Create a task
//! task = Task("task_id", transport)
//!
//! # Get task status
//! status = task.status()
//!
//! # Cancel a task
//! task.cancel()
//!
//! # Get service info
//! service_info = ServiceInfo(config)
//! info = service_info.get_service_info()
//!
//! # Perform HTTP GET request
//! transport = Transport(config)
//! response = transport.get("/endpoint", None)
//! ```
//!
//! # Notes
//!
//! - The `Runtime::new().unwrap()` calls are used to create a Tokio runtime for asynchronous operations.
//! - Error handling is done using `PyResult` and `pyo3::exceptions::PyRuntimeError`.
//! - The `#[pymodule]` attribute is used to define the Python module initialization function.

use ga4gh_sdk::clients::ServiceType;
use pyo3::prelude::*;

pub mod clients;
pub mod utils;
use crate::clients::serviceinfo::PyServiceInfo;
use crate::clients::tes::{PyTES, PyTask, PyTesTask};
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
fn GA4GH(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyServiceType>()?;
    m.add_class::<PyConfiguration>()?;
    m.add_class::<PyServiceInfo>()?;
    m.add_class::<PyTransport>()?;
    m.add_class::<PyTES>()?;
    m.add_class::<PyTesTask>()?;
    m.add_class::<PyTask>()?;
    Ok(())
}
