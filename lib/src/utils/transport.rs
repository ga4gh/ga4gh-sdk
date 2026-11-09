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
use log::error;
use reqwest::Client;
use reqwest::multipart::Form;
use serde_json::Value;
use std::error::Error;
use std::fmt;
use url::Url;

/// Percent-encodes one URL path segment, including `/` and spaces.
pub fn urlencode<T: AsRef<str>>(value: T) -> String {
    value.as_ref().as_bytes().iter().map(|byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            (*byte as char).to_string()
        } else {
            format!("%{byte:02X}")
        }
    }).collect()
}

/// An HTTP response whose status is not successful.
#[derive(Debug)]
pub struct HttpStatusError {
    pub status: reqwest::StatusCode,
    pub body: String,
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MAX_BODY_CHARS: usize = 300;
        let end = self.body.char_indices().nth(MAX_BODY_CHARS)
            .map_or(self.body.len(), |(index, _)| index);
        write!(f, "Request failed with status: {}. Response: {}", self.status, &self.body[..end])?;
        if end < self.body.len() {
            write!(f, "... [truncated]")?;
        }
        Ok(())
    }
}

impl Error for HttpStatusError {}

async fn read_response_text(response: reqwest::Response) -> std::io::Result<String> {
    response.text().await.map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to read response text: {error}"),
        )
    })
}

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
                write!(f, "base_path must be a hierarchical URL without a query or fragment")
            }
            Self::InvalidPath => {
                write!(f, "endpoint must be a relative path without dot segments or backslashes")
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

// note: could implement custom certs handling, such as in-TEE generated ephemerial certs
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
    /// # Arguments
    ///
    /// * `config` - The configuration for the transport.
    ///
    /// # Returns
    ///
    /// A new `Transport` instance.
    pub fn new(config: &Configuration) -> Self {
        Transport {
            config: config.clone(),
            client: Client::new(),
        }
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

    /// Sends an HTTP request with the specified method, endpoint, data, and parameters.
    ///
    /// # Arguments
    ///
    /// * `method` - The HTTP method for the request.
    /// * `endpoint` - The endpoint for the request.
    /// * `data` - The data to send with the request (optional).
    /// * `params` - The query parameters for the request (optional).
    ///
    /// # Returns
    ///
    /// A `Result` containing the response body as a string, or an error if the request fails.
    async fn request(
        &self,
        method: reqwest::Method,
        endpoint: &str,
        data: Option<Value>,
        params: Option<Value>,
    ) -> Result<String, Box<dyn Error>> {
        let url = self.endpoint(endpoint)?;

        let mut request_builder = self.client.request(method, url);

        if let Some(ref user_agent) = self.config.user_agent {
            request_builder = request_builder.header(reqwest::header::USER_AGENT, user_agent.clone());
        }

        if let Some(ref params_value) = params {
            // Validate or log params_value before setting it as query parameters
            if params_value.is_object() {
                request_builder = request_builder.query(params_value);
            } else {
                error!("params_value is not an object and cannot be used as query parameters: {:?}", params_value);
                return Err(Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, "params_value must be an object")));
            }
        }

        if let Some(ref data) = data {
            if serde_json::to_string(&data).is_ok() {
                request_builder = request_builder.json(&data);
            } else {
                log::error!("Parameters are invalid, and can't convert to JSON");
            }
        }

        let resp = request_builder.send().await.map_err(|e| {
	            eprintln!("HTTP request failed: {}", e);
	            e
	        })?;

        let status = resp.status();
        let content = read_response_text(resp).await?;

        if status.is_success() {
            Ok(content)
        } else {
            Err(Box::new(HttpStatusError { status, body: content }))
        }
    }

    /// Sends a GET request to the specified endpoint with the given query parameters.
    ///
    /// # Arguments
    ///
    /// * `endpoint` - The endpoint for the request.
    /// * `params` - The query parameters for the request (optional).
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
        let url = self.endpoint(endpoint)?;
        let mut request = self.client.post(url).multipart(form);
        if let Some(user_agent) = &self.config.user_agent {
            request = request.header(reqwest::header::USER_AGENT, user_agent);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = read_response_text(response).await?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(Box::new(HttpStatusError { status, body }))
        }
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
    use crate::utils::transport::{EndpointError, HttpStatusError, Transport, urlencode};
    use mockito::mock;
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
        let error = HttpStatusError { status: reqwest::StatusCode::BAD_GATEWAY, body: body.clone() };
        assert_eq!(error.body, body);
        assert!(error.to_string().ends_with(&format!("{}... [truncated]", "é".repeat(300))));

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
                assert!(matches!(transport.endpoint(&path), Err(EndpointError::InvalidPath)));
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
            assert!(matches!(transport.endpoint("tasks"), Err(EndpointError::InvalidBase)));
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
}
