/// Represents a service information client.
pub mod models;
use crate::utils::configuration::Configuration;
use crate::utils::transport::Transport;

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
    pub fn new(config: &Configuration) -> Result<Self, Box<dyn std::error::Error>> {
        let transport = Transport::new(config);
        let instance = ServiceInfo {
            transport: transport.clone(),
        };
        Ok(instance)
    }

    /// Retrieves the service information.
    ///
    /// # Returns
    ///
    /// A `Result` containing the service information or an error.
    pub async fn get(&self) -> Result<models::Service, Box<dyn std::error::Error>> {
        self.get_at("service-info").await
    }

    /// Retrieves service information from a path relative to the configured API root,
    /// within the `discovery` limit of the configuration's
    /// [`Timeouts`](crate::utils::timeouts::Timeouts).
    pub async fn get_at(
        &self,
        endpoint: &str,
    ) -> Result<models::Service, Box<dyn std::error::Error>> {
        fetch(&self.transport, endpoint).await
    }
}

/// Fetches service info from `endpoint` within the discovery timeout of the
/// transport's configuration, which replaces its request timeout.
pub(crate) async fn fetch_raw(
    transport: &Transport,
    endpoint: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    transport
        .get_within(endpoint, transport.config.timeouts.discovery)
        .await
}

/// Fetches and parses service info as [`fetch_raw`] does.
pub(crate) async fn fetch(
    transport: &Transport,
    endpoint: &str,
) -> Result<models::Service, Box<dyn std::error::Error>> {
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
