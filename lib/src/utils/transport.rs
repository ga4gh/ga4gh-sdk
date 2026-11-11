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
use crate::utils::configuration::Configuration;
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
    let limit = if response.status().is_success() {
        limits.success
    } else {
        limits.error
    };
    let response = read_bounded(response, limit).await?;
    let body = String::from_utf8_lossy(&response.body).into_owned();
    if response.status.is_success() {
        Ok(body)
    } else {
        Err(Box::new(HttpError {
            status: response.status,
            body,
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
#[derive(Debug)]
pub struct HttpError {
    pub status: reqwest::StatusCode,
    pub body: String,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MAX_BODY_CHARS: usize = 300;
        let end = self
            .body
            .char_indices()
            .nth(MAX_BODY_CHARS)
            .map_or(self.body.len(), |(index, _)| index);
        write!(
            f,
            "Request failed with status: {}. Response: {}",
            self.status,
            &self.body[..end]
        )?;
        if end < self.body.len() {
            write!(f, "... [truncated]")?;
        }
        Ok(())
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
    pub client: reqwest::Client,
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

    // A client with the connect and request timeouts of `config.timeouts`.
    fn build(config: &Configuration) -> reqwest::Result<Self> {
        Client::builder()
            .connect_timeout(config.timeouts.connect)
            .timeout(config.timeouts.request)
            .build()
            .map(|client| Self::with_client(config, client))
    }

    /// Inject a configured client, for connection budgets, TLS and downstream use.
    ///
    /// The client's own connect and request timeouts apply, and
    /// `config.timeouts.connect` and `request` are not used; the `discovery` and
    /// `submission` limits still come from `config.timeouts`.
    pub fn with_client(config: &Configuration, client: Client) -> Self {
        Self {
            config: config.clone(),
            client,
        }
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
        let request = self
            .request_builder(reqwest::Method::GET, endpoint)?
            .timeout(timeout);
        let operation = crate::utils::operation::Operation::new(timeout);
        operation
            .run(async { response_text(request.send().await?, BodyLimits::default()).await })
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
        let mut attempted = false;
        let result = operation
            .run(async {
                attempted = true;
                response_text(request.send().await?, limits).await
            })
            .await;
        match result {
            Ok(Ok(body)) => Ok(body),
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
            let mut attempted = false;
            let result = options
                .operation
                .run(async {
                    attempted = true;
                    self.request_response(method.clone(), endpoint, data.clone(), params.clone())
                        .await
                })
                .await;
            let response = match result {
                Ok(Ok(response)) => response,
                Err(error) if attempted && options.safety == ReplaySafety::NonReplayable => {
                    return Err(Box::new(AmbiguousSubmissionError {
                        reason: error.to_string(),
                    }))
                }
                Err(error) => return Err(Box::new(error)),
                Ok(Err(error))
                    if attempt < attempts && error.downcast_ref::<reqwest::Error>().is_some() =>
                {
                    options
                        .operation
                        .sleep(options.retry.delay(attempt))
                        .await?;
                    continue;
                }
                Ok(Err(error))
                    if options.safety == ReplaySafety::NonReplayable
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
    /// Authentication remains controlled by the configured/injected client;
    /// see #31 for configuration-based authentication support.
    pub fn request_builder(
        &self,
        method: reqwest::Method,
        endpoint: &str,
    ) -> Result<reqwest::RequestBuilder, EndpointError> {
        let mut request = self.client.request(method, self.endpoint(endpoint)?);
        if let Some(agent) = &self.config.user_agent {
            request = request.header(reqwest::header::USER_AGENT, agent);
        }
        Ok(request)
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
    /// A raw response, or an endpoint/transport error. No automatic replay.
    pub async fn request_response(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
    ) -> Result<reqwest::Response, Box<dyn Error>> {
        let mut request_builder = self.request_builder(method, endpoint)?;

        if let Some(ref params_value) = params {
            let pairs = query_pairs(params_value).map_err(std::io::Error::from)?;
            if !pairs.is_empty() {
                request_builder = request_builder.query(&pairs);
            }
        }

        if let Some(ref data) = data {
            if serde_json::to_string(&data).is_ok() {
                request_builder = request_builder.json(&data);
            } else {
                log::error!("Parameters are invalid, and can't convert to JSON");
            }
        }

        Ok(request_builder.send().await?)
    }

    /// Text helper with caller-selected success/error bounds. No implicit retry.
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
            .request_builder(reqwest::Method::POST, endpoint)?
            .multipart(form);
        let response = request.send().await?;
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
    fn test_http_status_error_truncates_display_but_keeps_body() {
        let body = "é".repeat(301);
        let error = HttpStatusError {
            status: reqwest::StatusCode::BAD_GATEWAY,
            body: body.clone(),
        };
        assert_eq!(error.body, body);
        assert!(error
            .to_string()
            .ends_with(&format!("{}... [truncated]", "é".repeat(300))));

        let short = HttpStatusError {
            status: reqwest::StatusCode::NOT_FOUND,
            body: "service failure".to_string(),
        };
        assert!(short.to_string().ends_with("Response: service failure"));
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
}
