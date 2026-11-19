use crate::utils::convert::{check_extra_args, to_json, to_json_opt, with_query};
use crate::utils::runtime::io;
use crate::PyConfiguration;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use reqwest::multipart::{Form, Part};
use std::borrow::Cow;

/// Sends requests below the configured API root and returns response bodies as text.
///
/// Subclass it to add helpers for further endpoints. In all methods, `endpoint` is
/// relative to the API root, such as `tasks/123`; encode IDs that you put into it, for
/// example with `urllib.parse.quote(task_id, safe="")`. `params` holds query parameters,
/// as a mapping or a JSON object string: values are strings, numbers, booleans or lists
/// of these, a list repeats the parameter, and `None` or an empty list leaves it out.
#[pyclass(name = "Transport", module = "ga4gh_sdk", subclass)]
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

    /// Sends a `GET` request.
    ///
    /// Args:
    ///     endpoint: The path below the API root.
    ///     params: Query parameters.
    ///
    /// Returns:
    ///     The response body.
    ///
    /// Raises:
    ///     ValueError: `params` holds an unsupported value.
    ///     RuntimeError: The request failed or the server answered with an error status.
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

    /// Sends a `POST` request with a JSON body.
    ///
    /// Args:
    ///     endpoint: The path below the API root.
    ///     data: Any object that `json.dumps` accepts; a `str` is taken as JSON text,
    ///         so pass `json.dumps(text)` to send a JSON string.
    ///     params: Query parameters.
    ///
    /// Returns:
    ///     The response body.
    ///
    /// Raises:
    ///     ValueError: `data` or `params` is invalid.
    ///     RuntimeError: The request failed or the server answered with an error status.
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

    /// Sends a `PUT` request with a JSON body.
    ///
    /// Args:
    ///     endpoint: The path below the API root.
    ///     data: Any object that `json.dumps` accepts; a `str` is taken as JSON text.
    ///     params: Query parameters.
    ///
    /// Returns:
    ///     The response body.
    ///
    /// Raises:
    ///     ValueError: `data` or `params` is invalid.
    ///     RuntimeError: The request failed or the server answered with an error status.
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

    /// Sends a `DELETE` request.
    ///
    /// Args:
    ///     endpoint: The path below the API root.
    ///     params: Query parameters.
    ///
    /// Returns:
    ///     The response body.
    ///
    /// Raises:
    ///     ValueError: `params` holds an unsupported value.
    ///     RuntimeError: The request failed or the server answered with an error status.
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

    /// Sends a `multipart/form-data` `POST` request.
    ///
    /// Names may repeat, as for WES `workflow_attachment`. To start a WES run, use
    /// `WES.run()` instead.
    ///
    /// Args:
    ///     endpoint: The path below the API root.
    ///     fields: `(name, value)` text parts.
    ///     files: `(name, filename, content)` or `(name, filename, content, content_type)`
    ///         file parts, with `content` as `bytes`.
    ///     params: Query parameters.
    ///
    /// Returns:
    ///     The response body.
    ///
    /// Raises:
    ///     TypeError: An argument has the wrong type, such as a `dict` for `fields`.
    ///     ValueError: A value is invalid.
    ///     RuntimeError: The request failed or the server answered with an error status.
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
