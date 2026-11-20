//! Results that keep the JSON the server sent: `DrsObject`, `AccessURL` and
//! `BulkAccessURL`.

use super::helpers::{invalid_response, parse_model};
use crate::utils::convert::to_python;
use crate::utils::json::{field, view_accessors};
use ga4gh_sdk::clients::drs::models::{AccessUrl, BulkAccessUrl, DrsObject};
use pyo3::prelude::*;
use serde_json::Value;

/// A DRS object's metadata, as returned by the server.
///
/// Keeps the object's full JSON, including fields that the DRS models don't know.
/// Nested values, such as `access_methods`, come as `dict`s and `list`s.
#[pyclass(name = "DrsObject", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyDrsObject {
    /// The object parsed into the DRS model; derived from `raw`.
    inner: DrsObject,
    /// The object's JSON as received.
    raw: Value,
}

view_accessors!(PyDrsObject, DrsObject);

impl PyDrsObject {
    pub(super) fn parse(raw: Value) -> Result<Self, String> {
        Ok(PyDrsObject {
            inner: parse_model(&raw)?,
            raw,
        })
    }

    /// Builds an object from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }
}

#[pymethods]
impl PyDrsObject {
    /// The object's ID.
    #[getter]
    fn id(&self) -> String {
        self.inner().id.clone()
    }

    /// The object's name, such as a file name.
    #[getter]
    fn name(&self) -> Option<String> {
        self.inner().name.clone()
    }

    /// The object's `drs://` URI.
    #[getter]
    fn self_uri(&self) -> String {
        self.inner().self_uri.clone()
    }

    /// The object's size in bytes; for a bundle, the sum of its contents.
    #[getter]
    fn size(&self) -> i64 {
        self.inner().size
    }

    /// When the object was created, as an RFC 3339 timestamp.
    #[getter]
    fn created_time(&self) -> String {
        self.inner().created_time.clone()
    }

    /// When the object was last updated, as an RFC 3339 timestamp.
    #[getter]
    fn updated_time(&self) -> Option<String> {
        self.inner().updated_time.clone()
    }

    /// The object's version, as the server names it.
    #[getter]
    fn version(&self) -> Option<String> {
        self.inner().version.clone()
    }

    /// The object's media type, such as `"application/json"`.
    #[getter]
    fn mime_type(&self) -> Option<String> {
        self.inner().mime_type.clone()
    }

    /// The object's checksums, as `dict`s with `type` and `checksum`.
    #[getter]
    fn checksums(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "checksums")
    }

    /// The ways to fetch the object's bytes, as `dict`s with `type` and `access_id` or
    /// `access_url`; `None` for a bundle.
    #[getter]
    fn access_methods(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "access_methods")
    }

    /// For a bundle, the objects that it contains, as `dict`s.
    #[getter]
    fn contents(&self, py: Python<'_>) -> PyResult<Option<Py<PyAny>>> {
        field(py, self.raw(), "contents")
    }

    /// A description of the object.
    #[getter]
    fn description(&self) -> Option<String> {
        self.inner().description.clone()
    }

    /// Other names of the object.
    #[getter]
    fn aliases(&self) -> Option<Vec<String>> {
        self.inner().aliases.clone()
    }

    /// Gives the object as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// Gives the object as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let id = slf.getattr("id")?.repr()?;
        let size = slf.borrow().inner().size;
        Ok(format!("{name}(id={id}, size={size})"))
    }
}

/// An access URL of a DRS object.
///
/// The URL and headers may hold credentials, such as a signature or a bearer token, so
/// `repr()` doesn't show them; don't log them either.
#[pyclass(name = "AccessURL", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyAccessURL {
    /// The access URL parsed into the DRS model; derived from `raw`.
    inner: AccessUrl,
    /// The access URL's JSON as received.
    raw: Value,
}

view_accessors!(PyAccessURL, AccessUrl);

impl PyAccessURL {
    fn parse(raw: Value) -> Result<Self, String> {
        Ok(PyAccessURL {
            inner: parse_model(&raw)?,
            raw,
        })
    }

    /// Builds an access URL from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }
}

#[pymethods]
impl PyAccessURL {
    /// The URL to fetch the object's bytes from.
    #[getter]
    fn url(&self) -> String {
        self.inner().url.clone()
    }

    /// The headers to send with a request to `url`, as `"Name: value"` strings.
    #[getter]
    fn headers(&self) -> Option<Vec<String>> {
        self.inner().headers.clone()
    }

    /// Gives the access URL as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// Gives the access URL as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let headers = slf.borrow().inner().headers.as_ref().map_or(0, Vec::len);
        Ok(format!("{name}(url=<redacted>, headers={headers})"))
    }
}

/// An access URL from `DRS.get_bulk_access_urls()`, with the IDs it belongs to.
///
/// Like `AccessURL`, `repr()` doesn't show the URL or headers.
#[pyclass(name = "BulkAccessURL", module = "ga4gh_sdk", subclass)]
#[derive(Clone)]
pub struct PyBulkAccessURL {
    /// The access URL parsed into the DRS model; derived from `raw`.
    inner: BulkAccessUrl,
    /// The entry's JSON as received.
    raw: Value,
}

view_accessors!(PyBulkAccessURL, BulkAccessUrl);

impl PyBulkAccessURL {
    pub(super) fn parse(raw: Value) -> Result<Self, String> {
        Ok(PyBulkAccessURL {
            inner: parse_model(&raw)?,
            raw,
        })
    }

    /// Builds an entry from a server's JSON, keeping the JSON. Errors are `RuntimeError`s.
    pub fn from_json(raw: Value) -> PyResult<Self> {
        Self::parse(raw).map_err(invalid_response)
    }
}

#[pymethods]
impl PyBulkAccessURL {
    /// The ID of the object that the URL belongs to.
    #[getter]
    fn drs_object_id(&self) -> Option<String> {
        self.inner().drs_object_id.clone()
    }

    /// The access ID that the URL belongs to.
    #[getter]
    fn drs_access_id(&self) -> Option<String> {
        self.inner().drs_access_id.clone()
    }

    /// The URL to fetch the object's bytes from.
    #[getter]
    fn url(&self) -> String {
        self.inner().url.clone()
    }

    /// The headers to send with a request to `url`, as `"Name: value"` strings.
    #[getter]
    fn headers(&self) -> Option<Vec<String>> {
        self.inner().headers.clone()
    }

    /// Gives the entry as JSON text, as received.
    pub fn json(&self) -> String {
        self.raw().to_string()
    }

    /// Gives the entry as a new `dict`, as received.
    pub fn to_dict(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, self.raw())
    }

    fn __repr__(slf: &Bound<'_, Self>) -> PyResult<String> {
        let name = slf.get_type().qualname()?;
        let object_id = slf.getattr("drs_object_id")?.repr()?;
        let headers = slf.borrow().inner().headers.as_ref().map_or(0, Vec::len);
        Ok(format!(
            "{name}(drs_object_id={object_id}, url=<redacted>, headers={headers})"
        ))
    }
}

// The tests call `parse`, which `from_json` shares (see `utils::json::tests`).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::json::tests::assert_describes;
    use serde_json::json;

    #[test]
    fn an_object_describes_its_json() {
        let raw = json!({
            "id": "obj-1",
            "self_uri": "drs://example.org/obj-1",
            "size": 3,
            "created_time": "2024-01-01T00:00:00Z",
            "checksums": [{"checksum": "abc", "type": "md5"}],
            "description": null,
            "x_storage": {"tier": "cold"}
        });
        let object = PyDrsObject::parse(raw.clone()).unwrap();
        assert_describes(object.inner(), object.raw(), &object.json(), &raw);
        assert_eq!(object.inner().description, None);
        assert_eq!(object.inner().extra["x_storage"], json!({"tier": "cold"}));
    }

    #[test]
    fn an_access_url_describes_its_json() {
        let raw = json!({"url": "https://example.org/a", "headers": null, "x_expires": 60});
        let url = PyAccessURL::parse(raw.clone()).unwrap();
        assert_describes(url.inner(), url.raw(), &url.json(), &raw);
        assert_eq!(url.inner().headers, None);
        assert_eq!(url.inner().extra["x_expires"], 60);
    }

    #[test]
    fn a_bulk_access_url_describes_its_json() {
        let raw = json!({
            "drs_object_id": "obj-1",
            "drs_access_id": null,
            "url": "https://example.org/a",
            "x_expires": 60
        });
        let url = PyBulkAccessURL::parse(raw.clone()).unwrap();
        assert_describes(url.inner(), url.raw(), &url.json(), &raw);
        assert_eq!(url.inner().drs_access_id, None);
        assert_eq!(url.inner().extra["x_expires"], 60);
    }

    #[test]
    fn json_that_is_no_object_is_rejected() {
        let raw = json!(["https://example.org/a"]);
        assert_eq!(
            PyDrsObject::parse(raw.clone()).err().unwrap(),
            "not a JSON object"
        );
        assert_eq!(
            PyAccessURL::parse(raw.clone()).err().unwrap(),
            "not a JSON object"
        );
        assert_eq!(
            PyBulkAccessURL::parse(raw).err().unwrap(),
            "not a JSON object"
        );
    }
}
