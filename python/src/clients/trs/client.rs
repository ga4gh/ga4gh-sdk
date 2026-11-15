//! The TRS client and its Python methods.

use super::files::{PyFileWrapper, PyToolFile};
use super::helpers::{
    bounded_int, capability_override, parse_descriptor_type, require_ids, require_segment,
    source_name, support_name, to_raw, version_policy,
};
use super::page::PyToolPage;
use super::tools::{PyTool, PyToolVersion};
use super::DEFAULT_ZIP_LIMIT;
use crate::utils::convert::{check_extra_args, to_python};
use crate::utils::runtime::io;
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
use ga4gh_sdk::clients::trs::responses::{list_tools_page, next_tools_page};
use ga4gh_sdk::clients::trs::{ToolFilter, TrsOptions, TRS};
use ga4gh_sdk::compatibility::VersionPolicy;
use ga4gh_sdk::utils::transport::{BodyLimits, Transport};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple, PyType};

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
        Python::attach(|py| Self::connect(py, config, TrsOptions::default(), true))
    }

    /// Builds a TRS client with `options` for Rust extension crates, as `TRS(config,
    /// api_version=..., tag_filter=..., discovery=...)` does, releasing the GIL during
    /// discovery. Without discovery, `options.version` must be explicit.
    pub fn with_options(
        config: &PyConfiguration,
        options: TrsOptions,
        discovery: bool,
    ) -> PyResult<Self> {
        Python::attach(|py| Self::connect(py, config, options, discovery))
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
        let options = TrsOptions::default()
            .with_version(version_policy(api_version)?)
            .with_tag_filter(capability_override(tag_filter)?);
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
    fn discovered_service_info(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match self.inner.discovered_service_info() {
            Some(service) => to_python(py, &to_raw(service)?).map(Some),
            None => Ok(None),
        }
    }

    /// Reads the service info again, as a `dict`.
    pub fn service_info(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let service = io(py, |rt| {
            rt.block_on(self.inner.service_info())
                .map_err(|error| format!("Failed to get service info: {error}"))
        })?;
        to_python(py, &to_raw(&service)?)
    }

    /// Lists the tool classes of the registry, as `dict`s.
    pub fn list_tool_classes(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
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
        limit: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<PyToolPage> {
        let limit = limit
            .map(|limit| {
                bounded_int(
                    limit,
                    1,
                    i32::MAX.into(),
                    "limit must be between 1 and 2147483647",
                )
                .map(|limit| limit as i32)
            })
            .transpose()?;
        let filter = ToolFilter {
            id,
            alias,
            tool_class,
            descriptor_type: descriptor_type
                .as_deref()
                .map(parse_descriptor_type::<DescriptorType>)
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
        require_ids(id, version_id)?;
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
        require_ids(id, version_id)?;
        let descriptor_type: DescriptorTypeWithPlain = parse_descriptor_type(descriptor_type)?;
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
        require_ids(id, version_id)?;
        let descriptor_type: DescriptorTypeWithPlain = parse_descriptor_type(descriptor_type)?;
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
        require_ids(id, version_id)?;
        let descriptor_type: DescriptorType = parse_descriptor_type(descriptor_type)?;
        let files = io(py, |rt| {
            rt.block_on(self.inner.list_files(id, version_id, descriptor_type))
                .map_err(|error| format!("Failed to list files: {error}"))
        })?;
        files.into_iter().map(PyToolFile::new).collect()
    }

    /// Downloads all files of a tool version as one ZIP archive, as `bytes`.
    ///
    /// The archive is read into memory, at most `limit` bytes (`None`: 64 MiB); a larger
    /// archive, or a response that isn't a ZIP archive, raises `RuntimeError`.
    #[pyo3(signature = (id, version_id, descriptor_type, limit=None))]
    pub fn get_files_zip<'py>(
        &self,
        py: Python<'py>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
        limit: Option<&Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyBytes>> {
        require_ids(id, version_id)?;
        let descriptor_type: DescriptorType = parse_descriptor_type(descriptor_type)?;
        let limit = match limit {
            Some(limit) => bounded_int(limit, 1, i64::MAX, "limit must be positive")?,
            None => DEFAULT_ZIP_LIMIT,
        };
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let archive = io(py, |rt| {
            rt.block_on(
                self.inner
                    .get_files_zip(id, version_id, descriptor_type, limit),
            )
            .map_err(|error| format!("Failed to get files as ZIP: {error}"))
        })?;
        Ok(PyBytes::new(py, &archive))
    }

    /// Gets the container specifications, such as Dockerfiles, of a tool version, as
    /// `FileWrapper`s.
    pub fn get_containerfiles(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
    ) -> PyResult<Vec<PyFileWrapper>> {
        require_ids(id, version_id)?;
        let files = io(py, |rt| {
            rt.block_on(self.inner.get_containerfiles(id, version_id))
                .map_err(|error| format!("Failed to get container files: {error}"))
        })?;
        files.into_iter().map(PyFileWrapper::new).collect()
    }
}
