//! Python bindings of the TRS client.
//!
//! `TRS` wraps the Rust [`TRS`] client, with its version selection (TRS 2.0.1 or 2.0.2) and
//! its checks of IDs and tags. `Tool`, `ToolVersion`, `FileWrapper` and `ToolFile` are
//! results that keep the JSON of the Rust models, including fields that the TRS models
//! don't know; `ToolPage` is one page of `TRS.list_tools()`.

//!
//! - `client`: `TRS`, the client.
//! - `tools`: `Tool` and `ToolVersion`.
//! - `files`: `FileWrapper` and `ToolFile`.
//! - `page`: `ToolPage` and its response headers.
//! - `helpers`: functions shared by this binding; no Python classes.

mod client;
mod files;
mod helpers;
mod page;
mod tools;

pub use client::PyTRS;
pub use files::{PyFileWrapper, PyToolFile};
pub use page::PyToolPage;
pub use tools::{PyTool, PyToolVersion};

use pyo3::prelude::*;

/// The default size limit of `TRS.get_files_zip()`, as in the CLI: 64 MiB.
pub const DEFAULT_ZIP_LIMIT: i64 = 64 * 1024 * 1024;

/// Registers the TRS classes in a module.
pub fn register(_py: Python<'_>, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyTRS>()?;
    m.add_class::<PyTool>()?;
    m.add_class::<PyToolVersion>()?;
    m.add_class::<PyFileWrapper>()?;
    m.add_class::<PyToolFile>()?;
    m.add_class::<PyToolPage>()?;
    Ok(())
}
