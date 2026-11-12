use crate::utils::convert::{check_extra_args, to_python};
use crate::ServiceSection;
use ga4gh_sdk::utils::configuration::Configuration;
use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use serde_json::Value;
use url::Url;

#[pyclass(name = "Configuration", module = "GA4GH", subclass)]
pub struct PyConfiguration {
    pub inner: Configuration,
}

impl PyConfiguration {
    pub fn new(base_path: String) -> PyResult<Self> {
        let url = Url::parse(&base_path)
            .map_err(|error| PyValueError::new_err(format!("Invalid base_path: {error}")))?;
        let config = Configuration::new(url);
        Ok(PyConfiguration { inner: config })
    }
}

#[pymethods]
impl PyConfiguration {
    #[new]
    #[classmethod]
    #[pyo3(signature = (base_path, *args, **kwargs))]
    fn py_new(
        cls: &Bound<'_, PyType>,
        base_path: String,
        args: &Bound<'_, PyTuple>,
        kwargs: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Self> {
        check_extra_args::<Self>(cls, args, kwargs)?;
        Self::new(base_path)
    }

    pub fn set_base_path(&mut self, base_path: String) -> PyResult<()> {
        let url = Url::parse(&base_path)
            .map_err(|error| PyValueError::new_err(format!("Invalid base_path: {error}")))?;
        self.inner.set_base_path(url);
        Ok(())
    }

    pub fn get_base_path(&mut self) -> String {
        self.inner.base_path.to_string()
    }

    /// Returns the other keys of the configuration file section as a `dict`.
    pub fn get_extra(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &Value::Object(self.inner.extra.clone()))
    }

    /// Reads a section of `~/.ga4gh-cli/config.json`: a `ServiceType`, or the name of
    /// any other section as a `str`. In a file with profiles, the section comes from
    /// the default profile.
    pub fn from_file(&mut self, service_type: ServiceSection) -> PyResult<()> {
        self.inner = Configuration::from_file(service_type.into()).map_err(|error| {
            let message = error.to_string();
            match error.downcast_ref::<std::io::Error>() {
                Some(io_error) if io_error.kind() == std::io::ErrorKind::NotFound => {
                    PyFileNotFoundError::new_err(message)
                }
                Some(_) => PyOSError::new_err(message),
                None => PyValueError::new_err(message),
            }
        })?;
        Ok(())
    }
}
