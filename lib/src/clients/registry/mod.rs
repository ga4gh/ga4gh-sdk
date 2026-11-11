//! Client for the [GA4GH Service Registry API](https://github.com/ga4gh-discovery/ga4gh-service-registry) 1.0.
//!
//! A registry lists GA4GH services and their base URLs, so that clients can
//! discover TES, WES or other endpoints instead of configuring each one.
//! The models hold only the fields of the specification: fields that an
//! implementation adds are ignored when responses are parsed.
//!
//! ```no_run
//! use ga4gh_sdk::clients::registry::ServiceRegistry;
//! use ga4gh_sdk::utils::configuration::Configuration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let base_path = url::Url::parse("https://registry.example.org/ga4gh/registry/v1")?;
//! let registry = ServiceRegistry::new(&Configuration::new(base_path));
//! for service in registry.list_services().await? {
//!     if service.r#type.artifact == "tes" {
//!         println!("{} {}", service.id, service.url);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod models;

use crate::utils::configuration::Configuration;
use crate::utils::transport::{urlencode, Transport};
use serde::de::DeserializeOwned;
use std::error::Error;

/// A client for a GA4GH Service Registry.
#[derive(Clone, Debug)]
pub struct ServiceRegistry {
    transport: Transport,
}

impl ServiceRegistry {
    /// Creates a client for the registry whose API root is `config.base_path`,
    /// such as `https://host/ga4gh/registry/v1`.
    ///
    /// No request is sent until a method is called.
    pub fn new(config: &Configuration) -> Self {
        Self {
            transport: Transport::new(config),
        }
    }

    /// Lists the services in the registry (`GET /services`).
    pub async fn list_services(&self) -> Result<Vec<models::ExternalService>, Box<dyn Error>> {
        self.get_json("services").await
    }

    /// Gets one service by its ID (`GET /services/{serviceId}`).
    ///
    /// The ID is percent-encoded as a single path segment. An empty ID is an
    /// error, sent to no server.
    pub async fn get_service(
        &self,
        service_id: &str,
    ) -> Result<models::ExternalService, Box<dyn Error>> {
        if service_id.is_empty() {
            return Err("service ID must not be empty".into());
        }
        self.get_json(&format!("services/{}", urlencode(service_id)))
            .await
    }

    /// Lists the distinct types of the services in the registry (`GET /services/types`).
    pub async fn list_service_types(&self) -> Result<Vec<models::ServiceType>, Box<dyn Error>> {
        self.get_json("services/types").await
    }

    /// Gets information about the registry itself (`GET /service-info`).
    pub async fn service_info(&self) -> Result<models::Service, Box<dyn Error>> {
        self.get_json("service-info").await
    }

    async fn get_json<T: DeserializeOwned>(&self, endpoint: &str) -> Result<T, Box<dyn Error>> {
        let body = self.transport.get(endpoint, None).await?;
        serde_json::from_str(&body).map_err(|error| {
            format!("Invalid response from the service registry ({endpoint}): {error}").into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::transport::HttpError;
    use mockito::mock;
    use serde_json::{json, Value};
    use url::Url;

    fn service(id: &str, artifact: &str) -> Value {
        json!({
            "id": id,
            "name": format!("Service {id}"),
            "type": {"group": "org.ga4gh", "artifact": artifact, "version": "1.1.0"},
            "organization": {"name": "Example", "url": "https://example.org"},
            "version": "1.0.0",
            "url": format!("https://example.org/ga4gh/{artifact}/v1"),
        })
    }

    fn registry(prefix: &str) -> ServiceRegistry {
        let base = Url::parse(&format!("{}{prefix}", mockito::server_url())).unwrap();
        ServiceRegistry::new(&Configuration::new(base))
    }

    #[tokio::test]
    async fn list_services_under_api_root_with_or_without_trailing_slash() {
        let _m = mock("GET", "/list/ga4gh/registry/v1/services")
            .with_status(200)
            .with_body(json!([service("org.example.tes", "tes")]).to_string())
            .expect(2)
            .create();
        for suffix in ["", "/"] {
            let services = registry(&format!("/list/ga4gh/registry/v1{suffix}"))
                .list_services()
                .await
                .unwrap();
            assert_eq!(services.len(), 1);
            assert_eq!(services[0].id, "org.example.tes");
            assert_eq!(services[0].url, "https://example.org/ga4gh/tes/v1");
            assert_eq!(services[0].r#type.artifact, "tes");
        }
        _m.assert();
    }

    #[tokio::test]
    async fn extra_fields_are_kept() {
        let mut body = service("org.example.wes", "wes");
        body["sync"] = json!({"enabled": true});
        body["organization"]["logo"] = json!("https://example.org/logo.png");
        let _m = mock("GET", "/extra/services")
            .with_status(200)
            .with_body(json!([body]).to_string())
            .create();
        let services = registry("/extra").list_services().await.unwrap();
        assert_eq!(services[0].id, "org.example.wes");
        assert_eq!(services[0].extra["sync"], json!({"enabled": true}));
        assert_eq!(serde_json::to_value(&services[0]).unwrap(), body);
    }

    #[tokio::test]
    async fn get_service_encodes_the_id_as_one_segment() {
        let _m = mock("GET", "/get/services/a%2Fb%20c%3F")
            .with_status(200)
            .with_body(service("a/b c?", "drs").to_string())
            .create();
        let service = registry("/get").get_service("a/b c?").await.unwrap();
        assert_eq!(service.id, "a/b c?");
        _m.assert();
    }

    #[tokio::test]
    async fn get_service_rejects_dot_segment_ids() {
        for id in [".", ".."] {
            let error = registry("/dots").get_service(id).await.unwrap_err();
            assert!(error.to_string().contains("dot segments"), "{error}");
        }
        let error = registry("/dots").get_service("").await.unwrap_err();
        assert_eq!(error.to_string(), "service ID must not be empty");
    }

    #[tokio::test]
    async fn get_service_reports_not_found() {
        let _m = mock("GET", "/missing/services/unknown")
            .with_status(404)
            .with_body(r#"{"status":404,"title":"Not found"}"#)
            .create();
        let error = registry("/missing")
            .get_service("unknown")
            .await
            .unwrap_err();
        let http = error.downcast_ref::<HttpError>().unwrap();
        assert_eq!(http.status, 404);
        assert!(http.body.contains("Not found"));
    }

    #[tokio::test]
    async fn list_service_types() {
        let _m = mock("GET", "/types/v1/services/types")
            .with_status(200)
            .with_body(
                json!([
                    {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
                    {"group": "org.ga4gh", "artifact": "wes", "version": "1.1.0", "extra": 1},
                ])
                .to_string(),
            )
            .create();
        let types = registry("/types/v1/").list_service_types().await.unwrap();
        let mut wes = models::ServiceType::new("org.ga4gh".into(), "wes".into(), "1.1.0".into());
        // A field that the spec doesn't define is kept, even if it is called `extra`.
        wes.extra.insert("extra".into(), json!(1));
        assert_eq!(
            types,
            vec![
                models::ServiceType::new("org.ga4gh".into(), "tes".into(), "1.1.0".into()),
                wes,
            ]
        );
    }

    #[tokio::test]
    async fn service_info_of_the_registry() {
        let mut body = service("org.example.registry", "service-registry");
        body.as_object_mut().unwrap().remove("url");
        let _m = mock("GET", "/info/service-info")
            .with_status(200)
            .with_body(body.to_string())
            .create();
        let info = registry("/info").service_info().await.unwrap();
        assert_eq!(info.r#type.artifact, "service-registry");
    }

    #[tokio::test]
    async fn server_errors_keep_status_and_body() {
        let _m = mock("GET", "/fail/services")
            .with_status(500)
            .with_body("registry down")
            .create();
        let error = registry("/fail").list_services().await.unwrap_err();
        let http = error.downcast_ref::<HttpError>().unwrap();
        assert_eq!(http.status, 500);
        assert_eq!(http.body, "registry down");
    }

    #[tokio::test]
    async fn invalid_bodies_are_errors() {
        let mut without_url = service("org.example.tes", "tes");
        without_url.as_object_mut().unwrap().remove("url");
        for (prefix, body) in [
            ("/invalid-json", "not JSON".to_string()),
            ("/invalid-shape", json!({"services": []}).to_string()),
            ("/invalid-url", json!([without_url]).to_string()),
        ] {
            let _m = mock("GET", format!("{prefix}/services").as_str())
                .with_status(200)
                .with_body(body)
                .create();
            let error = registry(prefix).list_services().await.unwrap_err();
            assert!(
                error
                    .to_string()
                    .starts_with("Invalid response from the service registry (services): "),
                "{prefix}: {error}"
            );
        }
    }
}
