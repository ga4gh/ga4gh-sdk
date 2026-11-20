//! Descriptor, test, container, and listed-file results.

use super::helpers::invalid_response;
use crate::utils::convert::to_python;
use crate::utils::json::{field, view_accessors};
use ga4gh_sdk::clients::trs::models::{FileWrapper, ToolFile};
use pyo3::prelude::*;
use serde_json::Value;

/// A descriptor, test or container file: its `content`, or a `url` to fetch it from.
///
/// For `PLAIN_*` descriptor types, only `content` is set. `repr()` shows the length of
/// `content`, not the content.
#[pyclass(name = "FileWrapper", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyFileWrapper {
    /// The file in the TRS model.
    inner: FileWrapper,
    /// The file as JSON; derived from `inner`.
    raw: Value,
}

view_accessors!(PyFileWrapper, FileWrapper);

impl PyFileWrapper {
    fn build(inner: FileWrapper) -> serde_json::Result<Self> {
        Ok(Self {
            raw: serde_json::to_value(&inner)?,
            inner,
        })
    }

    /// Wraps a file of the Rust client, keeping its JSON.
    pub fn new(inner: FileWrapper) -> PyResult<Self> {
        Self::build(inner).map_err(invalid_response)
    }
}

#[pymethods]
impl PyFileWrapper {
    /// The file's content.
    #[getter]
    fn content(&self) -> Option<String> {
        self.inner().content.clone()
    }

    /// The URL to fetch the file from.
    #[getter]
    fn url(&self) -> Option<String> {
        self.inner().url.clone()
    }

    /// The file's checksums, as `dict`s with `type` and `checksum`.
    #[getter]
    fn checksum(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "checksum")
    }

    /// The image or descriptor type of the file, such as `"Docker"` or `"CWL"`.
    #[getter]
    fn image_type(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "image_type")
    }

    /// Gives the file as JSON text.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// Gives the file as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    /// Shows the URL and the length of the content, not the content itself.
    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let this = slf.borrow();
        let url = slf.getattr("url")?.repr()?;
        let content = match &this.inner().content {
            Some(content) => format!("<{} characters>", content.chars().count()),
            None => "None".to_owned(),
        };
        Ok(format!("{name}(url={url}, content={content})"))
    }
}

/// A file of a tool version, from `TRS.list_files()`.
#[pyclass(name = "ToolFile", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyToolFile {
    /// The file in the TRS model.
    inner: ToolFile,
    /// The file as JSON; derived from `inner`.
    raw: Value,
}

view_accessors!(PyToolFile, ToolFile);

impl PyToolFile {
    fn build(inner: ToolFile) -> serde_json::Result<Self> {
        Ok(Self {
            raw: serde_json::to_value(&inner)?,
            inner,
        })
    }

    /// Wraps a file of the Rust client, keeping its JSON.
    pub fn new(inner: ToolFile) -> PyResult<Self> {
        Self::build(inner).map_err(invalid_response)
    }
}

#[pymethods]
impl PyToolFile {
    /// The file's path, relative to the primary descriptor.
    #[getter]
    fn path(&self) -> Option<String> {
        self.inner().path.clone()
    }

    /// The file type, such as `"PRIMARY_DESCRIPTOR"`, as the server sent it.
    #[getter]
    fn file_type(&self) -> Option<String> {
        self.inner().file_type.as_ref().map(ToString::to_string)
    }

    /// The file's checksum, as a `dict` with `type` and `checksum`.
    #[getter]
    fn checksum(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "checksum")
    }

    /// Gives the file as JSON text.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// Gives the file as a new `dict`.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let path = slf.getattr("path")?.repr()?;
        let file_type = slf.getattr("file_type")?.repr()?;
        Ok(format!("{name}(path={path}, file_type={file_type})"))
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
    fn a_file_describes_its_json() {
        let file = json!({"content": "cwlVersion: v1.2", "x_encoding": "utf-8"});
        let mut given = file.clone();
        given["url"] = Value::Null;
        let view = PyFileWrapper::build(model(&given)).unwrap();
        // The models serialize unknown fields, and leave out the ones that are `null`.
        assert_describes(view.inner(), view.raw(), &view.json(), &file);
        assert_eq!(view.inner().url, None);
        assert_eq!(view.inner().extra["x_encoding"], "utf-8");
    }

    #[test]
    fn a_listed_file_describes_its_json() {
        let file = json!({"path": "main.cwl", "file_type": "PRIMARY_DESCRIPTOR", "x_size": 9});
        let mut given = file.clone();
        given["checksum"] = Value::Null;
        let view = PyToolFile::build(model(&given)).unwrap();
        assert_describes(view.inner(), view.raw(), &view.json(), &file);
        assert_eq!(view.inner().checksum, None);
        assert_eq!(view.inner().extra["x_size"], 9);
    }
}
