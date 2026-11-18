//! Client for the GA4GH Data Repository Service (DRS) 1.4 API.
//!
//! [`DRS`] reads object metadata and access URLs, one at a time or in bulk
//! ([`bulk`]), and waits while a server prepares an answer ([`resolution`]).
//! [`uri`] parses `drs://` URIs and plans access to their objects. The client
//! never downloads object bytes.

pub mod bulk;
/// The DRS 1.4 models, generated from its OpenAPI specification.
// Generated models (utils/build_models.sh): the generator writes `Default`
// for enums by hand.
#[allow(clippy::derivable_impls, missing_docs)]
pub mod models;
pub mod resolution;
pub mod uri;

use crate::clients::drs::models::AccessUrl;
use crate::clients::drs::models::DrsObject;
use crate::clients::serviceinfo::models::Service;
use crate::clients::serviceinfo::DiscoveryError;
use crate::utils::configuration::Configuration;
use crate::utils::transport::{urlencode, Transport};
use serde::de::DeserializeOwned;

/// The main struct for interacting with a DRS service.
///
/// A client is immutable after construction: build a new one, for example
/// with [`with_transport`](Self::with_transport), to change its API root,
/// credentials, HTTP client, or time limits. Read its state with
/// [`configuration`](Self::configuration), [`transport`](Self::transport),
/// and [`discovered_service_info`](Self::discovered_service_info); its
/// fields are private (see the
/// [client state ownership record](https://github.com/ga4gh/ga4gh-sdk/blob/main/docs/state_ownership.md)).
///
/// Assigning a field fails to compile:
///
/// ```compile_fail,E0616
/// use ga4gh_sdk::{clients::drs::DRS, utils::transport::Transport};
/// fn retarget(drs: &mut DRS, transport: Transport) {
///     drs.transport = transport;
/// }
/// ```
///
/// ```compile_fail,E0616
/// use ga4gh_sdk::clients::{drs::DRS, serviceinfo::models::Service};
/// fn swap_service(drs: &mut DRS, service: Service) {
///     drs.service = service;
/// }
/// ```
#[derive(Debug)]
pub struct DRS {
    /// The service info that the client read when it was built; returned by
    /// [`Self::discovered_service_info`].
    service: Service,
    /// The transport that sends the client's requests; returned by
    /// [`Self::transport`].
    transport: Transport,
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
    pub async fn new(config: &Configuration) -> crate::Result<Self> {
        Self::with_transport(Transport::new(config)).await
    }

    /// Creates a new `DRS` instance that sends its requests with `transport`, for
    /// example one from [`Transport::with_client`]. The client's configuration is
    /// the transport's.
    ///
    /// # Returns
    /// - A new `DRS` instance, or an error if the initialization fails.
    ///
    /// # Errors
    /// [`Error::UnexpectedService`](crate::Error::UnexpectedService) if the
    /// service info names another type of service. If the service answers the
    /// service-info request with an error status, an
    /// [`Error::Other`](crate::Error::Other), with a hint on the API root for a
    /// 404, whose [`downcast_ref`](crate::Error::downcast_ref) returns the request's
    /// [`HttpError`](crate::utils::transport::HttpError). Other failures of the
    /// request keep their variant, such as
    /// [`Error::Transport`](crate::Error::Transport).
    pub async fn with_transport(transport: Transport) -> crate::Result<Self> {
        let instance = Self::with_artifact(transport, "drs").await?;
        if instance.discovered_service_info().r#type.group != "org.ga4gh" {
            return Err(crate::Error::UnexpectedService(
                "The endpoint is not an instance of DRS".to_owned(),
            ));
        }
        Ok(instance)
    }

    /// Creates a new `DRS` instance, as [`Self::with_transport`] does, for a
    /// service whose service info names `artifact` in place of `drs`, for
    /// example a downstream service with a DRS API. Unlike
    /// [`Self::with_transport`], it accepts any `group`.
    ///
    /// # Errors
    /// [`Error::UnexpectedService`](crate::Error::UnexpectedService) if the
    /// service info names another artifact; otherwise the errors of the
    /// service-info request, as for [`Self::with_transport`].
    pub async fn with_artifact(transport: Transport, artifact: &str) -> crate::Result<Self> {
        let service = crate::clients::serviceinfo::fetch(&transport, "service-info")
            .await
            .map_err(|source| DiscoveryError { api: "DRS", source })?;
        crate::clients::serviceinfo::check_artifact(&service, artifact, "DRS")?;
        Ok(Self::without_discovery(transport, service))
    }

    /// Creates a client from a transport and service info that the caller
    /// already has, without a service-info request, for example in tests or
    /// for a known deployment without usable discovery. It does not check the
    /// service type; [`with_artifact`](Self::with_artifact) does.
    ///
    /// This replaces building a `DRS` from its fields.
    pub fn without_discovery(transport: Transport, service: Service) -> Self {
        DRS { transport, service }
    }

    /// The configuration of the client's transport: the value that requests
    /// and resolution references read. See
    /// [`Transport::configuration`] for what it determines, and for the
    /// settings of an injected HTTP client that apply instead.
    pub fn configuration(&self) -> &Configuration {
        self.transport().configuration()
    }

    /// The transport that sends the client's requests, for example to share
    /// it with a client for an added endpoint.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// The service info that the client read when it was built. It is a
    /// snapshot: the client does not fetch it again, and the SDK never
    /// replaces it.
    pub fn discovered_service_info(&self) -> &Service {
        &self.service
    }

    /// Retrieves the details of a specific DRS object.
    ///
    /// # Arguments
    /// - `object_id`: The ID of the object to retrieve.
    ///
    /// # Returns
    /// - On success, returns a `DrsObject` containing the object details.
    /// - On failure, returns an error.
    pub async fn get_object(&self, object_id: &str) -> crate::Result<DrsObject> {
        self.get_object_as(object_id).await
    }

    /// Retrieves an object as any type that deserializes from the server's
    /// object JSON, for example a downstream type that reads fields of its own.
    /// A pending answer (`202 Accepted`) is a
    /// [`PendingResolution`](resolution::PendingResolution) error, as for
    /// [`Self::get_object`].
    pub async fn get_object_as<T: DeserializeOwned>(&self, object_id: &str) -> crate::Result<T> {
        let reference = self.resolution_reference(format!("objects/{}", urlencode(object_id)));
        match self.resolve_reference(&reference).await? {
            resolution::Resolution::Ready(value) => Ok(value),
            resolution::Resolution::Pending(pending) => Err(crate::Error::other(pending)),
        }
    }

    /// Retrieves an object's JSON without discarding fields unknown to the SDK model.
    pub async fn get_object_raw(&self, object_id: &str) -> crate::Result<String> {
        let reference = self.resolution_reference(format!("objects/{}", urlencode(object_id)));
        match self.resolve_reference_raw(&reference).await? {
            resolution::Resolution::Ready(body) => Ok(body),
            resolution::Resolution::Pending(pending) => Err(crate::Error::other(pending)),
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
    ) -> crate::Result<AccessUrl> {
        match self.get_access_url_once(object_id, access_id).await? {
            resolution::Resolution::Ready(value) => Ok(value),
            resolution::Resolution::Pending(pending) => Err(crate::Error::other(pending)),
        }
    }

    /// Retrieves access URL JSON without discarding fields unknown to the SDK model.
    pub async fn get_access_url_raw(
        &self,
        object_id: &str,
        access_id: &str,
    ) -> crate::Result<String> {
        let reference = self.resolution_reference(format!(
            "objects/{}/access/{}",
            urlencode(object_id),
            urlencode(access_id)
        ));
        match self.resolve_reference_raw(&reference).await? {
            resolution::Resolution::Ready(body) => Ok(body),
            resolution::Resolution::Pending(pending) => Err(crate::Error::other(pending)),
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
        let drs = DRS::without_discovery(transport, Service::default());
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
        assert_eq!(drs.discovered_service_info().r#type.artifact, "drs");
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
        let cause = std::error::Error::source(&error)
            .and_then(|source| source.downcast_ref::<crate::utils::transport::HttpError>())
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

        let drs = DRS::without_discovery(
            transport,
            Service {
                r#type: Box::new(ServiceType {
                    artifact: "drs".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            },
        );

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

        let drs = DRS::without_discovery(
            transport,
            Service {
                r#type: Box::new(ServiceType {
                    artifact: "drs".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            },
        );

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
        let drs = DRS::without_discovery(Transport::new(&config), Service::default());
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

    /// The accessors return the transport's own configuration and the
    /// discovered service, below a non-default API root and with timeouts
    /// from `Transport::with_timeouts`.
    #[tokio::test]
    async fn configuration_is_transport_configuration() {
        let _m = mock("GET", "/accessors/custom/drs/v9/service-info")
            .with_status(200)
            .with_body(
                serde_json::json!({
                    "id": "org.example.drs",
                    "name": "Example",
                    "type": { "group": "org.ga4gh", "artifact": "drs", "version": "1.0.0" },
                    "organization": { "name": "Example", "url": "https://example.org" },
                    "version": "1.0.0"
                })
                .to_string(),
            )
            .create();
        let root = format!("{}/accessors/custom/drs/v9/", server_url());
        let config = Configuration::new(url::Url::parse(&root).unwrap());
        let connect = std::time::Duration::from_secs(3);
        let request = std::time::Duration::from_secs(7);
        let transport = Transport::with_timeouts(&config, connect, request).unwrap();

        let client = DRS::with_transport(transport).await.unwrap();

        assert_eq!(client.configuration().base_path.as_str(), root);
        assert_eq!(client.configuration().timeouts.connect, connect);
        assert_eq!(client.configuration().timeouts.request, request);
        assert!(std::ptr::eq(
            client.configuration(),
            client.transport().configuration()
        ));
        assert!(std::ptr::eq(client.transport(), &client.transport));
        assert_eq!(client.discovered_service_info().id, "org.example.drs");
        assert!(std::ptr::eq(
            client.discovered_service_info(),
            &client.service
        ));
    }
}
