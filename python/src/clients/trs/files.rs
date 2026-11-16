//! Descriptor, test, container, and listed-file results.

use super::helpers::to_raw;
use crate::utils::convert::to_python;
use crate::utils::json::field;
use ga4gh_sdk::clients::trs::models::{FileWrapper, ToolFile};
use pyo3::prelude::*;
use serde_json::Value;

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
    fn checksum(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "checksum")
    }

    /// The image or descriptor type of the file, such as `"Docker"` or `"CWL"`.
    #[getter]
    fn image_type(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "image_type")
    }

    /// The file as JSON text.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The file as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
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
    fn checksum(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, &self.raw, "checksum")
    }

    /// The file as JSON text.
    pub fn json(&self) -> String {
        self.raw.to_string()
    }

    /// The file as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &self.raw)
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let path = slf.getattr("path")?.repr()?;
        let file_type = slf.getattr("file_type")?.repr()?;
        Ok(format!("{name}(path={path}, file_type={file_type})"))
    }
}
