use crate::utils::convert::{check_extra_args, to_python};
use crate::ServiceSection;
use ga4gh_sdk::utils::auth::{AccessToken, BoxError, TokenProvider, TokenRequest};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::timeouts::Timeouts;
use pyo3::exceptions::{PyFileNotFoundError, PyOSError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyTuple, PyType};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;

/// A Python callable as a token provider; see `Configuration.set_token_provider()`.
struct PyTokenProvider(Arc<Py<PyAny>>);

#[async_trait::async_trait]
impl TokenProvider for PyTokenProvider {
    async fn token(&self, request: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
        let callable = self.0.clone();
        let audience = request.audience.to_owned();
        // The callable may block, for example on a token endpoint, so it runs on a
        // blocking thread of the runtime, with the GIL, and not in the request's future.
        tokio::task::spawn_blocking(move || {
            Python::with_gil(|py| call_token_provider(py, &callable, &audience))
        })
        .await
        .map_err(|error| format!("The token provider didn't finish: {error}"))?
    }
}

/// Calls `callable(audience)` and reads its `str` or `(str, float)` result.
///
/// Python exceptions become their text, as `TypeName: message`, while the GIL is held.
fn call_token_provider(
    py: Python<'_>,
    callable: &Py<PyAny>,
    audience: &str,
) -> Result<AccessToken, BoxError> {
    let result = callable
        .call1(py, (audience,))
        .map_err(|error| error.to_string())?;
    let result = result.bind(py);
    if let Ok(token) = result.extract::<String>() {
        return Ok(AccessToken::new(token));
    }
    let Ok((token, seconds)) = result.extract::<(String, f64)>() else {
        return Err("The token provider must return a str or a (str, float) tuple".into());
    };
    let until = Duration::try_from_secs_f64(seconds)
        .ok()
        .and_then(|lifetime| Instant::now().checked_add(lifetime))
        .ok_or("The token provider's reuse time must be a non-negative number of seconds")?;
    Ok(AccessToken::new(token).reusable_until(until))
}

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

    /// Gets a bearer token from `provider` for each request, in place of a static
    /// credential; `None` removes the provider.
    ///
    /// `provider(audience)` is called with `audience`, or the API root if it is
    /// `None`, and returns the token as a `str`, or a `(str, float)` tuple with the
    /// seconds for which the SDK may reuse it. See the Python bindings guide.
    #[pyo3(signature = (provider, *, audience=None))]
    pub fn set_token_provider(
        &mut self,
        provider: Option<Bound<'_, PyAny>>,
        audience: Option<String>,
    ) -> PyResult<()> {
        let Some(provider) = provider else {
            if audience.is_some() {
                return Err(PyValueError::new_err("audience needs a token provider"));
            }
            self.inner.token_provider = None;
            self.inner.token_audience = None;
            return Ok(());
        };
        if !provider.is_callable() {
            return Err(PyTypeError::new_err("provider must be callable or None"));
        }
        let mut config = self
            .inner
            .clone()
            .with_token_provider(PyTokenProvider(Arc::new(provider.unbind())));
        config.token_audience = audience;
        self.inner = config;
        Ok(())
    }

    /// Reads a section of `~/.ga4gh-cli/config.json`: a `ServiceType`, or the name of
    /// any other section as a `str`. It replaces all settings, including a token
    /// provider.
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
