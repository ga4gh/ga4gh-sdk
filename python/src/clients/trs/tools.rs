//! Tool and tool-version results.

use super::helpers::invalid_response;
use crate::utils::convert::to_python;
use crate::utils::json::{field, view_accessors};
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
    /// The version in the TRS model.
    #[deprecated(note = "read `inner()`; build a changed version from a model with `new`")]
    pub inner: ToolVersion,
    /// The version as JSON; derived from `inner`.
    #[deprecated(note = "read `raw()`; build a changed version from a model with `new`")]
    pub raw: Value,
}

view_accessors!(PyToolVersion, ToolVersion);

impl PyToolVersion {
    #[allow(deprecated)] // Builds the fields that the accessors read.
    fn build(inner: ToolVersion) -> serde_json::Result<Self> {
        Ok(Self {
            raw: serde_json::to_value(&inner)?,
            inner,
        })
    }

    /// Wraps a version of the Rust client, keeping its JSON.
    pub fn new(inner: ToolVersion) -> PyResult<Self> {
        Self::build(inner).map_err(invalid_response)
    }
}

#[pymethods]
impl PyToolVersion {
    #[getter]
    fn id(&self) -> String {
        self.inner().id.clone()
    }

    #[getter]
    fn url(&self) -> String {
        self.inner().url.clone()
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner().name.clone()
    }

    #[getter]
    fn author(&self) -> Option<Vec<String>> {
        self.inner().author.clone()
    }

    /// The version's description (TRS 2.0.2), if it is a string.
    #[getter]
    fn description(&self) -> Option<String> {
        self.inner().description().map(str::to_owned)
    }

    #[getter]
    fn is_production(&self) -> Option<bool> {
        self.inner().is_production
    }

    #[getter]
    fn images(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "images")
    }

    /// The descriptor types of the version, such as `["CWL"]`.
    #[getter]
    fn descriptor_type(&self) -> Option<Vec<String>> {
        self.inner()
            .descriptor_type
            .as_ref()
            .map(|types| types.iter().map(ToString::to_string).collect())
    }

    #[getter]
    fn descriptor_type_version(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "descriptor_type_version")
    }

    #[getter]
    fn containerfile(&self) -> Option<bool> {
        self.inner().containerfile
    }

    #[getter]
    fn meta_version(&self) -> Option<String> {
        self.inner().meta_version.clone()
    }

    #[getter]
    fn verified(&self) -> Option<bool> {
        self.inner().verified
    }

    #[getter]
    fn verified_source(&self) -> Option<Vec<String>> {
        self.inner().verified_source.clone()
    }

    #[getter]
    fn signed(&self) -> Option<bool> {
        self.inner().signed
    }

    #[getter]
    fn included_apps(&self) -> Option<Vec<String>> {
        self.inner().included_apps.clone()
    }

    /// The version as JSON text.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// The version as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
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
    /// The tool in the TRS model.
    #[deprecated(note = "read `inner()`; build a changed tool from a model with `new`")]
    pub inner: Tool,
    /// The tool as JSON; derived from `inner`.
    #[deprecated(note = "read `raw()`; build a changed tool from a model with `new`")]
    pub raw: Value,
}

view_accessors!(PyTool, Tool);

impl PyTool {
    #[allow(deprecated)] // Builds the fields that the accessors read.
    fn build(inner: Tool) -> serde_json::Result<Self> {
        Ok(Self {
            raw: serde_json::to_value(&inner)?,
            inner,
        })
    }

    /// Wraps a tool of the Rust client, keeping its JSON.
    pub fn new(inner: Tool) -> PyResult<Self> {
        Self::build(inner).map_err(invalid_response)
    }
}

#[pymethods]
impl PyTool {
    #[getter]
    fn id(&self) -> String {
        self.inner().id.clone()
    }

    #[getter]
    fn url(&self) -> String {
        self.inner().url.clone()
    }

    #[getter]
    fn aliases(&self) -> Option<Vec<String>> {
        self.inner().aliases.clone()
    }

    #[getter]
    fn organization(&self) -> String {
        self.inner().organization.clone()
    }

    #[getter]
    fn name(&self) -> Option<String> {
        self.inner().name.clone()
    }

    /// The tool class as a `dict`, such as `{"id": "1", "name": "Workflow"}`.
    #[getter]
    fn toolclass(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "toolclass")
    }

    #[getter]
    fn description(&self) -> Option<String> {
        self.inner().description.clone()
    }

    #[getter]
    fn meta_version(&self) -> Option<String> {
        self.inner().meta_version.clone()
    }

    #[getter]
    fn has_checker(&self) -> Option<bool> {
        self.inner().has_checker
    }

    #[getter]
    fn checker_url(&self) -> Option<String> {
        self.inner().checker_url.clone()
    }

    /// The tool's versions, as `ToolVersion`s.
    #[getter]
    fn versions(&self) -> PyResult<Vec<PyToolVersion>> {
        self.inner()
            .versions
            .iter()
            .cloned()
            .map(PyToolVersion::new)
            .collect()
    }

    /// The tool as JSON text.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// The tool as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let tool_name = slf.getattr("name")?.repr()?;
        Ok(format!("{name}(id={id}, name={tool_name})"))
    }
}

// The tests call `build`, which `new` shares (see `utils::json::tests`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::json::tests::assert_describes;
    use serde_json::json;

    /// `raw` parsed into a model: the SDK's TRS client gives the bindings models.
    fn model<T: serde::de::DeserializeOwned>(raw: &Value) -> T {
        serde_json::from_value(raw.clone()).unwrap()
    }

    #[test]
    fn a_version_describes_its_json() {
        let version = json!({"id": "v1", "url": "https://example.org/v1", "x_lab": "kept"});
        let given =
            json!({"id": "v1", "url": "https://example.org/v1", "name": null, "x_lab": "kept"});
        let view = PyToolVersion::build(model(&given)).unwrap();
        // The models serialize unknown fields, and leave out the ones that are `null`.
        assert_describes(view.inner(), view.raw(), &view.json(), &version);
        assert_eq!(view.inner().name, None);
        assert_eq!(view.inner().extra["x_lab"], "kept");
    }

    #[test]
    fn a_tool_describes_its_json() {
        let tool = json!({
            "id": "t1",
            "url": "https://example.org/t1",
            "organization": "example",
            "toolclass": {"id": "c1", "name": "CommandLineTool"},
            "versions": [{"id": "v1", "url": "https://example.org/v1", "x_version": 1}],
            "x_tool": {"kept": true}
        });
        let mut given = tool.clone();
        given["description"] = Value::Null;
        let view = PyTool::build(model(&given)).unwrap();
        assert_describes(view.inner(), view.raw(), &view.json(), &tool);
        assert_eq!(view.inner().description, None);
        assert_eq!(view.inner().extra["x_tool"], json!({"kept": true}));
        let version = PyToolVersion::build(view.inner().versions[0].clone()).unwrap();
        assert_eq!(version.raw(), &tool["versions"][0]);
    }
}
