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
        self.get_at("/service-info").await
    }

    /// Retrieves service information from a service-specific endpoint.
    pub async fn get_at(&self, endpoint: &str) -> Result<models::Service, Box<dyn std::error::Error>> {
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            self.transport.get(endpoint, None)
        ).await?;
        match response {
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
        let service_type= ServiceType::new("group".to_string(), "artifact".to_string(), "version".to_string());
        let service_organization = models::ServiceOrganization::new("org_name".to_string(), "org_url".to_string());

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
            .with_body(r#"{
                "error": "Internal Server Error"
            }"#)
            .create();

        let mock_url = Url::parse(&mockito::server_url()).unwrap();
        let config = Configuration::new(mock_url);
        let service_info = ServiceInfo::new(&config).unwrap();

        // Act
        let result = service_info.get().await;

        // Assert
        assert!(result.is_err());
    }
}
