use crate::utils::convert::{check_extra_args, to_python};
use crate::ServiceSection;
use ga4gh_sdk::utils::configuration::{BasicAuth, Configuration};
use ga4gh_sdk::utils::timeouts::Timeouts;
use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use serde_json::Value;
use url::Url;

#[pyclass(name = "Configuration", module = "ga4gh_sdk", subclass)]
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

    /// Set one Basic credential for clients built from this configuration.
    #[pyo3(signature = (username, password=None))]
    pub fn set_basic_auth(&mut self, username: String, password: Option<String>) {
        self.inner.basic_auth = Some(BasicAuth { username, password });
    }

    /// Set one OAuth bearer token for clients built from this configuration.
    pub fn set_oauth_access_token(&mut self, token: String) {
        self.inner.oauth_access_token = Some(token);
    }

    /// Set one generic bearer token for clients built from this configuration.
    pub fn set_bearer_access_token(&mut self, token: String) {
        self.inner.bearer_access_token = Some(token);
    }

    /// Remove configured Basic and bearer credentials before selecting another.
    pub fn clear_auth(&mut self) {
        self.inner.basic_auth = None;
        self.inner.oauth_access_token = None;
        self.inner.bearer_access_token = None;
        self.inner.api_key = None;
    }

    /// Sets time limits of requests, in seconds; limits that aren't given stay as
    /// they are. See `get_timeouts()`.
    #[pyo3(signature = (*, connect=None, request=None, discovery=None, submission=None))]
    pub fn set_timeouts(
        &mut self,
        connect: Option<f64>,
        request: Option<f64>,
        discovery: Option<f64>,
        submission: Option<f64>,
    ) -> PyResult<()> {
        let mut timeouts = self.inner.timeouts;
        for (name, seconds) in Timeouts::NAMES
            .into_iter()
            .zip([connect, request, discovery, submission])
        {
            if let Some(seconds) = seconds {
                timeouts
                    .set_seconds(name, seconds)
                    .map_err(PyValueError::new_err)?;
            }
        }
        // All or nothing: a rejected value leaves every limit as it was.
        self.inner.timeouts = timeouts;
        Ok(())
    }

    /// Returns the time limits of requests in seconds, as a `dict` with `connect`,
    /// `request`, `discovery` and `submission`.
    pub fn get_timeouts(&self) -> std::collections::BTreeMap<&'static str, f64> {
        self.inner.timeouts.as_seconds().into_iter().collect()
    }

    /// Returns the other keys of the configuration file section as a `dict`.
    pub fn get_extra(&self, py: Python<'_>) -> PyResult<PyObject> {
        to_python(py, &Value::Object(self.inner.extra.clone()))
    }

    /// Reads a section of `~/.ga4gh-cli/config.json`: a `ServiceType`, or the name of
    /// any other section as a `str`.
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
