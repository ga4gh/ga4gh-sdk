//! DRS client python binding.
//!
//! The DRS service reads objects and access URLs, one at a time
//! or in bulk, and can wait for objects that the server is still preparing.
//! Consisted of:
//! - `DrsResolver` - turns hostname-based `drs://` URIs into access plans,
//! - `DrsObject` / `AccessURL` / `BulkAccessURL` - results that keep the JSON they were built from.
//!
//! - `client`: `DRS`, the client.
//! - `objects`: `DrsObject`, `AccessURL` and `BulkAccessURL`.
//! - `pending`: `PendingResolution`.
//! - `bulk`: `BulkPolicy`, `BulkOutcome` and `BulkReport`.
//! - `uri`: `DrsUri`.
//! - `resolver`: `DrsResolver` and `AccessPlan`.
//! - `helpers`: the functions that these modules share; they define no Python classes.

pub mod bulk;
pub mod client;
mod helpers;
pub mod objects;
pub mod pending;
pub mod resolver;
pub mod uri;

pub use bulk::{PyBulkOutcome, PyBulkPolicy, PyBulkReport};
pub use client::PyDRS;
pub use objects::{PyAccessURL, PyBulkAccessURL, PyDrsObject};
pub use pending::PyPendingResolution;
pub use resolver::{PyAccessPlan, PyDrsResolver};
pub use uri::PyDrsUri;

use pyo3::prelude::*;

/// Registers the DRS classes in a module.
pub fn register(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyDRS>()?;
    m.add_class::<PyDrsObject>()?;
    m.add_class::<PyAccessURL>()?;
    m.add_class::<PyBulkAccessURL>()?;
    m.add_class::<PyPendingResolution>()?;
    m.add_class::<PyBulkPolicy>()?;
    m.add_class::<PyBulkOutcome>()?;
    m.add_class::<PyBulkReport>()?;
    m.add_class::<PyDrsUri>()?;
    m.add_class::<PyDrsResolver>()?;
    m.add_class::<PyAccessPlan>()?;
    Ok(())
}
