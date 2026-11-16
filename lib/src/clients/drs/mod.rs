pub mod bulk;
// Generated models (utils/build_models.sh): the generator writes `Default`
// for enums by hand.
#[allow(clippy::derivable_impls)]
pub mod models;
pub mod resolution;
pub mod uri;

use crate::clients::drs::models::AccessUrl;
use crate::clients::drs::models::DrsObject;
use crate::clients::serviceinfo::models::Service;
use crate::utils::configuration::Configuration;
use crate::utils::transport::{urlencode, HttpStatusError, Transport};
use std::error::Error;
use std::fmt;

// Describes a failed service-info request, with a hint if the API root seems wrong.
fn access_error(error: &(dyn std::error::Error + 'static)) -> String {
    let mut message = format!("Error accessing DRS service: {error}");
    if error
        .downcast_ref::<HttpStatusError>()
        .is_some_and(|error| error.status == reqwest::StatusCode::NOT_FOUND)
    {
        message.push_str(". Check that base_path is the DRS API root");
    }
    message
}

/// Keeps the original discovery failure inspectable while adding the API-root hint.
#[derive(Debug)]
struct DiscoveryError(Box<dyn Error>);

impl fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&access_error(self.0.as_ref()))
    }
}

impl Error for DiscoveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.0.as_ref())
    }
}

/// The main struct for interacting with a DRS service.
#[derive(Debug)]
pub struct DRS {
    #[allow(dead_code)]
    pub config: Configuration,
    pub service: Result<Service, Box<dyn std::error::Error + Send + Sync>>,
    pub transport: Transport,
}

impl DRS {
    /// Creates a new `DRS` instance.
    ///
    /// # Arguments
    /// - `config`: A reference to the service configuration; its `timeouts`
    ///   set the time limits (see [`Timeouts`](crate::utils::timeouts::Timeouts)).
    ///
    /// # Returns
    /// - A new `DRS` instance, or an error if the initialization fails.
    pub async fn new(config: &Configuration) -> Result<Self, Box<dyn std::error::Error>> {
        Self::with_transport(Transport::new(config)).await
    }

    /// Creates a new `DRS` instance that sends its requests with `transport`, for
    /// example one from [`Transport::with_client`]. The client's configuration is
    /// the transport's.
    ///
    /// # Returns
    /// - A new `DRS` instance, or an error if the initialization fails.
    pub async fn with_transport(transport: Transport) -> Result<Self, Box<dyn std::error::Error>> {
        let service = crate::clients::serviceinfo::fetch(&transport, "service-info")
            .await
            .map_err(|error| Box::new(DiscoveryError(error)) as Box<dyn Error>)?;

        let instance = DRS {
            config: transport.config.clone(),
            transport,
            service: Ok(service),
        };

        instance.check()?;
        Ok(instance)
    }

    /// Checks if the service is of DRS class.
    ///
    /// # Returns
    /// - Ok(()) if the service is valid.
    /// - Err(String) if the service is invalid or an error occurs.
    fn check(&self) -> Result<(), String> {
        let resp = &self.service;
        match resp.as_ref() {
            Ok(service)
                if service.r#type.group == "org.ga4gh" && service.r#type.artifact == "drs" =>
            {
                Ok(())
            }
            Ok(_) => Err("The endpoint is not an instance of DRS".into()),
            // `DRS::new` stores only `Ok`; an `Err` comes from a client built by hand.
            Err(error) => Err(access_error(error.as_ref())),
        }
    }

    /// Retrieves the details of a specific DRS object.
    ///
    /// # Arguments
    /// - `object_id`: The ID of the object to retrieve.
    ///
    /// # Returns
    /// - On success, returns a `DrsObject` containing the object details.
    /// - On failure, returns an error.
    pub async fn get_object(
        &self,
        object_id: &str,
    ) -> Result<DrsObject, Box<dyn std::error::Error>> {
        match self.get_object_once(object_id).await? {
            resolution::Resolution::Ready(value) => Ok(value),
            resolution::Resolution::Pending(pending) => Err(Box::new(pending)),
        }
    }

    /// Retrieves an object's JSON without discarding fields unknown to the SDK model.
    pub async fn get_object_raw(
        &self,
        object_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let reference = self.resolution_reference(format!("objects/{}", urlencode(object_id)));
        match self.resolve_reference_raw(&reference).await? {
            resolution::Resolution::Ready(body) => Ok(body),
            resolution::Resolution::Pending(pending) => Err(Box::new(pending)),
        }
    }

    /// Retrieves an access URL for a specific DRS object.
    ///
    /// # Arguments
    /// - `object_id`: The ID of the object.
    /// - `access_id`: The access ID.
    ///
    /// # Returns
    /// - On success, returns an `AccessUrl` containing the URL and headers.
    /// - On failure, returns an error.
    pub async fn get_access_url(
        &self,
        object_id: &str,
        access_id: &str,
    ) -> Result<AccessUrl, Box<dyn std::error::Error>> {
        match self.get_access_url_once(object_id, access_id).await? {
            resolution::Resolution::Ready(value) => Ok(value),
            resolution::Resolution::Pending(pending) => Err(Box::new(pending)),
        }
    }

    /// Retrieves access URL JSON without discarding fields unknown to the SDK model.
    pub async fn get_access_url_raw(
        &self,
        object_id: &str,
        access_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let reference = self.resolution_reference(format!(
            "objects/{}/access/{}",
            urlencode(object_id),
            urlencode(access_id)
        ));
        match self.resolve_reference_raw(&reference).await? {
            resolution::Resolution::Ready(body) => Ok(body),
            resolution::Resolution::Pending(pending) => Err(Box::new(pending)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::serviceinfo::models::ServiceType;
    use mockito::{mock, server_url};

    #[test]
    fn test_clients_can_move_between_threads() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<DRS>();
        send_sync::<uri::DrsResolver>();
    }

    #[test]
    fn test_debug_output() {
        let config = crate::utils::configuration::config_with_markers();
        let transport = Transport::new(&config);
        let drs = DRS {
            config,
            service: Ok(Service::default()),
            transport,
        };
        let output = format!("{:?}", drs);
        assert!(!output.contains("SENTINEL"), "{output}");
    }

    #[tokio::test]
    async fn test_new_uses_drs_service_info_path() {
        let _m = mock("GET", "/ga4gh/drs/v1/service-info")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "id": "org.example.drs",
                    "name": "Example DRS",
                    "type": { "group": "org.ga4gh", "artifact": "drs", "version": "1.4.0" },
                    "organization": { "name": "Example", "url": "https://example.org" },
                    "version": "1.0.0"
                })
                .to_string(),
            )
            .create();

        let config =
            Configuration::new(url::Url::parse(&format!("{}/ga4gh/drs/v1", server_url())).unwrap());
        let drs = DRS::new(&config).await.unwrap();
        assert_eq!(drs.service.unwrap().r#type.artifact, "drs");
    }

    #[tokio::test]
    async fn discovery_error_retains_http_status_and_api_root_hint() {
        let _m = mock("GET", "/bad-root/service-info")
            .with_status(404)
            .with_body("missing")
            .create();
        let config =
            Configuration::new(url::Url::parse(&format!("{}/bad-root", server_url())).unwrap());
        let error = DRS::new(&config).await.unwrap_err();
        assert!(error
            .to_string()
            .contains("Check that base_path is the DRS API root"));
        let cause = error
            .source()
            .and_then(|source| source.downcast_ref::<HttpStatusError>())
            .expect("HTTP error remains available as the source");
        assert_eq!(cause.status, reqwest::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_get_object() {
        let object_id = "123";
        let response_body = r#"{
            "id": "123",
            "self_uri": "drs://drs.example.org/123",
            "size": 1024,
            "created_time": "2023-01-01T00:00:00Z",
            "checksums": []
        }"#;

        let _m = mock("GET", "/ga4gh/drs/v1/objects/123")
            .with_status(200)
            .with_body(response_body)
            .create();

        let mock_url =
            url::Url::parse(&format!("{}/ga4gh/drs/v1", server_url())).expect("Invalid URL");
        let config = Configuration::new(mock_url);
        let transport = Transport::new(&config);

        let drs = DRS {
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "drs".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
            transport,
        };

        let result = drs.get_object(object_id).await;
        assert!(result.is_ok());
        let object = result.unwrap();
        assert_eq!(object.id, "123");
        assert_eq!(object.size, 1024);
    }

    #[tokio::test]
    async fn test_get_access_url() {
        let object_id = "123";
        let access_id = "access123";
        let response_body = r#"{
            "url": "http://example.com/data",
            "headers": ["Authorization: Basic Z2E0Z2g6ZHJz"]
        }"#;

        let _m = mock("GET", "/ga4gh/drs/v1/objects/123/access/access123")
            .with_status(200)
            .with_body(response_body)
            .create();

        let mock_url =
            url::Url::parse(&format!("{}/ga4gh/drs/v1", server_url())).expect("Invalid URL");
        let config = Configuration::new(mock_url);
        let transport = Transport::new(&config);

        let drs = DRS {
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "drs".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
            transport,
        };

        let result = drs.get_access_url(object_id, access_id).await;
        assert!(result.is_ok());
        let access_url = result.unwrap();
        assert_eq!(access_url.url, "http://example.com/data");
        assert_eq!(
            access_url.headers.unwrap()[0],
            "Authorization: Basic Z2E0Z2g6ZHJz"
        );
    }

    #[tokio::test]
    async fn test_ids_are_encoded_as_path_segments() {
        let object_id = "a/b ?#";
        let access_id = "x/y ?#";
        let _object = mock("GET", "/ga4gh/drs/v1/objects/a%2Fb%20%3F%23")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "id": object_id,
                    "self_uri": "drs://example.org/a%2Fb%20%3F%23",
                    "size": 1,
                    "created_time": "2023-01-01T00:00:00Z",
                    "checksums": []
                })
                .to_string(),
            )
            .create();
        let _access = mock(
            "GET",
            "/ga4gh/drs/v1/objects/a%2Fb%20%3F%23/access/x%2Fy%20%3F%23",
        )
        .with_status(200)
        .with_body(r#"{"url":"https://example.org/data"}"#)
        .create();

        let config =
            Configuration::new(url::Url::parse(&format!("{}/ga4gh/drs/v1", server_url())).unwrap());
        let drs = DRS {
            transport: Transport::new(&config),
            config,
            service: Ok(Service::default()),
        };
        assert_eq!(drs.get_object(object_id).await.unwrap().id, object_id);
        assert_eq!(
            drs.get_access_url(object_id, access_id).await.unwrap().url,
            "https://example.org/data"
        );
    }

    #[tokio::test]
    async fn test_drs_methods_under_api_root_with_both_slash_forms() {
        let object_id = "a/b ?#";
        let access_id = "x/y ?#";
        for suffix in ["", "/"] {
            let prefix = "/proxy/ga4gh/drs/v1";
            let _service = mock("GET", format!("{prefix}/service-info").as_str())
                .with_status(200)
                .with_body(
                    serde_json::json!({
                        "id": "org.example.drs",
                        "name": "Example DRS",
                        "type": { "group": "org.ga4gh", "artifact": "drs", "version": "1.4.0" },
                        "organization": { "name": "Example", "url": "https://example.org" },
                        "version": "1.0.0"
                    })
                    .to_string(),
                )
                .create();
            let _object = mock(
                "GET",
                format!("{prefix}/objects/{}", urlencode(object_id)).as_str(),
            )
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "id": object_id,
                    "self_uri": "drs://example.org/object",
                    "size": 1,
                    "created_time": "2023-01-01T00:00:00Z",
                    "checksums": []
                })
                .to_string(),
            )
            .create();
            let _access = mock(
                "GET",
                format!(
                    "{prefix}/objects/{}/access/{}",
                    urlencode(object_id),
                    urlencode(access_id)
                )
                .as_str(),
            )
            .with_status(200)
            .with_body(r#"{"url":"https://example.org/data"}"#)
            .create();

            let base = url::Url::parse(&format!("{}{prefix}{suffix}", server_url())).unwrap();
            let drs = DRS::new(&Configuration::new(base)).await.unwrap();
            assert_eq!(drs.get_object(object_id).await.unwrap().id, object_id);
            assert_eq!(
                drs.get_access_url(object_id, access_id).await.unwrap().url,
                "https://example.org/data"
            );
        }
    }

    #[tokio::test]
    async fn test_service_info_error_keeps_status_and_404_hint() {
        for (status, path) in [(404, "/missing/drs/v1"), (503, "/unavailable/drs/v1")] {
            let _service = mock("GET", format!("{path}/service-info").as_str())
                .with_status(status)
                .with_body("service failure")
                .create();
            let base = url::Url::parse(&format!("{}{path}", server_url())).unwrap();
            let error = DRS::new(&Configuration::new(base))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(&status.to_string()), "{error}");
            assert!(!error.contains("service failure"), "{error}");
            assert_eq!(error.contains("base_path"), status == 404, "{error}");
        }
    }
}
