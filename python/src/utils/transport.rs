use crate::PyConfiguration;
use ga4gh_sdk::utils::transport::Transport;
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
    pub fn get(&self, endpoint: String, params: Option<String>) -> PyResult<String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let params_value = params.map(|p| serde_json::from_str(&p).unwrap());
        let result = rt.block_on(self.inner.get(&endpoint, params_value));

        match result {
            Ok(response) => Ok(response),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "GET request failed: {}",
                e
            ))),
        }
    }

    #[pyo3(signature = (endpoint, data=None))]
    pub fn post(&self, endpoint: String, data: Option<String>) -> PyResult<String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let data_value = data.map(|d| serde_json::from_str(&d).unwrap());
        let result = rt.block_on(self.inner.post(&endpoint, data_value));

        match result {
            Ok(response) => Ok(response),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "POST request failed: {}",
                e
            ))),
        }
    }

    pub fn put(&self, endpoint: String, data: String) -> PyResult<String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let data_value = serde_json::from_str(&data).unwrap();
        let result = rt.block_on(self.inner.put(&endpoint, data_value));

        match result {
            Ok(response) => Ok(response),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "PUT request failed: {}",
                e
            ))),
        }
    }

    pub fn delete(&self, endpoint: String) -> PyResult<String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(self.inner.delete(&endpoint));

        match result {
            Ok(response) => Ok(response),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "DELETE request failed: {}",
                e
            ))),
        }
    }
}
