//! Tool and tool-version results.

use super::helpers::to_raw;
use crate::utils::convert::to_python;
use crate::utils::json::field;
use ga4gh_sdk::clients::trs::models::{Tool, ToolVersion};
use pyo3::prelude::*;
use serde_json::Value;

/// A tool version, as returned by the server.
///
/// `json()` and `to_dict()` return the version as JSON, including fields that the TRS models
/// don't know.
#[pyclass(name = "ToolVersion", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyToolVersion {
    pub inner: ToolVersion,
    /// The version as JSON; derived from `inner`.
    pub raw: Value,
}

impl PyToolVersion {
    /// Wraps a version of the Rust client, keeping its JSON.
    pub fn new(inner: ToolVersion) -> PyResult<Self> {
        Ok(Self {
            raw: to_raw(&inner)?,
            inner,
        })
    }
}

#[pymethods]
impl PyToolVersion {
    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn url(&self) -> String {
        self.inner.url.clone()
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name.clone()
    }

    #[getter]
    fn author(&self) -> Option<Vec<String>> {
        self.inner.author.clone()
    }

    /// The version's description (TRS 2.0.2), if it is a string.
    #[getter]
    fn description(&self) -> Option<String> {
        self.inner.description().map(str::to_owned)
    }

    #[getter]
    fn is_production(&self) -> Option<bool> {
        self.inner.is_production
    }

    #[getter]
    fn images(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "images")
    }

    /// The descriptor types of the version, such as `["CWL"]`.
    #[getter]
    fn descriptor_type(&self) -> Option<Vec<String>> {
        self.inner
            .descriptor_type
            .as_ref()
            .map(|types| types.iter().map(ToString::to_string).collect())
    }

    #[getter]
    fn descriptor_type_version(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "descriptor_type_version")
    }

    #[getter]
    fn containerfile(&self) -> Option<bool> {
        self.inner.containerfile
    }

    #[getter]
    fn meta_version(&self) -> Option<String> {
        self.inner.meta_version.clone()
    }

    #[getter]
    fn verified(&self) -> Option<bool> {
        self.inner.verified
    }

    #[getter]
    fn verified_source(&self) -> Option<Vec<String>> {
        self.inner.verified_source.clone()
    }

    #[getter]
    fn signed(&self) -> Option<bool> {
        self.inner.signed
    }

    #[getter]
    fn included_apps(&self) -> Option<Vec<String>> {
        self.inner.included_apps.clone()
    }

    /// The version as JSON text.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The version as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let version_name = slf.getattr("name")?.repr()?;
        Ok(format!("{name}(id={id}, name={version_name})"))
    }
}

/// A tool with its versions, as returned by the server.
///
/// `json()` and `to_dict()` return the tool as JSON, including fields that the TRS models
/// don't know.
#[pyclass(name = "Tool", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyTool {
    pub inner: Tool,
    /// The tool as JSON; derived from `inner`.
    pub raw: Value,
}

impl PyTool {
    /// Wraps a tool of the Rust client, keeping its JSON.
    pub fn new(inner: Tool) -> PyResult<Self> {
        Ok(Self {
            raw: to_raw(&inner)?,
            inner,
        })
    }
}

#[pymethods]
impl PyTool {
    #[getter]
    fn id(&self) -> String {
        self.inner.id.clone()
    }

    #[getter]
    fn url(&self) -> String {
        self.inner.url.clone()
    }

    #[getter]
    fn aliases(&self) -> Option<Vec<String>> {
        self.inner.aliases.clone()
    }

    #[getter]
    fn organization(&self) -> String {
        self.inner.organization.clone()
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner.name.clone()
    }

    /// The tool class as a `dict`, such as `{"id": "1", "name": "Workflow"}`.
    #[getter]
    fn toolclass(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "toolclass")
    }

    #[getter]
    fn description(&self) -> Option<String> {
        self.inner.description.clone()
    }

    #[getter]
    fn meta_version(&self) -> Option<String> {
        self.inner.meta_version.clone()
    }

    #[getter]
    fn has_checker(&self) -> Option<bool> {
        self.inner.has_checker
    }

    #[getter]
    fn checker_url(&self) -> Option<String> {
        self.inner.checker_url.clone()
    }

    /// The tool's versions, as `ToolVersion`s.
    #[getter]
    fn versions(&self) -> PyResult<Vec<PyToolVersion>> {
        self.inner
            .versions
            .iter()
            .cloned()
            .map(PyToolVersion::new)
            .collect()
    }

    /// The tool as JSON text.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The tool as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let tool_name = slf.getattr("name")?.repr()?;
        Ok(format!("{name}(id={id}, name={tool_name})"))
    }
}
