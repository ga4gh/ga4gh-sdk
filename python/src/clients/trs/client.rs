//! The TRS client and its Python methods.

use super::files::{PyFileWrapper, PyToolFile};
use super::helpers::{bounded_int, parse_descriptor_type, require_ids, to_raw};
use super::page::PyToolPage;
use super::tools::{PyTool, PyToolVersion};
use super::versions::{
    build_client, capability_override, require_tag_filter, source_name, support_name,
    version_policy,
};
use super::DEFAULT_ZIP_LIMIT;
use crate::utils::convert::{check_extra_args, to_python};
use crate::utils::runtime::request;
use crate::utils::validate::require_id;
use crate::{PyConfiguration, PyTransport};
use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
use ga4gh_sdk::clients::trs::responses::{list_tools_page, next_tools_page};
use ga4gh_sdk::clients::trs::{ToolFilter, TrsOptions, TRS};
use ga4gh_sdk::utils::transport::BodyLimits;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyTuple, PyType};

/// A TRS client (TRS 2.0.1 and 2.0.2).
///
/// `api_version` is `"auto"`, `"strict"`, `"2.0.1"` or `"2.0.2"`; `tag_filter` is
/// `"auto"`, `"enabled"` or `"disabled"`. `discovery=False` skips the service info and
/// needs an explicit `api_version`.
///
/// IDs, version IDs and relative paths are sent unchanged, as one percent-encoded URL path
/// segment each, so IDs such as Dockstore's `#workflow/github.com/org/repo` work. Empty
/// values, `.` and `..` raise `ValueError` before a request. Failed requests, such as for
/// a missing tool (404), and responses that don't fit the TRS models raise `RuntimeError`.
#[pyclass(name = "TRS", module = "ga4gh_sdk", subclass)]
pub struct PyTRS {
    /// The library client, for Rust extension crates.
    pub inner: TRS,
}

impl PyTRS {
    /// Builds a TRS client for Rust extension crates, with the default options, while
    /// releasing the GIL during discovery.
    pub fn new(config: &PyConfiguration) -> PyResult<Self> {
        Self::with_options(config, TrsOptions::default(), true)
    }

    /// Builds a TRS client with `options` for Rust extension crates, as `TRS(config,
    /// api_version=..., tag_filter=..., discovery=...)` does, releasing the GIL during
    /// discovery. Without discovery, `options.version` must be explicit.
    pub fn with_options(
        config: &PyConfiguration,
        options: TrsOptions,
        discovery: bool,
    ) -> PyResult<Self> {
        Python::attach(|py| {
            build_client(py, config, options, discovery).map(|inner| Self { inner })
        })
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
        build_client(cls.py(), config, options, discovery).map(|inner| Self { inner })
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

    /// Where `api_version` comes from: `"declared"`, `"caller"` or `"baseline"`.
    ///
    /// `"baseline"` means that the service declared an unknown version, or none, and is
    /// read as 2.0.1 without a check that this works.
    #[getter]
    fn version_source(&self) -> &'static str {
        source_name(self.inner.compatibility().selection().source())
    }

    /// Whether `list_tools(tags=...)` may be sent.
    ///
    /// It may if this is `"specified"` (TRS 2.0.2) or `"caller_enabled"`, and may not if
    /// it is `"not_specified"`, `"caller_disabled"` or `"unknown"`.
    #[getter]
    fn tag_filter(&self) -> &'static str {
        support_name(self.inner.compatibility().tag_filter())
    }

    /// The service info read when the client was built, or `None` without discovery.
    #[getter]
    fn discovered_service_info(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        match self.inner.discovered_service_info() {
            Some(service) => to_python(py, &to_raw(service)?).map(Some),
            None => Ok(None),
        }
    }

    /// Reads the service info again.
    ///
    /// Returns:
    ///     The service info as a `dict`.
    ///
    /// Raises:
    ///     RuntimeError: The request failed.
    pub fn service_info(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        let service = request(py, "get service info", || self.inner.service_info())?;
        to_python(py, &to_raw(&service)?)
    }

    /// Lists the registry's tool classes.
    ///
    /// Returns:
    ///     The tool classes as `dict`s, such as `{"id": "1", "name": "Workflow"}`.
    ///
    /// Raises:
    ///     RuntimeError: The request failed.
    pub fn list_tool_classes(&self, py: Python<'_>) -> PyResult<Vec<Py<PyAny>>> {
        let classes = request(py, "list tool classes", || self.inner.list_tool_classes())?;
        classes
            .iter()
            .map(|class| to_python(py, &to_raw(class)?))
            .collect()
    }

    /// Lists tools, one page at a time; unset filters are not sent.
    ///
    /// Requested filters are never dropped silently.
    ///
    /// Args:
    ///     id: A tool ID.
    ///     alias: An alias of a tool.
    ///     tool_class: A tool class name, such as `"Workflow"`.
    ///     descriptor_type: A descriptor type, such as `"CWL"`.
    ///     registry: The image registry of the tools' containers.
    ///     organization: The publishing organization.
    ///     name: The image name.
    ///     toolname: The tool's name.
    ///     description: Text in the tool's description.
    ///     author: An author.
    ///     checker: Whether to list checker tools.
    ///     tags: Tags that the tools have (TRS 2.0.2); sent as one comma-separated value.
    ///     offset: Where the page starts, as the server's paging gives it.
    ///     limit: The most tools on the page, between 1 and 2147483647.
    ///
    /// Returns:
    ///     The first or requested page.
    ///
    /// Raises:
    ///     ValueError: `limit` is out of range, `descriptor_type` is empty, `.` or `..`,
    ///         a tag is empty or contains a comma, or `tags` is given while
    ///         `tag_filter` doesn't allow it.
    ///     RuntimeError: The request failed.
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
        require_tag_filter(&self.inner, &filter.tags)?;
        let page = request(py, "list tools", || {
            list_tools_page(&self.inner, params, BodyLimits::default())
        })?;
        PyToolPage::new(page)
    }

    /// Gets the page at `ToolPage.next_page`.
    ///
    /// Only URLs of the tools endpoint of the client's API root are followed, so
    /// credentials never go to another server.
    ///
    /// Args:
    ///     next_page: The `next_page` URL of the previous page.
    ///
    /// Returns:
    ///     The next page.
    ///
    /// Raises:
    ///     ValueError: `next_page` is empty.
    ///     RuntimeError: The URL is not at the client's tools endpoint (before a request),
    ///         or the request failed.
    pub fn next_tools_page(&self, py: Python<'_>, next_page: &str) -> PyResult<PyToolPage> {
        if next_page.is_empty() {
            return Err(PyValueError::new_err("next_page must not be empty"));
        }
        let page = request(py, "list tools", || {
            next_tools_page(&self.inner, next_page, BodyLimits::default())
        })?;
        PyToolPage::new(page)
    }

    /// Gets a tool, with its versions.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///
    /// Returns:
    ///     The tool.
    ///
    /// Raises:
    ///     ValueError: `id` is empty, `.` or `..`.
    ///     RuntimeError: The request failed, for example because the tool doesn't exist.
    pub fn get_tool(&self, py: Python<'_>, id: &str) -> PyResult<PyTool> {
        require_id(id, "tool ID")?;
        let tool = request(py, "get tool", || self.inner.get_tool(id))?;
        PyTool::new(tool)
    }

    /// Lists the versions of a tool.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///
    /// Returns:
    ///     The versions.
    ///
    /// Raises:
    ///     ValueError: `id` is empty, `.` or `..`.
    ///     RuntimeError: The request failed.
    pub fn list_versions(&self, py: Python<'_>, id: &str) -> PyResult<Vec<PyToolVersion>> {
        require_id(id, "tool ID")?;
        let versions = request(py, "list tool versions", || self.inner.list_versions(id))?;
        versions.into_iter().map(PyToolVersion::new).collect()
    }

    /// Gets one version of a tool.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///     version_id: The version's ID; some registries, such as Dockstore, take its name.
    ///
    /// Returns:
    ///     The version.
    ///
    /// Raises:
    ///     ValueError: An ID is empty, `.` or `..`.
    ///     RuntimeError: The request failed.
    pub fn get_version(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
    ) -> PyResult<PyToolVersion> {
        require_ids(id, version_id)?;
        let version = request(py, "get tool version", || {
            self.inner.get_version(id, version_id)
        })?;
        PyToolVersion::new(version)
    }

    /// Gets the primary descriptor, or the file at `relative_path` below it.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///     version_id: The version's ID.
    ///     descriptor_type: A TRS descriptor type, such as `"CWL"`, `"WDL"`, `"NFL"`,
    ///         `"GALAXY"` or `"SMK"`; or `"PLAIN_CWL"` and so on for the bare file, which
    ///         comes as a `FileWrapper` with only `content`. Types are sent as given.
    ///     relative_path: The path of another file, relative to the primary descriptor.
    ///
    /// Returns:
    ///     The file.
    ///
    /// Raises:
    ///     ValueError: An ID, `descriptor_type` or `relative_path` is empty, `.` or `..`.
    ///     RuntimeError: The request failed.
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
            require_id(path, "relative path")?;
        }
        let file = request(py, "get descriptor", || async move {
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
        })?;
        PyFileWrapper::new(file)
    }

    /// Lists the test parameter files of a tool version.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///     version_id: The version's ID.
    ///     descriptor_type: A descriptor type, such as `"CWL"`; for a `PLAIN_` type, a bare
    ///         test file comes as one `FileWrapper` with only `content`.
    ///
    /// Returns:
    ///     The test files.
    ///
    /// Raises:
    ///     ValueError: An ID or `descriptor_type` is empty, `.` or `..`.
    ///     RuntimeError: The request failed.
    pub fn list_tests(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
    ) -> PyResult<Vec<PyFileWrapper>> {
        require_ids(id, version_id)?;
        let descriptor_type: DescriptorTypeWithPlain = parse_descriptor_type(descriptor_type)?;
        let files = request(py, "list tests", || {
            self.inner.list_tests(id, version_id, descriptor_type)
        })?;
        files.into_iter().map(PyFileWrapper::new).collect()
    }

    /// Lists the files of a tool version, with their paths and file types.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///     version_id: The version's ID.
    ///     descriptor_type: A descriptor type without `PLAIN_`, such as `"CWL"`.
    ///
    /// Returns:
    ///     The files.
    ///
    /// Raises:
    ///     ValueError: An ID or `descriptor_type` is empty, `.` or `..`.
    ///     RuntimeError: The request failed.
    pub fn list_files(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
        descriptor_type: &str,
    ) -> PyResult<Vec<PyToolFile>> {
        require_ids(id, version_id)?;
        let descriptor_type: DescriptorType = parse_descriptor_type(descriptor_type)?;
        let files = request(py, "list files", || {
            self.inner.list_files(id, version_id, descriptor_type)
        })?;
        files.into_iter().map(PyToolFile::new).collect()
    }

    /// Downloads the version's files as one ZIP archive of at most `limit` bytes.
    ///
    /// The archive is read into memory; it isn't extracted or checked further.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///     version_id: The version's ID.
    ///     descriptor_type: A descriptor type without `PLAIN_`, such as `"CWL"`.
    ///     limit: The most bytes to read; `None` means 64 MiB (67108864 bytes).
    ///
    /// Returns:
    ///     The archive.
    ///
    /// Raises:
    ///     ValueError: An ID or `descriptor_type` is empty, `.` or `..`, or `limit` is
    ///         below 1.
    ///     RuntimeError: The request failed, the archive is larger than `limit`, or the
    ///         response isn't a ZIP archive.
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
        let archive = request(py, "get files as ZIP", || {
            self.inner
                .get_files_zip(id, version_id, descriptor_type, limit)
        })?;
        Ok(PyBytes::new(py, &archive))
    }

    /// Gets the container specifications, such as Dockerfiles, of a tool version.
    ///
    /// Args:
    ///     id: The tool's ID.
    ///     version_id: The version's ID.
    ///
    /// Returns:
    ///     The container files.
    ///
    /// Raises:
    ///     ValueError: An ID is empty, `.` or `..`.
    ///     RuntimeError: The request failed.
    pub fn get_containerfiles(
        &self,
        py: Python<'_>,
        id: &str,
        version_id: &str,
    ) -> PyResult<Vec<PyFileWrapper>> {
        require_ids(id, version_id)?;
        let files = request(py, "get container files", || {
            self.inner.get_containerfiles(id, version_id)
        })?;
        files.into_iter().map(PyFileWrapper::new).collect()
    }
}
