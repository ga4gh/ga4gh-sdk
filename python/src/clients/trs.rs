//! Python bindings of the TRS client.
//!
//! `TRS` wraps the Rust [`TRS`] client, with its version selection (TRS 2.0.1 or 2.0.2) and
//! its checks of IDs and tags. `Tool`, `ToolVersion`, `FileWrapper` and `ToolFile` are
//! results that keep the JSON of the Rust models, including fields that the TRS models
//! don't know; `ToolPage` is one page of `TRS.list_tools()`.

use crate::utils::convert::{check_extra_args, to_python};
use crate::utils::runtime::io;
use crate::PyConfiguration;
use crate::PyTransport;
use ga4gh_sdk::clients::trs::models::{
    DescriptorType, DescriptorTypeWithPlain, FileWrapper, Tool, ToolFile, ToolVersion,
};
use ga4gh_sdk::clients::trs::responses::{list_tools_page, next_tools_page, ToolPage};
use ga4gh_sdk::clients::trs::{ToolFilter, TrsOptions, TrsVersion, TRS};
use ga4gh_sdk::compatibility::{
    CapabilityOverride, CapabilitySupport, SelectionSource, VersionPolicy,
};
use ga4gh_sdk::utils::transport::{BodyLimits, Transport};
use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple, PyType};
use serde::Serialize;
use serde_json::Value;

/// The default size limit of `TRS.get_files_zip()`, as in the CLI: 64 MiB.
pub const DEFAULT_ZIP_LIMIT: usize = 64 * 1024 * 1024;

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

/// A model as JSON. The models serialize their unknown fields too.
fn to_raw(value: &impl Serialize) -> PyResult<Value> {
    serde_json::to_value(value)
        .map_err(|error| PyRuntimeError::new_err(format!("Invalid TRS response: {error}")))
}

/// A field of a JSON object as a Python object, or `None` if it is missing or `null`.
fn field(py: Python<'_>, raw: &Value, key: &str) -> PyResult<Option<PyObject>> {
    match raw.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => to_python(py, value).map(Some),
    }
}

/// Rejects IDs and paths that can't be one URL path segment: empty, `.` or `..`.
///
/// The Rust client rejects them too, but as a request failure; this raises `ValueError`
/// before any request. Other values are sent unchanged, as one percent-encoded segment.
fn require_segment(value: &str, what: &str) -> PyResult<()> {
    if value.is_empty() {
        return Err(PyValueError::new_err(format!("{what} must not be empty")));
    }
    if matches!(value, "." | "..") {
        return Err(PyValueError::new_err(format!(
            "{what} must not be {value:?}"
        )));
    }
    Ok(())
}

/// A descriptor type name, such as `CWL` or `PLAIN_CWL`, as given: names that TRS doesn't
/// define are sent unchanged.
fn descriptor_type_with_plain(name: &str) -> PyResult<DescriptorTypeWithPlain> {
    require_segment(name, "descriptor type")?;
    serde_json::from_value(Value::String(name.to_owned()))
        .map_err(|error| PyValueError::new_err(format!("Invalid descriptor type: {error}")))
}

/// A descriptor type name without the `PLAIN_` variants, such as `CWL`, as given.
fn descriptor_type(name: &str) -> PyResult<DescriptorType> {
    require_segment(name, "descriptor type")?;
    serde_json::from_value(Value::String(name.to_owned()))
        .map_err(|error| PyValueError::new_err(format!("Invalid descriptor type: {error}")))
}

fn version_policy(api_version: &str) -> PyResult<VersionPolicy<TrsVersion>> {
    match api_version {
        "auto" => Ok(VersionPolicy::Auto),
        "strict" => Ok(VersionPolicy::RequireKnown),
        "2.0.1" => Ok(VersionPolicy::Explicit(TrsVersion::V2_0_1)),
        "2.0.2" => Ok(VersionPolicy::Explicit(TrsVersion::V2_0_2)),
        other => Err(PyValueError::new_err(format!(
            "Invalid api_version {other:?}: use 'auto', 'strict', '2.0.1' or '2.0.2'"
        ))),
    }
}

fn capability_override(tag_filter: &str) -> PyResult<CapabilityOverride> {
    match tag_filter {
        "auto" => Ok(CapabilityOverride::Auto),
        "enabled" => Ok(CapabilityOverride::Enabled),
        "disabled" => Ok(CapabilityOverride::Disabled),
        other => Err(PyValueError::new_err(format!(
            "Invalid tag_filter {other:?}: use 'auto', 'enabled' or 'disabled'"
        ))),
    }
}

fn source_name(source: SelectionSource) -> &'static str {
    match source {
        SelectionSource::DeclaredVersion => "declared",
        SelectionSource::Caller => "caller",
        SelectionSource::Baseline => "baseline",
        _ => "unknown",
    }
}

fn support_name(support: CapabilitySupport) -> &'static str {
    match support {
        CapabilitySupport::Specified => "specified",
        CapabilitySupport::NotSpecified => "not_specified",
        CapabilitySupport::CallerEnabled => "caller_enabled",
        CapabilitySupport::CallerDisabled => "caller_disabled",
        _ => "unknown",
    }
}

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
    fn images(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
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
    fn descriptor_type_version(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
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
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
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
    fn toolclass(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
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
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let tool_name = slf.getattr("name")?.repr()?;
        Ok(format!("{name}(id={id}, name={tool_name})"))
    }
}

/// A descriptor, test or container file: its `content`, or a `url` to fetch it from.
///
/// For `PLAIN_*` descriptor types, the server sends the bare file; it comes as a
/// `FileWrapper` with only `content` set.
#[pyclass(name = "FileWrapper", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyFileWrapper {
    pub inner: FileWrapper,
    /// The file as JSON; derived from `inner`.
    pub raw: Value,
}

impl PyFileWrapper {
    /// Wraps a file of the Rust client, keeping its JSON.
    pub fn new(inner: FileWrapper) -> PyResult<Self> {
        Ok(Self {
            raw: to_raw(&inner)?,
            inner,
        })
    }
}

#[pymethods]
impl PyFileWrapper {
    #[getter]
    fn content(&self) -> Option<String> {
        self.inner.content.clone()
    }

    #[getter]
    fn url(&self) -> Option<String> {
        self.inner.url.clone()
    }

    #[getter]
    fn checksum(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "checksum")
    }

    /// The image or descriptor type of the file, such as `"Docker"` or `"CWL"`.
    #[getter]
    fn image_type(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "image_type")
    }

    /// The file as JSON text.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The file as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    /// Shows the URL and the length of the content, not the content itself.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let this = slf.borrow();
        let url = slf.getattr("url")?.repr()?;
        let content = match &this.inner.content {
            Some(content) => format!("<{} characters>", content.chars().count()),
            None => "None".to_owned(),
        };
        Ok(format!("{name}(url={url}, content={content})"))
    }
}

/// A file of a tool version, from `TRS.list_files()`: its path and file type.
#[pyclass(name = "ToolFile", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyToolFile {
    pub inner: ToolFile,
    /// The file as JSON; derived from `inner`.
    pub raw: Value,
}

impl PyToolFile {
    /// Wraps a file of the Rust client, keeping its JSON.
    pub fn new(inner: ToolFile) -> PyResult<Self> {
        Ok(Self {
            raw: to_raw(&inner)?,
            inner,
        })
    }
}

#[pymethods]
impl PyToolFile {
    #[getter]
    fn path(&self) -> Option<String> {
        self.inner.path.clone()
    }

    /// The file type, such as `"PRIMARY_DESCRIPTOR"`, as the server sent it.
    #[getter]
    fn file_type(&self) -> Option<String> {
        self.inner.file_type.as_ref().map(ToString::to_string)
    }

    #[getter]
    fn checksum(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        field(py, &self.raw, "checksum")
    }

    /// The file as JSON text.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The file as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let path = slf.getattr("path")?.repr()?;
        let file_type = slf.getattr("file_type")?.repr()?;
        Ok(format!("{name}(path={path}, file_type={file_type})"))
    }
}

/// One page of tools from `TRS.list_tools()` or `TRS.next_tools_page()`.
///
/// `next_page` is the URL of the next page that the server sent, or `None` on the last
/// page; pass it to `TRS.next_tools_page()`.
#[pyclass(name = "ToolPage", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyToolPage {
    pub tools: Vec<PyTool>,
    /// The response headers, by lowercase name; repeated headers are joined with `, `.
    /// Headers whose values aren't text are left out.
    pub headers: Vec<(String, String)>,
}

impl PyToolPage {
    /// Wraps a page of the Rust client.
    pub fn new(page: ToolPage) -> PyResult<Self> {
        let mut headers: Vec<(String, String)> = Vec::new();
        for name in page.headers.keys() {
            let values: Vec<&str> = page
                .headers
                .get_all(name)
                .iter()
                .filter_map(|value| value.to_str().ok())
                .collect();
            if !values.is_empty() {
                headers.push((name.as_str().to_owned(), values.join(", ")));
            }
        }
        let tools = page
            .tools
            .into_iter()
            .map(PyTool::new)
            .collect::<PyResult<_>>()?;
        Ok(Self { tools, headers })
    }

    fn header(&self, name: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .filter(|value| !value.is_empty())
    }
}

#[pymethods]
impl PyToolPage {
    #[getter]
    pub fn tools(&self) -> Vec<PyTool> {
        self.tools.clone()
    }

    /// The URL of the next page (`next_page` header), or `None`.
    #[getter]
    pub fn next_page(&self) -> Option<String> {
        self.header("next_page")
    }

    /// The URL of the last page (`last_page` header), or `None`.
    #[getter]
    pub fn last_page(&self) -> Option<String> {
        self.header("last_page")
    }

    /// The URL of this page (`self_link` header), or `None`.
    #[getter]
    pub fn self_link(&self) -> Option<String> {
        self.header("self_link")
    }

    /// The offset of this page (`current_offset` header), as the server sent it, or `None`.
    #[getter]
    pub fn current_offset(&self) -> Option<String> {
        self.header("current_offset")
    }

    /// The page size (`current_limit` header), as the server sent it, or `None`.
    #[getter]
    pub fn current_limit(&self) -> Option<String> {
        self.header("current_limit")
    }

    /// All response headers as a new `dict`, by lowercase name.
    pub fn headers<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new_bound(py);
        for (name, value) in &self.headers {
            dict.set_item(name, value)?;
        }
        Ok(dict)
    }

    fn __len__(&self) -> usize {
        self.tools.len()
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let this = slf.borrow();
        let next_page = this.next_page().is_some();
        Ok(format!(
            "{name}(tools={}, next_page={})",
            this.tools.len(),
            if next_page { "True" } else { "False" }
        ))
    }
}

/// A TRS client (TRS 2.0.1 and 2.0.2).
///
/// `TRS(config)` reads the service info and selects the TRS version to read responses with,
/// as the Rust client does. Subclass it to add endpoints, using `self.transport`. The public
/// `inner` preserves the Rust extension API.
#[pyclass(name = "TRS", module = "ga4gh_sdk", subclass)]
pub struct PyTRS {
    pub inner: TRS,
}

impl PyTRS {
    /// Builds a TRS client for Rust extension crates, with the default options, while
    /// releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Python::with_gil(|py| Self::connect(py, config, TrsOptions::default(), true))
    }

    fn connect(
        py: Python<'_>,
        config: &PyConfiguration,
        options: TrsOptions,
        discovery: bool,
    ) -> PyResult<Self> {
        if !discovery {
            let VersionPolicy::Explicit(version) = options.version else {
                return Err(PyValueError::new_err(
                    "discovery=False needs an explicit api_version, '2.0.1' or '2.0.2'",
                ));
            };
            let inner = TRS::without_discovery(Transport::new(&config.inner), version)
                .with_tag_filter_override(options.tag_filter);
            return Ok(Self { inner });
        }
        io(py, |rt| {
            rt.block_on(TRS::with_options(&config.inner, options))
                .map(|inner| Self { inner })
                .map_err(|error| format!("Failed to create TRS instance: {error}"))
        })
    }

    fn filter_tags(&self, tags: &[String]) -> PyResult<()> {
        if tags.is_empty() {
            return Ok(());
        }
        self.inner
            .compatibility()
            .tag_filter()
            .require("TRS", "tags")
            .map_err(|error| PyValueError::new_err(error.to_string()))
    }
}

#[pymethods]
impl PyTRS {
    /// `api_version` is `"auto"` (the version that the service info declares, else 2.0.1),
    /// `"strict"` (only a declared version that the SDK supports), `"2.0.1"` or `"2.0.2"`.
    /// `tag_filter` is `"auto"`, `"enabled"` or `"disabled"`: whether the server supports
    /// the `tags` filter, beyond what its TRS version says. `discovery=False` skips the
    /// service info and needs an explicit `api_version`.
    #[new]
    #[classmethod]
    #[pyo3(signature = (config, *args, api_version="auto", tag_filter="auto", discovery=true, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        config: &PyConfiguration,
        args: &Bound<'_, PyTuple>,
        api_version: &str,
        tag_filter: &str,
        discovery: bool,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        let options = TrsOptions {
            version: version_policy(api_version)?,
            tag_filter: capability_override(tag_filter)?,
        };
        Self::connect(cls.py(), config, options, discovery)
    }

    /// The client's configuration.
    #[getter]
    fn config(&self) -> PyConfiguration {
        PyConfiguration {
            inner: self.inner.configuration().clone(),
        }
    }

    /// A transport with the client's configuration, for further endpoints.
    #[getter]
    fn transport(&self) -> PyTransport {
        PyTransport {
            inner: self.inner.transport().clone(),
        }
    }

    /// The TRS version that responses are read with: `"2.0.1"` or `"2.0.2"`.
    #[getter]
    fn api_version(&self) -> String {
        self.inner.compatibility().selection().profile().to_string()
    }

    /// The TRS version that the service info declares, or `None` without discovery.
    #[getter]
    fn declared_version(&self) -> Option<String> {
        self.inner
            .compatibility()
            .selection()
            .declared_version()
            .map(str::to_owned)
    }

    /// Where `api_version` comes from: `"declared"` (the service info), `"caller"` (the
    /// `api_version` argument) or `"baseline"` (an unknown declared version; read as 2.0.1).
    #[getter]
    fn version_source(&self) -> &'static str {
        source_name(self.inner.compatibility().selection().source())
    }

    /// Whether `list_tools(tags=...)` may be sent: `"specified"` (TRS 2.0.2),
    /// `"caller_enabled"`, or not: `"not_specified"`, `"caller_disabled"`, `"unknown"`.
    #[getter]
    fn tag_filter(&self) -> &'static str {
        support_name(self.inner.compatibility().tag_filter())
    }

    /// The service info read when the client was built, as a `dict`, or `None` without
    /// discovery.
    #[getter]
    fn discovered_service_info(&self, py: Python<'_>) -> PyResult<Option<PyObject>> {
        match self.inner.discovered_service_info() {
            Some(service) => to_python(py, &to_raw(service)?).map(Some),
            None => Ok(None),
        }
    }

    /// Reads the service info again, as a `dict`.
    pub fn service_info(&self, py: Python<'_>) -> PyResult<PyObject> {
        let service = io(py, |rt| {
            rt.block_on(self.inner.service_info())
                .map_err(|error| format!("Failed to get service info: {error}"))
        })?;
        to_python(py, &to_raw(&service)?)
    }

    /// Lists the tool classes of the registry, as `dict`s.
    pub fn list_tool_classes(&self, py: Python<'_>) -> PyResult<Vec<PyObject>> {
        let classes = io(py, |rt| {
            rt.block_on(self.inner.list_tool_classes())
                .map_err(|error| format!("Failed to list tool classes: {error}"))
        })?;
        classes
            .iter()
            .map(|class| to_python(py, &to_raw(class)?))
            .collect()
    }

    /// Lists tools, one page at a time, as a `ToolPage`.
    ///
    /// All filters are optional. `tags` is a list of tags; the server returns tools with all
    /// of them. `offset` is the server's start of the page, as a `str`; follow
    /// `ToolPage.next_page` with `next_tools_page()` instead where the server sends it.
    #[pyo3(signature = (
        *, id=None, alias=None, tool_class=None, descriptor_type=None, registry=None,
        organization=None, name=None, toolname=None, description=None, author=None,
        checker=None, tags=None, offset=None, limit=None
    ))]
    #[allow(clippy::too_many_arguments)]
    pub fn list_tools(
        &self,
        py: Python<'_>,
        id: Option<String>,
        alias: Option<String>,
        tool_class: Option<String>,
        descriptor_type: Option<String>,
        registry: Option<String>,
        organization: Option<String>,
        name: Option<String>,
        toolname: Option<String>,
        description: Option<String>,
        author: Option<String>,
        checker: Option<bool>,
        tags: Option<Vec<String>>,
        offset: Option<String>,
        limit: Option<i32>,
    ) -> PyResult<PyToolPage> {
        let filter = ToolFilter {
            id,
            alias,
            tool_class,
            descriptor_type: descriptor_type
                .as_deref()
                .map(self::descriptor_type)
                .transpose()?,
            registry,
            organization,
            name,
            toolname,
            description,
            author,
            checker,
            tags: tags.unwrap_or_default(),
            offset,
            limit,
        };
        let params = filter
            .query()
            .map_err(|error| PyValueError::new_err(format!("Invalid filter: {error}")))?;
        self.filter_tags(&filter.tags)?;
        let page = io(py, |rt| {
            rt.block_on(list_tools_page(&self.inner, params, BodyLimits::default()))
                .map_err(|error| format!("Failed to list tools: {error}"))
        })?;
        PyToolPage::new(page)
    }

    /// Gets the page of tools at `next_page`, the URL that `ToolPage.next_page` gives.
    ///
    /// The URL must point to the tools endpoint of the client's API root; others raise
    /// `RuntimeError` before a request, so credentials never go to another server.
    pub fn next_tools_page(&self, py: Python<'_>, next_page: &str) -> PyResult<PyToolPage> {
        if next_page.is_empty() {
            return Err(PyValueError::new_err("next_page must not be empty"));
        }
        let page = io(py, |rt| {
            rt.block_on(next_tools_page(
                &self.inner,
                next_page,
                BodyLimits::default(),
            ))
            .map_err(|error| format!("Failed to list tools: {error}"))
        })?;
        PyToolPage::new(page)
    }

    /// Gets a tool, with its versions.
    pub fn get_tool(&self, py: Python<'_>, id: &str) -> PyResult<PyTool> {
        require_segment(id, "tool ID")?;
        let tool = io(py, |rt| {
            rt.block_on(self.inner.get_tool(id))
                .map_err(|error| format!("Failed to get tool: {error}"))
        })?;
        PyTool::new(tool)
    }

    /// Lists the versions of a tool.
    pub fn list_versions(&self, py: Python<'_>, id: &str) -> PyResult<Vec<PyToolVersion>> {
        require_segment(id, "tool ID")?;
        let versions = io(py, |rt| {
            rt.block_on(self.inner.list_versions(id))
                .map_err(|error| format!("Failed to list tool versions: {error}"))
        })?;
        versions.into_iter().map(PyToolVersion::new).collect()
    }

    /// Gets one version of a tool.
    pub fn get_version(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
    ) -> PyResult<PyToolVersion> {
        require_segment(id, "tool ID")?;
        require_segment(version_id, "version ID")?;
        let version = io(py, |rt| {
            rt.block_on(self.inner.get_version(id, version_id))
                .map_err(|error| format!("Failed to get tool version: {error}"))
        })?;
        PyToolVersion::new(version)
    }

    /// Gets the primary descriptor of a tool version, or the file at `relative_path` below
    /// it, as a `FileWrapper`.
    ///
    /// `descriptor_type` is a TRS descriptor type, such as `"CWL"`, or a `PLAIN_` type, such
    /// as `"PLAIN_CWL"`, for which the server sends the bare file.
    #[pyo3(signature = (id, version_id, descriptor_type, relative_path=None))]
    pub fn get_descriptor(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
        relative_path: Option<&str>,
    ) -> PyResult<PyFileWrapper> {
        require_segment(id, "tool ID")?;
        require_segment(version_id, "version ID")?;
        let descriptor_type = descriptor_type_with_plain(descriptor_type)?;
        if let Some(path) = relative_path {
            require_segment(path, "relative path")?;
        }
        let file = io(py, |rt| {
            rt.block_on(async {
                match relative_path {
                    Some(path) => {
                        self.inner
                            .get_descriptor_file(id, version_id, descriptor_type, path)
                            .await
                    }
                    None => {
                        self.inner
                            .get_descriptor(id, version_id, descriptor_type)
                            .await
                    }
                }
            })
            .map_err(|error| format!("Failed to get descriptor: {error}"))
        })?;
        PyFileWrapper::new(file)
    }

    /// Lists the test parameter files of a tool version, as `FileWrapper`s.
    ///
    /// For `PLAIN_` types, a bare test file comes as one `FileWrapper` with only `content`.
    pub fn list_tests(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
    ) -> PyResult<Vec<PyFileWrapper>> {
        require_segment(id, "tool ID")?;
        require_segment(version_id, "version ID")?;
        let descriptor_type = descriptor_type_with_plain(descriptor_type)?;
        let files = io(py, |rt| {
            rt.block_on(self.inner.list_tests(id, version_id, descriptor_type))
                .map_err(|error| format!("Failed to list tests: {error}"))
        })?;
        files.into_iter().map(PyFileWrapper::new).collect()
    }

    /// Lists the files of a tool version with their paths and file types, as `ToolFile`s.
    pub fn list_files(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
    ) -> PyResult<Vec<PyToolFile>> {
        require_segment(id, "tool ID")?;
        require_segment(version_id, "version ID")?;
        let descriptor_type = self::descriptor_type(descriptor_type)?;
        let files = io(py, |rt| {
            rt.block_on(self.inner.list_files(id, version_id, descriptor_type))
                .map_err(|error| format!("Failed to list files: {error}"))
        })?;
        files.into_iter().map(PyToolFile::new).collect()
    }

    /// Downloads all files of a tool version as one ZIP archive, as `bytes`.
    ///
    /// The archive is read into memory, at most `limit` bytes (64 MiB by default); a larger
    /// archive, or a response that isn't a ZIP archive, raises `RuntimeError`.
    #[pyo3(signature = (id, version_id, descriptor_type, limit=DEFAULT_ZIP_LIMIT))]
    pub fn get_files_zip<'py>(
        &self,
        py: Python<'py>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
        limit: usize,
    ) -> PyResult<Bound<'py, PyBytes>> {
        require_segment(id, "tool ID")?;
        require_segment(version_id, "version ID")?;
        let descriptor_type = self::descriptor_type(descriptor_type)?;
        if limit == 0 {
            return Err(PyValueError::new_err("limit must be positive"));
        }
        let archive = io(py, |rt| {
            rt.block_on(
                self.inner
                    .get_files_zip(id, version_id, descriptor_type, limit),
            )
            .map_err(|error| format!("Failed to get files as ZIP: {error}"))
        })?;
        Ok(PyBytes::new_bound(py, &archive))
    }

    /// Gets the container specifications, such as Dockerfiles, of a tool version, as
    /// `FileWrapper`s.
    pub fn get_containerfiles(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
    ) -> PyResult<Vec<PyFileWrapper>> {
        require_segment(id, "tool ID")?;
        require_segment(version_id, "version ID")?;
        let files = io(py, |rt| {
            rt.block_on(self.inner.get_containerfiles(id, version_id))
                .map_err(|error| format!("Failed to get container files: {error}"))
        })?;
        files.into_iter().map(PyFileWrapper::new).collect()
    }
}
