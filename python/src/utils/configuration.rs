use crate::utils::convert::{check_extra_args, to_python};
use crate::ServiceSection;
use ga4gh_sdk::utils::auth::{AccessToken, BoxError, TokenProvider, TokenRequest, TokenSource};
use ga4gh_sdk::utils::configuration::{BasicAuth, Configuration};
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
            Python::attach(|py| call_token_provider(py, &callable, &audience))
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

/// The API root and settings for connecting to a GA4GH service.
///
/// `from_file()` reads a section of `~/.ga4gh-cli/config.json`, named by a
/// `ServiceType` or by a `str`; in a file with profiles it reads the default
/// profile. `get_extra()` returns the section's other keys.
/// `set_timeouts()` and `get_timeouts()` set and give the time limits of
/// requests in seconds; clients and transports read them when they are built.
/// `set_token_provider()` sets a callable that gives a bearer token for each
/// request.
///
/// Set one source of credentials before building a client: Basic credentials, an
/// OAuth or a generic bearer token, or a token provider. Clients and `Transport` take
/// the settings when they are built, so later changes don't affect them.
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

    /// Replaces the API root.
    ///
    /// Args:
    ///     base_path: The new API root, including the service's path prefix.
    ///
    /// Raises:
    ///     ValueError: `base_path` is not a valid URL.
    pub fn set_base_path(&mut self, base_path: String) -> PyResult<()> {
        let url = Url::parse(&base_path)
            .map_err(|error| PyValueError::new_err(format!("Invalid base_path: {error}")))?;
        self.inner.set_base_path(url);
        Ok(())
    }

    /// Gives the API root.
    ///
    /// Returns:
    ///     The API root as a URL string.
    pub fn get_base_path(&mut self) -> String {
        self.inner.base_path.to_string()
    }

    /// Sets one Basic credential for clients built from this configuration.
    ///
    /// Args:
    ///     username: The user name.
    ///     password: The password, if any.
    #[pyo3(signature = (username, password=None))]
    pub fn set_basic_auth(&mut self, username: String, password: Option<String>) {
        self.inner.basic_auth = Some(BasicAuth { username, password });
    }

    /// Sets one OAuth bearer token for clients built from this configuration.
    ///
    /// Args:
    ///     token: The access token, without the `Bearer ` prefix.
    pub fn set_oauth_access_token(&mut self, token: String) {
        self.inner.oauth_access_token = Some(token);
    }

    /// Sets one generic bearer token for clients built from this configuration.
    ///
    /// Args:
    ///     token: The token, without the `Bearer ` prefix.
    pub fn set_bearer_access_token(&mut self, token: String) {
        self.inner.bearer_access_token = Some(token);
    }

    /// Removes every credential source, the token provider included.
    pub fn clear_auth(&mut self) {
        self.inner.basic_auth = None;
        self.inner.oauth_access_token = None;
        self.inner.bearer_access_token = None;
        self.inner.api_key = None;
        self.inner.token_provider = None;
        self.inner.token_audience = None;
    }

    /// Sets time limits of requests, in seconds.
    ///
    /// Limits that aren't given stay as they are; if one value is invalid, none changes.
    /// Clients and `Transport` read the limits when they are built, so set them first.
    ///
    /// Args:
    ///     connect: Opening each connection (default 10).
    ///     request: Each request that the other limits don't apply to (default 60).
    ///     discovery: The service check when a client is built, and
    ///         `TRS.service_info()` (default 10).
    ///     submission: `TES.create()`, `AsyncTES.create()` and `WES.run()`, uploads
    ///         included (default 300).
    ///
    /// Raises:
    ///     ValueError: A limit is not a valid number of seconds.
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

    /// Gives the time limits of requests.
    ///
    /// Returns:
    ///     The limits in seconds, by name: `connect`, `request`, `discovery` and
    ///     `submission`.
    pub fn get_timeouts(&self) -> std::collections::BTreeMap<&'static str, f64> {
        self.inner.timeouts.as_seconds().into_iter().collect()
    }

    /// Gives the keys of the configuration file section that the SDK doesn't use.
    ///
    /// The values are as they are in the file, so don't log them if they can hold
    /// credentials.
    ///
    /// Returns:
    ///     A new `dict` of the section's other keys; empty if the configuration
    ///     wasn't read from a file.
    pub fn get_extra(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        to_python(py, &Value::Object(self.inner.extra.clone()))
    }

    /// Gets a bearer token from `provider` for each request, in place of a static one.
    ///
    /// Clients and `Transport` take the provider when they are built. After
    /// `401 Unauthorized`, the SDK drops a reused token and asks the provider again; a
    /// `GET` request is sent once more, submissions never. If the provider raises or
    /// returns something else, an empty token, or a token with characters other than
    /// visible ASCII, the call raises `RuntimeError` and its pending request isn't sent.
    /// A read-only request may already have received `401` before a failed refresh.
    /// Set the provider after `from_file()`, which replaces all settings.
    ///
    /// Args:
    ///     provider: Called as `provider(audience)`; returns the token as a `str`, or a
    ///         `(str, float)` tuple with the seconds for which the SDK may reuse it.
    ///         `None` removes the provider.
    ///     audience: The value passed to `provider`; by default, the API root.
    ///
    /// Raises:
    ///     TypeError: `provider` is not callable.
    ///     ValueError: `audience` is given without a provider.
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
        let provider: Arc<dyn TokenProvider> =
            Arc::new(PyTokenProvider(Arc::new(provider.unbind())));
        self.inner.token_provider = Some(TokenSource::new(provider));
        self.inner.token_audience = audience;
        Ok(())
    }

    /// Reads a section of `~/.ga4gh-cli/config.json`.
    ///
    /// It replaces all settings, including a token provider. In a file with profiles, the
    /// section comes from its default profile. A section can name an environment variable
    /// for its token with `oauth_access_token_env` or `bearer_access_token_env`.
    ///
    /// Args:
    ///     service_type: A `ServiceType`, or the name of any other section.
    ///
    /// Raises:
    ///     FileNotFoundError: The file doesn't exist.
    ///     ValueError: The section is missing, the file is malformed or a named
    ///         environment variable is unset.
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
