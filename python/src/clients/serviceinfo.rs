use crate::PyConfiguration;
use ga4gh_sdk::clients::serviceinfo::ServiceInfo;
use pyo3::prelude::*;

#[pyclass(name = "ServiceInfo", module = "GA4GH")]
pub struct PyServiceInfo {
    inner: ServiceInfo,
}

#[pymethods]
impl PyServiceInfo {
    #[new]
    pub fn new(py_config: &PyConfiguration) -> PyResult<Self> {
        let service_info = ServiceInfo::new(&py_config.inner)
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        Ok(PyServiceInfo {
            inner: service_info,
        })
    }

    pub fn get_service_info(&self, py: Python<'_>) -> PyResult<String> {
        crate::utils::runtime::io(py, |rt| {
            rt.block_on(self.inner.get())
                .map(|service| format!("Service: {service:?}"))
                .map_err(|e| format!("Failed to get service info: {e}"))
        })
    }
}
