use crate::utils::convert::check_extra_args;
use crate::PyConfiguration;
use ga4gh_sdk::clients::serviceinfo::ServiceInfo;
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};

#[pyclass(name = "ServiceInfo", module = "GA4GH", subclass)]
pub struct PyServiceInfo {
    pub inner: ServiceInfo,
}

impl PyServiceInfo {
    pub fn new(py_config: &PyConfiguration) -> PyResult<Self> {
        let service_info = ServiceInfo::new(&py_config.inner)
            .map_err(|error| pyo3::exceptions::PyValueError::new_err(error.to_string()))?;
        Ok(PyServiceInfo {
            inner: service_info,
        })
    }
}

#[pymethods]
impl PyServiceInfo {
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

    pub fn get_service_info(&self) -> PyResult<String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(self.inner.get());

        match result {
            Ok(service_info) => Ok(format!("Service: {:?}", service_info)),
            Err(e) => Err(pyo3::exceptions::PyRuntimeError::new_err(format!(
                "Failed to get service info: {}",
                e
            ))),
        }
    }
}
