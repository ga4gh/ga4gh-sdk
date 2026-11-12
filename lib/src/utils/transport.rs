/// A struct representing a transport for making HTTP requests.
///
/// The `Transport` struct is responsible for handling HTTP requests using the `reqwest` crate.
/// It provides methods for making GET, POST, PUT, and DELETE requests.
///
/// # Examples
///
/// ```
/// use crate::ga4gh_sdk::utils::configuration::Configuration;
/// use crate::ga4gh_sdk::utils::transport::Transport;
///
/// let config = Configuration::new(url::Url::parse("https://api.example.com").unwrap());
/// let transport = Transport::new(&config);
///
/// // Make a GET request
/// async {
/// let response = transport.get("users", None).await;
/// };
///
/// // Make a POST request
/// async {
///     let data = serde_json::json!({"name": "John Doe", "age": 30});
///     let response = transport.post("users", Some(data)).await;
/// };
/// // Make a PUT request
/// async {
/// let data = serde_json::json!({"name": "John Doe", "age": 30});
/// let response = transport.put("users/1", data).await;
/// };
///
/// // Make a DELETE request
/// async {
/// let response = transport.delete("users/1").await;
/// };
/// ```
use crate::utils::auth::{AuthConflictError, TokenProviderError};
use crate::utils::configuration::{AuthError, Configuration, ConfiguredAuth};
use crate::utils::query::query_pairs;
use reqwest::multipart::Form;
use reqwest::Client;
use serde_json::Value;
use std::error::Error;
use std::fmt;
use url::Url;

/// Default bounds for convenience text/JSON helpers. Advanced callers can choose
/// their own limits or consume the raw response incrementally.
#[derive(Clone, Copy, Debug)]
pub struct BodyLimits {
    pub success: usize,
    pub error: usize,
}

/// Explicit absolute deadline, replay classification and bounded retry policy.
#[derive(Clone, Debug)]
pub struct RequestOptions {
    pub operation: crate::utils::operation::Operation,
    pub retry: crate::utils::operation::RetryPolicy,
    pub safety: crate::utils::operation::ReplaySafety,
    pub limits: BodyLimits,
}

impl RequestOptions {
    /// Defaults to one non-replayable attempt.
    pub fn new(budget: std::time::Duration) -> Self {
        Self {
            operation: crate::utils::operation::Operation::new(budget),
            retry: Default::default(),
            safety: crate::utils::operation::ReplaySafety::NonReplayable,
            limits: Default::default(),
        }
    }
}

impl Default for BodyLimits {
    fn default() -> Self {
        Self {
            success: 8 * 1024 * 1024,
            error: 16 * 1024,
        }
    }
}

/// An oversized body. HTTP status remains available even on error responses.
#[derive(Debug)]
pub struct BodyLimitError {
    pub status: reqwest::StatusCode,
    pub limit: usize,
}
impl fmt::Display for BodyLimitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HTTP {} response exceeds body limit of {} bytes",
            self.status, self.limit
        )
    }
}
impl Error for BodyLimitError {}

/// Lossless buffered response, including successful and unsuccessful HTTP status.
#[derive(Debug)]
pub struct BufferedResponse {
    pub status: reqwest::StatusCode,
    pub headers: reqwest::header::HeaderMap,
    pub body: Vec<u8>,
}

/// Read incrementally with a hard bound, including bodies without Content-Length.
/// Dropping this future also drops the response and stops consumption.
pub async fn read_bounded(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<BufferedResponse, Box<dyn Error>> {
    let status = response.status();
    let headers = response.headers().clone();
    if response
        .content_length()
        .is_some_and(|len| len > limit as u64)
    {
        return Err(Box::new(BodyLimitError { status, limit }));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if chunk.len() > limit.saturating_sub(body.len()) {
            return Err(Box::new(BodyLimitError { status, limit }));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(BufferedResponse {
        status,
        headers,
        body,
    })
}

/// Compatibility text decoding, bounded before decoding for both success/errors.
pub async fn response_text(
    response: reqwest::Response,
    limits: BodyLimits,
) -> Result<String, Box<dyn Error>> {
    let response = checked_response(response, limits).await?;
    Ok(String::from_utf8_lossy(&response.body).into_owned())
}

/// Decode bounded JSON from the original bytes. Invalid UTF-8 and malformed JSON
/// return a decoding error. Non-success responses retain the existing HttpError.
pub async fn response_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
    limits: BodyLimits,
) -> Result<T, Box<dyn Error>> {
    let response = checked_response(response, limits).await?;
    // Validate the whole document even when T ignores unknown properties.
    // A byte deserializer may skip UTF-8 checks for discarded string values.
    let text = std::str::from_utf8(&response.body)?;
    Ok(serde_json::from_str(text)?)
}

async fn checked_response(
    response: reqwest::Response,
    limits: BodyLimits,
) -> Result<BufferedResponse, Box<dyn Error>> {
    let limit = if response.status().is_success() {
        limits.success
    } else {
        limits.error
    };
    let response = read_bounded(response, limit).await?;
    if response.status.is_success() {
        Ok(response)
    } else {
        Err(Box::new(HttpError {
            status: response.status,
            body: String::from_utf8_lossy(&response.body).into_owned(),
        }))
    }
}

/// Percent-encodes one URL path segment, including `/` and spaces.
pub fn urlencode<T: AsRef<str>>(value: T) -> String {
    value
        .as_ref()
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// An HTTP response whose status is not successful.
///
/// `body` is the untrusted server response and may contain credentials echoed
/// by the server. Access it explicitly when needed, but do not log or display
/// it without reviewing its contents. `Display` and `Debug` omit the body.
pub struct HttpError {
    pub status: reqwest::StatusCode,
    pub body: String,
}

impl fmt::Debug for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpError")
            .field("status", &self.status)
            .field("body", &"<untrusted response omitted>")
            .finish()
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Request failed with status: {}", self.status)
    }
}

impl Error for HttpError {}

/// Previous name for [`HttpError`], retained for SDK callers.
pub use HttpError as HttpStatusError;

/// An endpoint or API root that cannot safely be used for a request.
#[derive(Debug)]
pub enum EndpointError {
    InvalidBase,
    InvalidPath,
    OutsideApiRoot,
    Parse(url::ParseError),
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase => {
                write!(
                    f,
                    "base_path must be a hierarchical URL without a query or fragment"
                )
            }
            Self::InvalidPath => {
                write!(
                    f,
                    "endpoint must be a relative path without dot segments or backslashes"
                )
            }
            Self::OutsideApiRoot => write!(f, "endpoint resolves outside base_path"),
            Self::Parse(error) => write!(f, "invalid endpoint URL: {error}"),
        }
    }
}

impl Error for EndpointError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            _ => None,
        }
    }
}

impl From<url::ParseError> for EndpointError {
    fn from(error: url::ParseError) -> Self {
        Self::Parse(error)
    }
}

// WHATWG URL parsing treats percent-encoded dots as dot segments too.
fn is_dot_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    let mut index = 0;
    let mut dots = 0;
    while index < bytes.len() {
        if bytes[index] == b'.' {
            index += 1;
        } else if bytes.get(index..index + 3).is_some_and(|encoded| {
            encoded[0] == b'%' && encoded[1] == b'2' && encoded[2].eq_ignore_ascii_case(&b'e')
        }) {
            index += 3;
        } else {
            return false;
        }
        dots += 1;
    }
    dots == 1 || dots == 2
}

// note: could implement custom certs handling, such as in-TEE generated ephemeral certs
#[derive(Clone)]
pub struct Transport {
    pub config: Configuration,
    client: reqwest::Client,
    apply_configured_auth: bool,
}

impl fmt::Debug for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transport")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl Transport {
    /// Creates a new `Transport` instance with the given configuration.
    ///
    /// The HTTP client uses the `connect` and `request` limits of
    /// `config.timeouts`; see [`Timeouts`](crate::utils::timeouts::Timeouts).
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration for the transport.
    ///
    /// # Returns
    ///
    /// A new `Transport` instance.
    pub fn new(config: &Configuration) -> Self {
        Self::build(config).expect("default HTTP client initialization failed")
    }

    // SDK-owned clients never follow redirects implicitly: a redirect can
    // leave the configured API root or replay a submission.
    fn build(config: &Configuration) -> reqwest::Result<Self> {
        Client::builder()
            .connect_timeout(config.timeouts.connect)
            .timeout(config.timeouts.request)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(|client| Self {
                config: config.clone(),
                client,
                apply_configured_auth: true,
            })
    }

    /// Inject a configured client, for connection budgets, TLS and downstream use.
    ///
    /// The client's own connect and request timeouts apply, and
    /// `config.timeouts.connect` and `request` are not used; the `discovery` and
    /// `submission` limits still come from `config.timeouts`. The configured
    /// credentials, including a token provider, are not applied. If both are
    /// present, requests fail with
    /// [`AuthError::CallerOwnedClient`]; use
    /// [`with_client_and_configured_auth`](Self::with_client_and_configured_auth)
    /// to opt in. The caller retains responsibility for redirect policy.
    pub fn with_client(config: &Configuration, client: Client) -> Self {
        Self {
            config: config.clone(),
            client,
            apply_configured_auth: false,
        }
    }

    /// Explicitly apply configured credentials to a caller-owned HTTP client.
    /// The caller owns that client's redirect policy and must prevent credential
    /// forwarding outside the configured API root and replay of submissions.
    pub fn with_client_and_configured_auth(config: &Configuration, client: Client) -> Self {
        Self {
            config: config.clone(),
            client,
            apply_configured_auth: true,
        }
    }

    /// Read-only access to the underlying HTTP client. Requests made through a
    /// clone of it do not acquire SDK-configured authentication automatically.
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Creates a transport with these connect and request timeouts in place of
    /// those of `config.timeouts`, and records them there.
    pub fn with_timeouts(
        config: &Configuration,
        connect: std::time::Duration,
        request: std::time::Duration,
    ) -> reqwest::Result<Self> {
        let mut config = config.clone();
        config.timeouts.connect = connect;
        config.timeouts.request = request;
        Self::build(&config)
    }

    /// Sends a GET request within `timeout`, in place of the request timeout,
    /// including reading the response. The connect timeout still applies.
    pub(crate) async fn get_within(
        &self,
        endpoint: &str,
        timeout: std::time::Duration,
    ) -> Result<String, Box<dyn Error>> {
        let operation = crate::utils::operation::Operation::new(timeout);
        operation
            .run(async {
                let response = self
                    .send_authorized(
                        reqwest::Method::GET,
                        endpoint,
                        true,
                        |request| Ok(request.timeout(timeout)),
                        None,
                    )
                    .await?;
                response_text(response, BodyLimits::default()).await
            })
            .await?
    }

    /// Strict JSON discovery under the configured absolute discovery budget.
    pub(crate) async fn get_json_within<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        timeout: std::time::Duration,
    ) -> Result<T, Box<dyn Error>> {
        crate::utils::operation::Operation::new(timeout)
            .run(async {
                let response = self
                    .send_authorized(
                        reqwest::Method::GET,
                        endpoint,
                        true,
                        |request| {
                            Ok(request
                                .header(reqwest::header::ACCEPT, "application/json")
                                .timeout(timeout))
                        },
                        None,
                    )
                    .await?;
                response_json(response, BodyLimits::default()).await
            })
            .await?
    }

    /// Sends exactly once under an absolute operation budget, including body I/O.
    /// Intended for streamed/non-replayable submissions with a deliberate budget.
    pub async fn send_once(
        &self,
        request: reqwest::RequestBuilder,
        operation: &crate::utils::operation::Operation,
        limits: BodyLimits,
    ) -> Result<String, Box<dyn Error>> {
        use crate::utils::operation::AmbiguousSubmissionError;
        let request = request.timeout(
            operation
                .deadline
                .saturating_duration_since(tokio::time::Instant::now()),
        );
        // The async body runs only when Operation::run polls the send future.
        // A token/deadline check outside that future would leave a race where
        // run rejects the operation before any attempt has begun.
        let attempted = std::sync::atomic::AtomicBool::new(false);
        let result = operation
            .run(async {
                response_text(self.send_checked(request, Some(&attempted)).await?, limits).await
            })
            .await;
        let attempted = attempted.load(std::sync::atomic::Ordering::SeqCst);
        match result {
            Ok(Ok(body)) => Ok(body),
            Ok(Err(error)) if !attempted => Err(error),
            Ok(Err(error)) if error.downcast_ref::<HttpError>().is_some() => Err(error),
            Ok(Err(error)) => Err(Box::new(AmbiguousSubmissionError {
                reason: error.to_string(),
            })),
            Err(error) if attempted => Err(Box::new(AmbiguousSubmissionError {
                reason: error.to_string(),
            })),
            Err(error) => Err(Box::new(error)),
        }
    }

    /// Replay policy is opt-in and operation-specific. Never classify task/run
    /// creation as ReadOnly. A lookup POST may be ReadOnly if the protocol says so.
    pub async fn request_with_policy(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
        options: &RequestOptions,
    ) -> Result<String, Box<dyn Error>> {
        use crate::utils::operation::{retry_after, AmbiguousSubmissionError, ReplaySafety};
        options.retry.validate()?;
        let attempts = if options.safety == ReplaySafety::ReadOnly {
            options.retry.max_attempts
        } else {
            1
        };
        for attempt in 1..=attempts {
            // Set when the request is sent, after the token provider (if any)
            // gave its token: a deadline before that is not an ambiguous submission.
            let sent = std::sync::atomic::AtomicBool::new(false);
            let result = options
                .operation
                .run(self.request_response_with(
                    method.clone(),
                    endpoint,
                    data.clone(),
                    params.clone(),
                    options.safety == ReplaySafety::ReadOnly,
                    Some(&sent),
                ))
                .await;
            let attempted = sent.load(std::sync::atomic::Ordering::SeqCst);
            let response = match result {
                Ok(Ok(response)) => response,
                Err(error) if attempted && options.safety == ReplaySafety::NonReplayable => {
                    return Err(Box::new(AmbiguousSubmissionError {
                        reason: error.to_string(),
                    }))
                }
                Err(error) => return Err(Box::new(error)),
                Ok(Err(error))
                    if attempted
                        && attempt < attempts
                        && error.downcast_ref::<reqwest::Error>().is_some() =>
                {
                    options
                        .operation
                        .sleep(options.retry.delay(attempt))
                        .await?;
                    continue;
                }
                Ok(Err(error))
                    if attempted
                        && options.safety == ReplaySafety::NonReplayable
                        && error.downcast_ref::<reqwest::Error>().is_some() =>
                {
                    return Err(Box::new(AmbiguousSubmissionError {
                        reason: error.to_string(),
                    }))
                }
                Ok(Err(error)) => return Err(error),
            };
            if attempt < attempts && matches!(response.status().as_u16(), 429 | 503) {
                let backoff = options.retry.delay(attempt);
                let delay = match response.headers().get(reqwest::header::RETRY_AFTER) {
                    Some(value) => {
                        retry_after(value.to_str()?, std::time::SystemTime::now())?.max(backoff)
                    }
                    None => backoff,
                };
                drop(response);
                options.operation.sleep(delay).await?;
                continue;
            }
            let result = options
                .operation
                .run(response_text(response, options.limits))
                .await;
            return match result {
                Ok(Ok(body)) => Ok(body),
                Ok(Err(error))
                    if options.safety == ReplaySafety::NonReplayable
                        && error.downcast_ref::<reqwest::Error>().is_some() =>
                {
                    Err(Box::new(AmbiguousSubmissionError {
                        reason: error.to_string(),
                    }))
                }
                Ok(Err(error)) => Err(error),
                Err(error) if options.safety == ReplaySafety::NonReplayable => {
                    Err(Box::new(AmbiguousSubmissionError {
                        reason: error.to_string(),
                    }))
                }
                Err(error) => Err(Box::new(error)),
            };
        }
        Err("retry attempts exhausted".into())
    }

    /// Shared API-relative request construction. Service-specific callers can
    /// add headers, streaming bodies or custom methods without losing policies.
    /// SDK-owned clients apply one configured Basic or bearer credential.
    /// Caller-owned clients retain their policy; configured credentials require
    /// [`with_client_and_configured_auth`](Self::with_client_and_configured_auth).
    /// With a token provider, use
    /// [`authorized_request_builder`](Self::authorized_request_builder) instead;
    /// this synchronous builder returns an error rather than sending anonymously.
    pub fn request_builder(
        &self,
        method: reqwest::Method,
        endpoint: &str,
    ) -> Result<reqwest::RequestBuilder, Box<dyn Error>> {
        self.request_builder_inner(method, endpoint, false)
    }

    fn request_builder_inner(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        allow_provider: bool,
    ) -> Result<reqwest::RequestBuilder, Box<dyn Error>> {
        if self.config.token_provider.is_some() {
            let sources = self.config.static_credentials();
            if !sources.is_empty() {
                return Err(Box::new(AuthConflictError { sources }));
            }
        }
        let auth = self.config.configured_auth()?;
        let has_provider = self.config.token_provider.is_some();
        if !self.apply_configured_auth && (has_provider || !matches!(auth, ConfiguredAuth::None)) {
            return Err(Box::new(AuthError::CallerOwnedClient));
        }
        if has_provider && !allow_provider {
            return Err(Box::new(AuthError::ProviderRequiresAsyncBuilder));
        }
        let mut request = self.client.request(method, self.endpoint(endpoint)?);
        if let Some(agent) = &self.config.user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        if self.apply_configured_auth {
            request = match auth {
                ConfiguredAuth::None => request,
                ConfiguredAuth::Basic(basic) => {
                    request.basic_auth(&basic.username, basic.password.as_deref())
                }
                ConfiguredAuth::Bearer(token) => request.bearer_auth(token),
            };
        }
        Ok(request)
    }

    /// Like [`request_builder`](Self::request_builder), with the token of the
    /// configured token provider, if there is one (see [`crate::utils::auth`]).
    ///
    /// The request is sent once, as the caller sends it; there is no further
    /// attempt after `401 Unauthorized`.
    ///
    /// # Errors
    ///
    /// An [`EndpointError`], an [`AuthConflictError`] if the configuration has
    /// other credentials as well, or a [`TokenProviderError`]. Nothing has been
    /// sent then. An injected client also requires explicit opt-in.
    pub async fn authorized_request_builder(
        &self,
        method: reqwest::Method,
        endpoint: &str,
    ) -> Result<reqwest::RequestBuilder, Box<dyn Error>> {
        let request = self.request_builder_inner(method, endpoint, true)?;
        self.authorize(request, false).await
    }

    /// Adds the provider's token to `request`, a new one if `after_rejection`.
    /// Without a provider, `request` stays as it is.
    async fn authorize(
        &self,
        request: reqwest::RequestBuilder,
        after_rejection: bool,
    ) -> Result<reqwest::RequestBuilder, Box<dyn Error>> {
        let Some(source) = &self.config.token_provider else {
            return Ok(request);
        };
        let audience = self.config.token_audience();
        let token = source.token(&audience, after_rejection).await?;
        let invalid = || TokenProviderError::InvalidToken {
            audience: audience.clone(),
        };
        let mut value =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.secret()))
                .map_err(|_| invalid())?;
        value.set_sensitive(true);
        Ok(request.header(reqwest::header::AUTHORIZATION, value))
    }

    /// Tells the token provider's reused tokens that the service answered
    /// `401 Unauthorized` to a request with these `headers`, if they carry a
    /// bearer token and a provider is configured.
    fn reject_token(&self, headers: &reqwest::header::HeaderMap) {
        let Some(source) = &self.config.token_provider else {
            return;
        };
        let secret = headers
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "));
        if let Some(secret) = secret {
            source.reject(&self.config.token_audience(), secret);
        }
    }

    /// Sends `request`, and tells the reused tokens about a `401 Unauthorized`.
    async fn send_checked(
        &self,
        request: reqwest::RequestBuilder,
        sent: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<reqwest::Response, Box<dyn Error>> {
        let (client, request) = request.build_split();
        let request = request?;
        let headers = request.headers().clone();
        if let Some(sent) = sent {
            sent.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let response = client.execute(request).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            self.reject_token(&headers);
        }
        Ok(response)
    }

    /// Builds a request with [`authorized_request_builder`](Self::authorized_request_builder)
    /// and `build`, and sends it. If it is `replayable`, a token provider is
    /// configured and the service answers `401 Unauthorized`, builds and sends it
    /// once more with a current token. Another concurrent request may already
    /// have refreshed the token. `sent`, if given, is set just before the first
    /// request is sent.
    pub(crate) async fn send_authorized(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        replayable: bool,
        build: impl Fn(reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, Box<dyn Error>>,
        sent: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<reqwest::Response, Box<dyn Error>> {
        let request = build(
            self.authorized_request_builder(method.clone(), endpoint)
                .await?,
        )?;
        let response = self.send_checked(request, sent).await?;
        if !replayable
            || response.status() != reqwest::StatusCode::UNAUTHORIZED
            || self.config.token_provider.is_none()
        {
            return Ok(response);
        }
        drop(response);
        log::debug!("Retrying a read after 401 Unauthorized with the current token");
        let request = self.request_builder_inner(method, endpoint, true)?;
        // send_checked already rejected the token used by the first request.
        // Do not force another rejection here: a concurrent request may have
        // installed a newer reusable token before this retry.
        let request = build(self.authorize(request, false).await?)?;
        self.send_checked(request, None).await
    }

    /// Resolves a relative path beneath the configured API root, with or without a trailing slash.
    /// A leading slash in `path` is treated as API-relative, too. Absolute URLs and path
    /// traversal are rejected before a request is sent.
    pub fn endpoint(&self, path: &str) -> Result<Url, EndpointError> {
        let mut base = self.config.base_path.clone();
        if base.cannot_be_a_base() || base.query().is_some() || base.fragment().is_some() {
            return Err(EndpointError::InvalidBase);
        }
        let relative = path.trim_start_matches('/');
        let path_part = relative.split(['?', '#']).next().unwrap_or_default();
        if path.starts_with("//")
            || Url::parse(path).is_ok()
            || path.contains('\\')
            || path_part.split('/').any(is_dot_segment)
        {
            return Err(EndpointError::InvalidPath);
        }
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        let url = base.join(relative)?;
        if url.origin() != base.origin() || !url.path().starts_with(base.path()) {
            return Err(EndpointError::OutsideApiRoot);
        }
        Ok(url)
    }

    /// Sends one request and returns status/headers before decoding its body.
    /// Non-2xx responses are also returned unchanged. Callers must interpret the
    /// status and either consume bounded bytes, stream chunks, or drop it.
    ///
    /// # Arguments
    ///
    /// * `method` - The HTTP method for the request.
    /// * `endpoint` - The endpoint for the request.
    /// * `data` - The data to send with the request (optional).
    /// * `params` - The query parameters for the request (optional), encoded as
    ///   described in [`crate::utils::query`]. Unsupported values fail with an
    ///   [`std::io::ErrorKind::InvalidInput`] error before anything is sent.
    ///
    /// # Returns
    ///
    /// A raw response, or an endpoint/transport error. With a token provider
    /// (see [`crate::utils::auth`]), a `GET` or `HEAD` request is sent once
    /// more with a new token after `401 Unauthorized`; there is no other replay.
    pub async fn request_response(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
    ) -> Result<reqwest::Response, Box<dyn Error>> {
        let replayable = matches!(method, reqwest::Method::GET | reqwest::Method::HEAD);
        self.request_response_with(method, endpoint, data, params, replayable, None)
            .await
    }

    async fn request_response_with(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
        replayable: bool,
        sent: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<reqwest::Response, Box<dyn Error>> {
        let pairs = match params {
            Some(ref params_value) => query_pairs(params_value).map_err(std::io::Error::from)?,
            None => Vec::new(),
        };
        self.send_authorized(
            method,
            endpoint,
            replayable,
            |mut request_builder| {
                if !pairs.is_empty() {
                    request_builder = request_builder.query(&pairs);
                }
                if let Some(ref data) = data {
                    if serde_json::to_string(&data).is_ok() {
                        request_builder = request_builder.json(&data);
                    } else {
                        log::error!("Parameters are invalid, and can't convert to JSON");
                    }
                }
                Ok(request_builder)
            },
            sent,
        )
        .await
    }

    /// Text helper with caller-selected success/error bounds. With a token
    /// provider, a `GET` or `HEAD` may retry once after `401 Unauthorized`.
    pub async fn request_text(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
        limits: BodyLimits,
    ) -> Result<String, Box<dyn Error>> {
        response_text(
            self.request_response(method, endpoint, data, params)
                .await?,
            limits,
        )
        .await
    }

    async fn request(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
    ) -> Result<String, Box<dyn Error>> {
        self.request_text(method, endpoint, data, params, BodyLimits::default())
            .await
    }

    /// Sends a GET request to the specified endpoint with the given query parameters.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The endpoint for the request.
    /// * `params` - The query parameters for the request (optional), see
    ///   [`Transport::request_response`].
    ///
    /// # Returns
    ///
    /// A `Result` containing the response body as a string, or an error if the request fails.
    pub async fn get(
        &self,
        endpoint: &str,
        params: Option<Value>,
    ) -> Result<String, Box<dyn Error>> {
        self.request(reqwest::Method::GET, endpoint, None, params)
            .await
    }

    /// Sends a POST request to the specified endpoint with the given data.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The endpoint for the request.
    /// * `data` - The data to send with the request (optional).
    ///
    /// # Returns
    ///
    /// A `Result` containing the response body as a string, or an error if the request fails.
    pub async fn post(
        &self,
        endpoint: &str,
        data: Option<Value>,
    ) -> Result<String, Box<dyn Error>> {
        self.request(reqwest::Method::POST, endpoint, data, None)
            .await
    }

    /// Sends a multipart form, as required by APIs such as WES RunWorkflow.
    pub async fn post_multipart(
        &self,
        endpoint: &str,
        form: Form,
    ) -> Result<String, Box<dyn Error>> {
        let request = self
            .authorized_request_builder(reqwest::Method::POST, endpoint)
            .await?
            .multipart(form);
        let response = self.send_checked(request, None).await?;
        response_text(response, BodyLimits::default()).await
    }

    /// Sends a PUT request to the specified endpoint with the given data.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The endpoint for the request.
    /// * `data` - The data to send with the request.
    ///
    /// # Returns
    ///
    /// A `Result` containing the response body as a string, or an error if the request fails.
    pub async fn put(&self, endpoint: &str, data: Value) -> Result<String, Box<dyn Error>> {
        self.request(reqwest::Method::PUT, endpoint, Some(data), None)
            .await
    }

    /// Sends a DELETE request to the specified endpoint.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The endpoint for the request.
    ///
    /// # Returns
    ///
    /// A `Result` containing the response body as a string, or an error if the request fails.
    pub async fn delete(&self, endpoint: &str) -> Result<String, Box<dyn Error>> {
        self.request(reqwest::Method::DELETE, endpoint, None, None)
            .await
    }

    // other HTTP methods can be added here
}

#[cfg(test)]
mod tests {
    use crate::utils::configuration::Configuration;
    use crate::utils::transport::{urlencode, EndpointError, HttpStatusError, Transport};
    use mockito::{mock, Matcher};
    use url::Url;

    // effectively no sense in testing various responses, as it's reqwest's responsibility
    // we should test Transport's methods instead

    #[test]
    fn test_debug_output() {
        let config = crate::utils::configuration::config_with_markers();
        let output = format!("{:?}", Transport::new(&config));
        assert_eq!(output, format!("Transport {{ config: {:?}, .. }}", config));
        assert!(!output.contains("SENTINEL"), "{output}");
    }

    #[test]
    fn test_http_status_error_keeps_body_but_omits_it_from_diagnostics() {
        let body = "Bearer SENTINEL_TOKEN\n".repeat(301);
        let error = HttpStatusError {
            status: reqwest::StatusCode::BAD_GATEWAY,
            body: body.clone(),
        };
        assert_eq!(error.body, body);
        assert_eq!(
            error.to_string(),
            "Request failed with status: 502 Bad Gateway"
        );
        let debug = format!("{error:?}");
        assert!(debug.contains("502"), "{debug}");
        assert!(!debug.contains("SENTINEL_TOKEN"), "{debug}");
    }

    #[test]
    fn test_endpoint_uses_api_root_with_or_without_trailing_slash() {
        for suffix in ["", "/"] {
            let base = Url::parse(&format!("https://example.org/a%20b/tes/v1{suffix}")).unwrap();
            let transport = Transport::new(&Configuration::new(base));
            for path in ["tasks/a%2Fb", "/tasks/a%2Fb"] {
                assert_eq!(
                    transport.endpoint(path).unwrap().as_str(),
                    "https://example.org/a%20b/tes/v1/tasks/a%2Fb"
                );
            }
        }
        assert_eq!(urlencode("a/b ?#"), "a%2Fb%20%3F%23");
    }

    #[test]
    fn test_endpoint_rejects_absolute_urls_and_dot_segments() {
        let base = Url::parse("https://example.org/ga4gh/tes/v1").unwrap();
        let transport = Transport::new(&Configuration::new(base));
        for path in [
            "https://other.example/tasks",
            "https://example.org/ga4gh/tes/v1/tasks",
            "//other.example/tasks",
            "other:tasks",
            "tasks/.",
            "tasks/..",
            "tasks/../secret",
            "tasks/%2e",
            "tasks/%2E%2e",
            "tasks/.%2E",
            "tasks/%2e.",
            "tasks/..?view=FULL",
            "tasks\\..\\secret",
        ] {
            assert!(
                matches!(transport.endpoint(path), Err(EndpointError::InvalidPath)),
                "{path} should be rejected"
            );
        }
        for id in [".", ".."] {
            for prefix in ["objects", "tasks", "runs"] {
                let path = format!("{prefix}/{}", urlencode(id));
                assert!(matches!(
                    transport.endpoint(&path),
                    Err(EndpointError::InvalidPath)
                ));
            }
        }
        assert_eq!(
            transport.endpoint("tasks/a..b?view=FULL").unwrap().as_str(),
            "https://example.org/ga4gh/tes/v1/tasks/a..b?view=FULL"
        );
    }

    #[test]
    fn test_endpoint_rejects_query_or_fragment_in_api_root() {
        for suffix in ["?key=secret", "#fragment"] {
            let base = Url::parse(&format!("https://example.org/ga4gh/tes/v1{suffix}")).unwrap();
            let transport = Transport::new(&Configuration::new(base));
            assert!(matches!(
                transport.endpoint("tasks"),
                Err(EndpointError::InvalidBase)
            ));
        }
    }

    #[tokio::test]
    async fn test_request() {
        let base_url_str = mockito::server_url();
        let base_url = Url::parse(&base_url_str).expect("Failed to parse mock server URL");

        let _m = mock("GET", "/test")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"message": "success"}"#)
            .create();

        let config = Configuration::new(base_url.clone());
        let transport = Transport::new(&config.clone());
        let response = transport.get("/test", None).await;

        assert!(response.is_ok());
        let body = response.unwrap();
        assert_eq!(body, r#"{"message": "success"}"#);
    }

    #[tokio::test]
    async fn test_repeated_query_parameters() {
        let _request = mock("GET", "/tasks")
            .match_query(Matcher::Exact(
                "page_size=10&tag_key=team+a&tag_key=other&tag_value=x%2Fy&tag_value=z".to_owned(),
            ))
            .with_status(200)
            .with_body("ok")
            .create();
        let config = Configuration::new(Url::parse(&mockito::server_url()).unwrap());
        let params = serde_json::json!({
            "tag_key": ["team a", "other"],
            "tag_value": ["x/y", "z"],
            "page_size": 10,
        });
        let response = Transport::new(&config)
            .get("tasks", Some(params))
            .await
            .unwrap();
        assert_eq!(response, "ok");
    }

    mod token_provider {
        use crate::utils::auth::{
            AccessToken, AuthConflictError, BoxError, TokenProvider, TokenProviderError,
            TokenRequest,
        };
        use crate::utils::configuration::{AuthError, Configuration};
        use crate::utils::operation::ReplaySafety;
        use crate::utils::transport::{HttpError, RequestOptions, Transport};
        use mockito::{mock, Matcher};
        use std::error::Error;
        use std::sync::{Arc, Mutex};
        use std::time::{Duration, Instant};
        use url::Url;

        /// Gives `token-<n>` for the n-th call and records the requests.
        #[derive(Default)]
        struct Recording {
            requests: Mutex<Vec<(String, bool)>>,
            lifetime: Option<Duration>,
            fail: bool,
            token: Option<&'static str>,
            delay: Option<Duration>,
        }

        #[async_trait::async_trait]
        impl TokenProvider for Recording {
            async fn token(&self, request: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
                if let Some(delay) = self.delay {
                    tokio::time::sleep(delay).await;
                }
                let mut requests = self.requests.lock().unwrap();
                requests.push((request.audience.to_owned(), request.after_rejection));
                if self.fail {
                    return Err("no session for SENTINEL_USER".into());
                }
                let token = match self.token {
                    Some(token) => AccessToken::new(token),
                    None => AccessToken::new(format!("token-{}", requests.len())),
                };
                Ok(match self.lifetime {
                    Some(lifetime) => token.reusable_until(Instant::now() + lifetime),
                    None => token,
                })
            }
        }

        impl Recording {
            fn calls(&self) -> Vec<(String, bool)> {
                self.requests.lock().unwrap().clone()
            }
        }

        fn base(root: &str) -> Url {
            Url::parse(&format!("{}/{root}", mockito::server_url())).unwrap()
        }

        /// A transport below `/<root>` of the mock server, with `provider`.
        fn transport(root: &str, provider: &Arc<Recording>) -> Transport {
            Transport::new(
                &Configuration::new(base(root)).with_shared_token_provider(provider.clone()),
            )
        }

        #[tokio::test]
        async fn test_sends_the_token_for_every_method() {
            let provider = Arc::new(Recording::default());
            let transport = transport("auth-methods", &provider);
            let mocks = [
                mock("GET", "/auth-methods/a").match_header("authorization", "Bearer token-1"),
                mock("POST", "/auth-methods/b").match_header("authorization", "Bearer token-2"),
                mock("PUT", "/auth-methods/c").match_header("authorization", "Bearer token-3"),
                mock("DELETE", "/auth-methods/d").match_header("authorization", "Bearer token-4"),
                mock("POST", "/auth-methods/e").match_header("authorization", "Bearer token-5"),
            ]
            .map(|mock| mock.with_status(200).with_body("ok").expect(1).create());
            transport.get("a", None).await.unwrap();
            transport.post("b", None).await.unwrap();
            transport.put("c", serde_json::json!({})).await.unwrap();
            transport.delete("d").await.unwrap();
            let form = reqwest::multipart::Form::new().text("x", "y");
            transport.post_multipart("e", form).await.unwrap();
            for mock in mocks {
                mock.assert();
            }
            let expected = vec![(base("auth-methods").to_string(), false); 5];
            assert_eq!(provider.calls(), expected);
        }

        #[tokio::test]
        async fn test_sends_no_header_without_a_provider() {
            let request = mock("GET", "/auth-none/a")
                .match_header("authorization", Matcher::Missing)
                .with_status(200)
                .expect(1)
                .create();
            Transport::new(&Configuration::new(base("auth-none")))
                .get("a", None)
                .await
                .unwrap();
            request.assert();
        }

        #[tokio::test]
        async fn test_configured_audience_and_reuse() {
            let provider = Arc::new(Recording {
                lifetime: Some(Duration::from_secs(60)),
                ..Default::default()
            });
            let config = Configuration::new(base("auth-reuse"))
                .with_shared_token_provider(provider.clone())
                .with_token_audience("tes".to_owned());
            let request = mock("GET", "/auth-reuse/a")
                .match_header("authorization", "Bearer token-1")
                .with_status(200)
                .expect(2)
                .create();
            // Transports of clones of a configuration share reused tokens.
            Transport::new(&config).get("a", None).await.unwrap();
            Transport::new(&config.clone())
                .get("a", None)
                .await
                .unwrap();
            request.assert();
            assert_eq!(provider.calls(), [("tes".to_owned(), false)]);
        }

        #[tokio::test]
        async fn test_replays_a_get_once_after_401() {
            let provider = Arc::new(Recording {
                lifetime: Some(Duration::from_secs(60)),
                ..Default::default()
            });
            let transport = transport("auth-replay", &provider);
            let rejected = mock("GET", "/auth-replay/a")
                .match_header("authorization", "Bearer token-1")
                .with_status(401)
                .expect(1)
                .create();
            let accepted = mock("GET", "/auth-replay/a")
                .match_header("authorization", "Bearer token-2")
                .with_status(200)
                .with_body("ok")
                .expect(2)
                .create();
            assert_eq!(transport.get("a", None).await.unwrap(), "ok");
            // The new token is reused.
            assert_eq!(transport.get("a", None).await.unwrap(), "ok");
            rejected.assert();
            accepted.assert();
            let root = base("auth-replay").to_string();
            assert_eq!(provider.calls(), [(root.clone(), false), (root, true)]);
        }

        #[tokio::test]
        async fn late_401_does_not_evict_a_newer_reusable_token() {
            use tokio::{
                io::{AsyncReadExt, AsyncWriteExt},
                net::TcpListener,
                sync::Notify,
            };

            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let late_started = Arc::new(Notify::new());
            let release_late = Arc::new(Notify::new());
            let seen = Arc::new(Mutex::new(Vec::new()));
            let server = tokio::spawn({
                let late_started = late_started.clone();
                let release_late = release_late.clone();
                let seen = seen.clone();
                async move {
                    loop {
                        let (mut socket, _) = listener.accept().await.unwrap();
                        let late_started = late_started.clone();
                        let release_late = release_late.clone();
                        let seen = seen.clone();
                        tokio::spawn(async move {
                            let mut bytes = Vec::new();
                            let mut chunk = [0; 1024];
                            loop {
                                let count = socket.read(&mut chunk).await.unwrap();
                                if count == 0 {
                                    return;
                                }
                                bytes.extend_from_slice(&chunk[..count]);
                                if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
                                    break;
                                }
                            }
                            let request = String::from_utf8(bytes).unwrap();
                            let path = request.split_whitespace().nth(1).unwrap();
                            let authorization = request
                                .lines()
                                .filter_map(|line| line.split_once(':'))
                                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                                .map(|(_, value)| value.trim())
                                .unwrap_or("");
                            seen.lock().unwrap().push(format!("{path} {authorization}"));
                            if path == "/auth-race/late" && authorization == "Bearer token-1" {
                                late_started.notify_one();
                                release_late.notified().await;
                            }
                            let response = match (path, authorization) {
                                ("/auth-race/late", "Bearer token-1")
                                | ("/auth-race/fast", "Bearer token-1") => {
                                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                                }
                                ("/auth-race/late", "Bearer token-2")
                                | ("/auth-race/fast", "Bearer token-2") => {
                                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok"
                                }
                                _ => "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            };
                            socket.write_all(response.as_bytes()).await.unwrap();
                        });
                    }
                }
            });

            let provider = Arc::new(Recording {
                lifetime: Some(Duration::from_secs(60)),
                ..Default::default()
            });
            let config =
                Configuration::new(Url::parse(&format!("http://{address}/auth-race")).unwrap())
                    .with_shared_token_provider(provider.clone());
            let transport = Arc::new(Transport::new(&config));
            // Cache token-1 before both reads start.
            let _ = transport
                .authorized_request_builder(reqwest::Method::GET, "late")
                .await
                .unwrap();
            let (late, ()) = tokio::time::timeout(Duration::from_secs(10), async {
                tokio::join!(transport.get("late", None), async {
                    late_started.notified().await;
                    assert_eq!(transport.get("fast", None).await.unwrap(), "ok");
                    release_late.notify_one();
                })
            })
            .await
            .unwrap();
            assert_eq!(late.unwrap(), "ok");
            assert_eq!(provider.calls().len(), 2, "the newer token must be reused");
            assert_eq!(seen.lock().unwrap().len(), 4);
            server.abort();
        }

        #[tokio::test]
        async fn test_replays_only_once() {
            let provider = Arc::new(Recording::default());
            let transport = transport("auth-twice", &provider);
            let rejected = mock("GET", "/auth-twice/a")
                .with_status(401)
                .with_body("denied")
                .expect(2)
                .create();
            let error = transport.get("a", None).await.unwrap_err();
            let error = error.downcast_ref::<HttpError>().unwrap();
            assert_eq!(error.status, reqwest::StatusCode::UNAUTHORIZED);
            rejected.assert();
            assert_eq!(provider.calls().len(), 2);
        }

        #[tokio::test]
        async fn test_never_replays_a_submission() {
            let provider = Arc::new(Recording::default());
            let transport = transport("auth-submit", &provider);
            let rejected = mock("POST", "/auth-submit/a")
                .with_status(401)
                .expect(2)
                .create();
            let error = transport.post("a", None).await.unwrap_err();
            assert_eq!(
                error.downcast_ref::<HttpError>().unwrap().status,
                reqwest::StatusCode::UNAUTHORIZED
            );
            let form = reqwest::multipart::Form::new().text("x", "y");
            let error = transport.post_multipart("a", form).await.unwrap_err();
            assert!(error.downcast_ref::<HttpError>().is_some(), "{error}");
            rejected.assert();
            assert_eq!(provider.calls().len(), 2);
        }

        #[tokio::test]
        async fn test_replay_follows_the_replay_safety() {
            let provider = Arc::new(Recording::default());
            let transport = transport("auth-policy", &provider);
            let rejected = mock("POST", "/auth-policy/lookup")
                .match_header("authorization", "Bearer token-1")
                .with_status(401)
                .expect(1)
                .create();
            let accepted = mock("POST", "/auth-policy/lookup")
                .match_header("authorization", "Bearer token-2")
                .with_status(200)
                .with_body("ok")
                .expect(1)
                .create();
            let mut options = RequestOptions::new(Duration::from_secs(10));
            options.safety = ReplaySafety::ReadOnly;
            let method = reqwest::Method::POST;
            let body = transport
                .request_with_policy(method.clone(), "lookup", None, None, &options)
                .await
                .unwrap();
            assert_eq!(body, "ok");
            rejected.assert();
            accepted.assert();

            let submission = mock("POST", "/auth-policy/submit")
                .with_status(401)
                .expect(1)
                .create();
            options.safety = ReplaySafety::NonReplayable;
            let error = transport
                .request_with_policy(method, "submit", None, None, &options)
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<HttpError>().is_some(), "{error}");
            submission.assert();
        }

        #[tokio::test]
        async fn test_rejected_submission_drops_the_reused_token() {
            let provider = Arc::new(Recording {
                lifetime: Some(Duration::from_secs(60)),
                ..Default::default()
            });
            let transport = transport("auth-drop", &provider);
            let rejected = mock("POST", "/auth-drop/a")
                .match_header("authorization", "Bearer token-1")
                .with_status(401)
                .expect(1)
                .create();
            let accepted = mock("POST", "/auth-drop/a")
                .match_header("authorization", "Bearer token-2")
                .with_status(200)
                .expect(1)
                .create();
            assert!(transport.post("a", None).await.is_err());
            transport.post("a", None).await.unwrap();
            rejected.assert();
            accepted.assert();
            let root = base("auth-drop").to_string();
            assert_eq!(provider.calls(), [(root.clone(), false), (root, true)]);

            // The same after `send_once`, as TES and WES submissions use it.
            let rejected = mock("POST", "/auth-drop/once")
                .match_header("authorization", "Bearer token-2")
                .with_status(401)
                .expect(1)
                .create();
            let operation = crate::utils::operation::Operation::new(Duration::from_secs(10));
            let request = transport
                .authorized_request_builder(reqwest::Method::POST, "once")
                .await
                .unwrap();
            let error = transport
                .send_once(request, &operation, Default::default())
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<HttpError>().is_some(), "{error}");
            rejected.assert();
            let _ = transport
                .authorized_request_builder(reqwest::Method::POST, "once")
                .await
                .unwrap();
            assert_eq!(provider.calls().len(), 3);
            assert!(provider.calls()[2].1);

            // And after `post_multipart`.
            let rejected = mock("POST", "/auth-drop/form")
                .match_header("authorization", "Bearer token-3")
                .with_status(401)
                .expect(1)
                .create();
            let form = reqwest::multipart::Form::new().text("x", "y");
            assert!(transport.post_multipart("form", form).await.is_err());
            rejected.assert();
            let _ = transport
                .authorized_request_builder(reqwest::Method::POST, "form")
                .await
                .unwrap();
            assert_eq!(provider.calls().len(), 4);
            assert!(provider.calls()[3].1);
        }

        #[tokio::test]
        async fn test_slow_provider_is_not_an_ambiguous_submission() {
            use crate::utils::operation::{AmbiguousSubmissionError, OperationError};
            let provider = Arc::new(Recording {
                delay: Some(Duration::from_secs(5)),
                ..Default::default()
            });
            let transport = transport("auth-slow", &provider);
            let request = mock("POST", "/auth-slow/a").expect(0).create();
            let options = RequestOptions::new(Duration::from_millis(50));
            let error = transport
                .request_with_policy(reqwest::Method::POST, "a", None, None, &options)
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<AmbiguousSubmissionError>().is_none());
            assert!(matches!(
                error.downcast_ref::<OperationError>(),
                Some(OperationError::Deadline)
            ));
            request.assert();
        }

        #[tokio::test]
        async fn test_provider_failure_sends_nothing() {
            let provider = Arc::new(Recording {
                fail: true,
                ..Default::default()
            });
            let transport = transport("auth-fail", &provider);
            let request = mock("GET", "/auth-fail/a").expect(0).create();
            let error = transport.get("a", None).await.unwrap_err();
            let message = error.to_string();
            let error = error.downcast_ref::<TokenProviderError>().unwrap();
            assert!(matches!(error, TokenProviderError::Failed { .. }));
            assert_eq!(
                message,
                "The token provider failed before the request was sent"
            );
            assert!(!format!("{error:?}").contains("SENTINEL_USER"));
            assert!(error
                .source()
                .unwrap()
                .to_string()
                .contains("SENTINEL_USER"));
            request.assert();
        }

        #[tokio::test]
        async fn test_invalid_tokens_are_not_sent_or_shown() {
            for token in ["", "SENTINEL\nTOKEN", "SENTINEL TOKEN"] {
                let provider = Arc::new(Recording {
                    token: Some(token),
                    lifetime: Some(Duration::from_secs(60)),
                    ..Default::default()
                });
                let transport = transport("auth-invalid", &provider);
                let request = mock("GET", "/auth-invalid/a").expect(0).create();
                let error = transport.get("a", None).await.unwrap_err();
                assert!(
                    !format!("{error} {error:?}").contains("SENTINEL"),
                    "{error:?}"
                );
                assert!(matches!(
                    error.downcast_ref::<TokenProviderError>().unwrap(),
                    TokenProviderError::InvalidToken { .. }
                ));
                assert!(transport.get("a", None).await.is_err());
                assert_eq!(
                    provider.calls().len(),
                    2,
                    "invalid tokens must not be cached"
                );
                request.assert();
            }
        }

        #[tokio::test]
        async fn test_conflicting_credentials_send_nothing() {
            let provider = Arc::new(Recording::default());
            let request = mock("GET", "/auth-conflict/a").expect(0).create();
            let config = Configuration::new(base("auth-conflict"))
                .with_shared_token_provider(provider.clone())
                .with_oauth_access_token("SENTINEL_OAUTH".to_owned());
            let error = Transport::new(&config).get("a", None).await.unwrap_err();
            assert!(!format!("{error} {error:?}").contains("SENTINEL"));
            assert_eq!(
                error.downcast_ref::<AuthConflictError>().unwrap().sources,
                ["oauth_access_token"]
            );
            let mut url = base("auth-conflict");
            url.set_username("alice").unwrap();
            let config = Configuration::new(url).with_shared_token_provider(provider.clone());
            let error = Transport::new(&config).get("a", None).await.unwrap_err();
            assert!(error.downcast_ref::<AuthConflictError>().is_some());
            request.assert();
            assert!(provider.calls().is_empty());
        }

        #[tokio::test]
        async fn test_provider_header_replaces_a_client_default() {
            let provider = Arc::new(Recording::default());
            let request = mock("GET", "/auth-client/a")
                .match_header("authorization", "Bearer token-1")
                .match_header("x-client", "kept")
                .with_status(200)
                .expect(1)
                .create();
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert("authorization", "Bearer client".parse().unwrap());
            headers.insert("x-client", "kept".parse().unwrap());
            let client = reqwest::Client::builder()
                .default_headers(headers)
                .build()
                .unwrap();
            let config =
                Configuration::new(base("auth-client")).with_shared_token_provider(provider);
            let error = Transport::with_client(&config, client.clone())
                .get("a", None)
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<AuthError>().is_some());
            Transport::with_client_and_configured_auth(&config, client)
                .get("a", None)
                .await
                .unwrap();
            request.assert();
        }

        #[tokio::test]
        async fn test_authorized_request_builder_sends_once() {
            let provider = Arc::new(Recording::default());
            let transport = transport("auth-builder", &provider);
            let request = mock("PATCH", "/auth-builder/a")
                .match_header("authorization", "Bearer token-1")
                .with_status(401)
                .expect(1)
                .create();
            let response = transport
                .authorized_request_builder(reqwest::Method::PATCH, "a")
                .await
                .unwrap()
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
            request.assert();
            assert_eq!(provider.calls().len(), 1);
        }

        #[tokio::test]
        async fn synchronous_builder_rejects_a_token_provider() {
            let provider = Arc::new(Recording::default());
            let transport = transport("auth-sync-builder", &provider);
            let error = transport
                .request_builder(reqwest::Method::GET, "a")
                .unwrap_err();
            assert!(matches!(
                error.downcast_ref::<AuthError>(),
                Some(AuthError::ProviderRequiresAsyncBuilder)
            ));
            assert!(provider.calls().is_empty());
        }

        #[tokio::test]
        async fn request_build_failure_is_not_an_ambiguous_submission() {
            use crate::utils::operation::AmbiguousSubmissionError;

            let provider = Arc::new(Recording::default());
            let config = Configuration::new(base("auth-build-error"))
                .with_shared_token_provider(provider.clone())
                .with_user_agent("invalid\nagent".into());
            let transport = Transport::new(&config);
            let request = mock("POST", "/auth-build-error/tasks").expect(0).create();
            let options = RequestOptions::new(Duration::from_secs(2));
            let error = transport
                .request_with_policy(reqwest::Method::POST, "tasks", None, None, &options)
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<reqwest::Error>().is_some(), "{error}");
            assert!(error.downcast_ref::<AmbiguousSubmissionError>().is_none());

            let request_builder = transport
                .authorized_request_builder(reqwest::Method::POST, "tasks")
                .await
                .unwrap();
            let operation = crate::utils::operation::Operation::new(Duration::from_secs(2));
            let error = transport
                .send_once(request_builder, &operation, Default::default())
                .await
                .unwrap_err();
            assert!(error.downcast_ref::<reqwest::Error>().is_some(), "{error}");
            assert!(error.downcast_ref::<AmbiguousSubmissionError>().is_none());
            assert_eq!(provider.calls().len(), 2);
            request.assert();
        }
    }
}
