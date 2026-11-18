//! Client for the GA4GH Service Info API, which every GA4GH service offers at
//! `service-info` below its API root.

/// The Service Info 1.0 models, generated from its OpenAPI specification.
#[allow(missing_docs)]
pub mod models;
use crate::utils::configuration::Configuration;
use crate::utils::transport::Transport;

/// A client that reads a service's service info.
#[derive(Clone)]
pub struct ServiceInfo {
    transport: Transport,
}

impl ServiceInfo {
    /// Creates a new instance of `ServiceInfo` with the given configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration for the service info client.
    ///
    /// # Returns
    ///
    /// A `Result` containing the `ServiceInfo` instance or an error.
    pub fn new(config: &Configuration) -> crate::Result<Self> {
        let transport = Transport::new(config);
        let instance = ServiceInfo {
            transport: transport.clone(),
        };
        Ok(instance)
    }

    /// Retrieves the service information.
    ///
    /// Any service type is returned as declared, including custom ones and
    /// unknown API versions; fields that the schema doesn't define stay in
    /// `extra`. See the [Service Info contract] for the full behavior.
    ///
    /// # Returns
    ///
    /// A `Result` containing the service information or an error.
    ///
    /// # Errors
    ///
    /// [`Error::Http`](crate::Error::Http) for a non-2xx status,
    /// [`Error::Parse`](crate::Error::Parse) for malformed JSON or a document
    /// that isn't a valid [`models::Service`], for example one without a
    /// required field, and an error with a [`std::str::Utf8Error`] for a body
    /// that isn't valid UTF-8, among the other errors of a request, such as a
    /// [`BodyLimitError`](crate::utils::transport::BodyLimitError) for a body
    /// over the limit or a timeout. A failed decoding isn't retried.
    ///
    /// [Service Info contract]: https://github.com/ga4gh/ga4gh-sdk/blob/main/docs/api_compatibility.md#service-info
    pub async fn get(&self) -> crate::Result<models::Service> {
        self.get_at("service-info").await
    }

    /// Retrieves service information from a path relative to the configured API root,
    /// within the `discovery` limit of the configuration's
    /// [`Timeouts`](crate::utils::timeouts::Timeouts). It decodes and fails as
    /// [`Self::get`] does.
    pub async fn get_at(&self, endpoint: &str) -> crate::Result<models::Service> {
        fetch(&self.transport, endpoint).await
    }

    /// Retrieves the response body of `service-info` as received, within the
    /// same `discovery` limit as [`get`](Self::get), without parsing it.
    ///
    /// Callers that print or store the service info use it to keep the
    /// server's JSON text, such as the spelling of large numbers.
    pub async fn get_raw(&self) -> crate::Result<String> {
        fetch_raw(&self.transport, "service-info").await
    }
}

/// Fetches service info from `endpoint` within the discovery timeout of the
/// transport's configuration, which replaces its request timeout.
pub(crate) async fn fetch_raw(transport: &Transport, endpoint: &str) -> crate::Result<String> {
    transport
        .get_within(endpoint, transport.configuration().timeouts.discovery)
        .await
}

/// Fetches and parses service info as [`fetch_raw`] does.
pub(crate) async fn fetch(transport: &Transport, endpoint: &str) -> crate::Result<models::Service> {
    match fetch_raw(transport, endpoint).await {
        Ok(response_body) => match serde_json::from_str::<models::Service>(&response_body) {
            Ok(service) => Ok(service),
            Err(e) => {
                log::error!("Failed to deserialize response. Error: {}", e);
                Err(e.into())
            }
        },
        Err(e) => {
            log::error!("Error getting response: {}", e);
            Err(e)
        }
    }
}

/// Checks that `service` names `artifact`; `api` names the API in the error,
/// for example `TES`.
///
/// # Errors
/// [`Error::UnexpectedService`](crate::Error::UnexpectedService) if the service
/// info names another artifact.
pub(crate) fn check_artifact(
    service: &models::Service,
    artifact: &str,
    api: &str,
) -> crate::Result<()> {
    if service.r#type.artifact == artifact {
        return Ok(());
    }
    let expected = if artifact == api.to_ascii_lowercase() {
        api.to_owned()
    } else {
        format!("'{artifact}'")
    };
    Err(crate::Error::UnexpectedService(format!(
        "The endpoint is not an instance of {expected}"
    )))
}

/// A service-info request of a client constructor that the service answered
/// with an error status, with a hint if the API root seems wrong. The request's
/// error stays available as the source and through
/// [`Error::downcast_ref`](crate::Error::downcast_ref).
#[derive(Debug)]
pub(crate) struct DiscoveryError {
    /// The name of the API, for example `DRS`.
    pub(crate) api: &'static str,
    /// The error of the request.
    pub(crate) source: crate::Error,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error accessing {} service: {}", self.api, self.source)?;
        if self
            .source
            .downcast_ref::<crate::utils::transport::HttpError>()
            .is_some_and(|error| error.status == reqwest::StatusCode::NOT_FOUND)
        {
            write!(f, ". Check that base_path is the {} API root", self.api)?;
        }
        Ok(())
    }
}

impl std::error::Error for DiscoveryError {
    // The request error itself, as before the SDK error type; `Error` would
    // only add a level that displays the same.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.inner()
    }
}

impl From<DiscoveryError> for crate::Error {
    /// An HTTP error gets the hint, in [`Error::Other`](crate::Error::Other);
    /// other errors, such as a failed connection, keep their own variant.
    fn from(error: DiscoveryError) -> Self {
        match error.source {
            crate::Error::Http(_) => Self::Other(Box::new(error)),
            source => source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::mock;
    use models::ServiceType;
    use url::Url;

    #[tokio::test]
    async fn test_get_service_info_success() {
        // Arrange
        let service_type = ServiceType::new(
            "group".to_string(),
            "artifact".to_string(),
            "version".to_string(),
        );
        let service_organization =
            models::ServiceOrganization::new("org_name".to_string(), "org_url".to_string());

        let _m = mock("GET", "/service-info")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(
                serde_json::json!({
                    "id": "123",
                    "name": "My Service",
                    "type": service_type,
                    "organization": service_organization,
                    "version": "1.0.0"
                })
                .to_string(),
            )
            .create();

        let mock_url = Url::parse(&mockito::server_url()).unwrap();
        let config = Configuration::new(mock_url);
        let service_info = ServiceInfo::new(&config).unwrap();

        // Act
        let result = service_info.get().await;

        // Assert
        assert!(result.is_ok());
        let service = result.unwrap();
        assert_eq!(service.name, "My Service");
        assert_eq!(service.version, "1.0.0");
    }

    #[tokio::test]
    async fn test_get_service_info_failure() {
        // Arrange
        let _m = mock("GET", "/service-info")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(
                r#"{
                "error": "Internal Server Error"
            }"#,
            )
            .create();

        let mock_url = Url::parse(&mockito::server_url()).unwrap();
        let config = Configuration::new(mock_url);
        let service_info = ServiceInfo::new(&config).unwrap();

        // Act
        let result = service_info.get().await;

        // Assert
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_raw_returns_the_body_as_received() {
        let body = r#"{"id":"i","big":123456789012345678901234567890,"f":1.00}"#;
        let _m = mock("GET", "/raw/v1/service-info")
            .with_status(200)
            .with_body(body)
            .create();
        let base = Url::parse(&format!("{}/raw/v1/", mockito::server_url())).unwrap();
        let client = ServiceInfo::new(&Configuration::new(base)).unwrap();
        assert_eq!(client.get_raw().await.unwrap(), body);

        let _error = mock("GET", "/raw-error/service-info")
            .with_status(503)
            .create();
        let base = Url::parse(&format!("{}/raw-error", mockito::server_url())).unwrap();
        let client = ServiceInfo::new(&Configuration::new(base)).unwrap();
        let error = client.get_raw().await.unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<crate::utils::transport::HttpError>()
                .map(|http| http.status.as_u16()),
            Some(503)
        );
    }

    #[tokio::test]
    async fn test_get_service_info_under_api_root() {
        for suffix in ["", "/"] {
            let _m = mock("GET", "/proxy/tes/v1/service-info")
                .with_status(200)
                .with_body(
                    serde_json::json!({
                        "id": "org.example.tes",
                        "name": "Example TES",
                        "type": { "group": "org.ga4gh", "artifact": "tes", "version": "1.1.0" },
                        "organization": { "name": "Example", "url": "https://example.org" },
                        "version": "1.0.0"
                    })
                    .to_string(),
                )
                .create();
            let base =
                Url::parse(&format!("{}/proxy/tes/v1{suffix}", mockito::server_url())).unwrap();
            let client = ServiceInfo::new(&Configuration::new(base)).unwrap();
            assert_eq!(client.get().await.unwrap().name, "Example TES");
        }
    }
}
