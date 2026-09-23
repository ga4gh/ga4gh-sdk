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
/// let response = transport.get("/users", None).await;
/// };
///
/// // Make a POST request
/// async {
///     let data = serde_json::json!({"name": "John Doe", "age": 30});
///     let response = transport.post("/users", Some(data)).await;
/// };
/// // Make a PUT request
/// async {
/// let data = serde_json::json!({"name": "John Doe", "age": 30});
/// let response = transport.put("/users/1", data).await;
/// };
///
/// // Make a DELETE request
/// async {
/// let response = transport.delete("/users/1").await;
/// };
/// ```
use crate::utils::configuration::Configuration;
use log::error;
use reqwest::Client;
use reqwest::multipart::Form;
use serde_json::Value;
use std::error::Error;

// note: could implement custom certs handling, such as in-TEE generated ephemerial certs
#[derive(Clone, Debug)]
pub struct Transport {
    pub config: Configuration,
    pub client: reqwest::Client,
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
        let base_url = &self.config.base_path;
        let url = base_url.join(endpoint).map_err(|e| {
            error!("Invalid endpoint (shouldn't contain base url): {}. Error: {}", endpoint, e);
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid endpoint")) as Box<dyn std::error::Error>
        })?;

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
        let content = resp.text().await.map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Failed to read response text: {}", e))
        })?;

        if status.is_success() {
            Ok(content)
        } else {
            Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Request failed with status: {}. Response: {}", status, content),
            )))
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
        let url = self.config.base_path.join(endpoint)?;
        let mut request = self.client.post(url).multipart(form);
        if let Some(user_agent) = &self.config.user_agent {
            request = request.header(reqwest::header::USER_AGENT, user_agent);
        }
        let response = request.send().await?;
        let status = response.status();
        let body = response.text().await?;
        if status.is_success() {
            Ok(body)
        } else {
            Err(format!("Request failed with status: {}. Response: {}", status, body).into())
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
    use crate::utils::transport::Transport;
    use mockito::mock;
    use url::Url;

    // effectively no sense in testing various responses, as it's reqwest's responsibility
    // we should test Transport's methods instead

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
