use crate::PyServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use url::Url;

#[pyclass(name = "Configuration", module = "GA4GH")]
pub struct PyConfiguration {
    pub inner: Configuration,
}

#[pymethods]
impl PyConfiguration {
    #[new]
    pub fn new(base_path: String) -> PyResult<Self> {
        let url = Url::parse(&base_path)
            .map_err(|error| PyValueError::new_err(format!("Invalid base_path: {error}")))?;
        let config = Configuration::new(url);
        Ok(PyConfiguration { inner: config })
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

    pub fn from_file(&mut self, service_type: PyServiceType) -> PyResult<()> {
        self.inner = Configuration::from_file(service_type.into())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyException, _>(format!("{}", e)))?;
        Ok(())
    }
}
