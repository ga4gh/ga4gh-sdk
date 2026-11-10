pub mod models;

use crate::clients::drs::models::AccessUrl;
use crate::clients::drs::models::DrsObject;
use crate::clients::serviceinfo::models::Service;
use crate::clients::serviceinfo::ServiceInfo;
use crate::utils::configuration::Configuration;
use crate::utils::transport::{urlencode, HttpStatusError, Transport};
use log::error;
use serde_json::from_str;

/// The main struct for interacting with a DRS service.
#[derive(Debug)]
pub struct DRS {
    #[allow(dead_code)]
    pub config: Configuration,
    pub service: Result<Service, Box<dyn std::error::Error>>,
    pub transport: Transport,
}

impl DRS {
    /// Creates a new `DRS` instance.
    ///
    /// # Arguments
    /// - `config`: A reference to the service configuration.
    ///
    /// # Returns
    /// - A new `DRS` instance, or an error if the initialization fails.
    pub async fn new(config: &Configuration) -> Result<Self, Box<dyn std::error::Error>> {
        let transport = Transport::new(config);
        let service_info = ServiceInfo::new(config)?;
        let resp = service_info.get().await;

        let instance = DRS {
            config: config.clone(),
            transport,
            service: resp,
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
            Err(e) => {
                let mut message = format!("Error accessing DRS service: {e}");
                if e.downcast_ref::<HttpStatusError>()
                    .is_some_and(|error| error.status == reqwest::StatusCode::NOT_FOUND)
                {
                    message.push_str(". Check that base_path is the DRS API root");
                }
                Err(message)
            }
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
        let response = self.get_object_raw(object_id).await;

        match response {
            Ok(resp_str) => {
                let object: DrsObject = from_str(&resp_str)?;
                Ok(object)
            }
            Err(e) => {
                error!("HTTP request failed: {:?}", e);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("HTTP request failed: {:?}", e),
                )))
            }
        }
    }

    /// Retrieves an object's JSON without discarding fields unknown to the SDK model.
    pub async fn get_object_raw(
        &self,
        object_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("objects/{}", urlencode(object_id));
        self.transport.get(&url, None).await
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
        let response = self.get_access_url_raw(object_id, access_id).await;

        match response {
            Ok(resp_str) => {
                let access_url: AccessUrl = from_str(&resp_str)?;
                Ok(access_url)
            }
            Err(e) => {
                error!("HTTP request failed: {:?}", e);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("HTTP request failed: {:?}", e),
                )))
            }
        }
    }

    /// Retrieves access URL JSON without discarding fields unknown to the SDK model.
    pub async fn get_access_url_raw(
        &self,
        object_id: &str,
        access_id: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!(
            "objects/{}/access/{}",
            urlencode(object_id),
            urlencode(access_id)
        );
        self.transport.get(&url, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::serviceinfo::models::ServiceType;
    use mockito::{mock, server_url};

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
            assert!(error.contains("service failure"), "{error}");
            assert_eq!(error.contains("base_path"), status == 404, "{error}");
        }
    }
}
