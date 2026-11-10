use crate::PyConfiguration;
use ga4gh_sdk::utils::transport::Transport;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

#[pyclass(name = "Transport", module = "GA4GH")]
pub struct PyTransport {
    pub inner: Transport,
}

#[pymethods]
impl PyTransport {
    #[new]
    pub fn new(py_config: &PyConfiguration) -> PyResult<Self> {
        let transport = Transport::new(&py_config.inner);
        Ok(PyTransport { inner: transport })
    }

    #[pyo3(signature = (endpoint, params=None))]
    pub fn get(
        &self,
        py: Python<'_>,
        endpoint: String,
        params: Option<String>,
    ) -> PyResult<String> {
        let params_value = params
            .map(|params| {
                serde_json::from_str(&params)
                    .map_err(|error| PyValueError::new_err(format!("Invalid params JSON: {error}")))
            })
            .transpose()?;
        crate::utils::runtime::io(py, |rt| {
            rt.block_on(self.inner.get(&endpoint, params_value))
                .map_err(|e| format!("GET request failed: {e}"))
        })
    }

    #[pyo3(signature = (endpoint, data=None))]
    pub fn post(&self, py: Python<'_>, endpoint: String, data: Option<String>) -> PyResult<String> {
        let data_value = data
            .map(|data| {
                serde_json::from_str(&data)
                    .map_err(|error| PyValueError::new_err(format!("Invalid data JSON: {error}")))
            })
            .transpose()?;
        crate::utils::runtime::io(py, |rt| {
            rt.block_on(self.inner.post(&endpoint, data_value))
                .map_err(|e| format!("POST request failed: {e}"))
        })
    }

    pub fn put(&self, py: Python<'_>, endpoint: String, data: String) -> PyResult<String> {
        let data_value = serde_json::from_str(&data)
            .map_err(|error| PyValueError::new_err(format!("Invalid data JSON: {error}")))?;
        crate::utils::runtime::io(py, |rt| {
            rt.block_on(self.inner.put(&endpoint, data_value))
                .map_err(|e| format!("PUT request failed: {e}"))
        })
    }

    pub fn delete(&self, py: Python<'_>, endpoint: String) -> PyResult<String> {
        crate::utils::runtime::io(py, |rt| {
            rt.block_on(self.inner.delete(&endpoint))
                .map_err(|e| format!("DELETE request failed: {e}"))
        })
    }
}
