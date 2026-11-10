use crate::utils::convert::{check_extra_args, to_json, to_json_opt, with_query};
use crate::utils::runtime::io;
use crate::PyConfiguration;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use reqwest::multipart::{Form, Part};
use std::borrow::Cow;

/// Sends requests below the configured API root.
///
/// `params` is a `dict` or a JSON object string of query parameters; list values repeat the
/// parameter. `data` is a JSON string or any JSON-serialisable object. Responses are returned as
/// text. Subclass it to add helpers for further endpoints.
#[pyclass(name = "Transport", module = "GA4GH", subclass)]
pub struct PyTransport {
    pub inner: Transport,
}

impl PyTransport {
    pub fn new(py_config: &PyConfiguration) -> PyResult<Self> {
        let transport = Transport::new(&py_config.inner);
        Ok(PyTransport { inner: transport })
    }
}

#[pymethods]
impl PyTransport {
    #[new]
    #[classmethod]
    #[pyo3(signature = (py_config, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        py_config: &PyConfiguration,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::new(py_config)
    }

    #[pyo3(signature = (endpoint, params=None))]
    pub fn get(
        &self,
        py: Python<'_>,
        endpoint: &str,
        params: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        let endpoint = with_query(endpoint, to_json_opt(params, "params")?)?;
        io(py, |rt| {
            rt.block_on(self.inner.get(&endpoint, None))
                .map_err(|error| format!("GET request failed: {error}"))
        })
    }

    #[pyo3(signature = (endpoint, data=None, params=None))]
    pub fn post(
        &self,
        py: Python<'_>,
        endpoint: &str,
        data: Option<&Bound<'_, PyAny>>,
        params: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        let data = to_json_opt(data, "data")?;
        let endpoint = with_query(endpoint, to_json_opt(params, "params")?)?;
        io(py, |rt| {
            rt.block_on(self.inner.post(&endpoint, data))
                .map_err(|error| format!("POST request failed: {error}"))
        })
    }

    #[pyo3(signature = (endpoint, data, params=None))]
    pub fn put(
        &self,
        py: Python<'_>,
        endpoint: &str,
        data: &Bound<'_, PyAny>,
        params: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        let data = to_json(data, "data")?;
        let endpoint = with_query(endpoint, to_json_opt(params, "params")?)?;
        io(py, |rt| {
            rt.block_on(self.inner.put(&endpoint, data))
                .map_err(|error| format!("PUT request failed: {error}"))
        })
    }

    #[pyo3(signature = (endpoint, params=None))]
    pub fn delete(
        &self,
        py: Python<'_>,
        endpoint: &str,
        params: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        let endpoint = with_query(endpoint, to_json_opt(params, "params")?)?;
        io(py, |rt| {
            rt.block_on(self.inner.delete(&endpoint))
                .map_err(|error| format!("DELETE request failed: {error}"))
        })
    }

    /// Sends a `multipart/form-data` POST request.
    ///
    /// `fields` is a list of `(name, value)` text parts. `files` is a list of
    /// `(name, filename, content)` or `(name, filename, content_type)` tuples, where
    /// `content` is `bytes`. Names may repeat, as for WES `workflow_attachment`.
    #[pyo3(signature = (endpoint, fields=None, files=None, params=None))]
    pub fn post_multipart(
        &self,
        py: Python<'_>,
        endpoint: &str,
        fields: Option<Vec<(String, String)>>,
        files: Option<Vec<Bound<'_, PyTuple>>>,
        params: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<String> {
        let mut form = Form::new();
        for (name, value) in fields.unwrap_or_default() {
            form = form.text(name, value);
        }
        for file in files.unwrap_or_default() {
            form = form.part(file_name(&file)?, file_part(&file)?);
        }
        let endpoint = with_query(endpoint, to_json_opt(params, "params")?)?;
        io(py, |rt| {
            rt.block_on(self.inner.post_multipart(&endpoint, form))
                .map_err(|error| format!("POST request failed: {error}"))
        })
    }
}

const FILE_TUPLE: &str =
    "files items must be (name, filename, content) or (name, filename, content, content_type)";

fn file_name(file: &Bound<'_, PyTuple>) -> PyResult<String> {
    if !(3..=4).contains(&file.len()) {
        return Err(PyValueError::new_err(FILE_TUPLE));
    }
    file.get_item(0)?
        .extract()
        .map_err(|_| PyValueError::new_err(FILE_TUPLE))
}

fn file_part(file: &Bound<'_, PyTuple>) -> PyResult<Part> {
    let invalid = |_| PyValueError::new_err(FILE_TUPLE);
    let filename: String = file.get_item(1)?.extract().map_err(invalid)?;
    let content = file.get_item(2)?;
    let content: Cow<'_, [u8]> = content.extract().map_err(invalid)?;
    let mut part = Part::bytes(content.into_owned()).file_name(filename);
    if file.len() == 4 {
        let content_type: String = file.get_item(3)?.extract().map_err(invalid)?;
        part = part.mime_str(&content_type).map_err(|error| {
            PyValueError::new_err(format!("Invalid files content type: {error}"))
        })?;
    }
    Ok(part)
}
