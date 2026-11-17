//! Invariants of client state for the supported construction paths: a
//! client's configuration is its transport's, and a discovered client and its
//! capability snapshot come from one service-info response. See
//! `docs/state_ownership.md`.

use ga4gh_sdk::{
    clients::{
        drs::DRS,
        serviceinfo::models::{Service, ServiceType},
        tes::TES,
        wes::WES,
    },
    reqwest,
    utils::{configuration::Configuration, transport::Transport},
};
use mockito::{mock, server_url};
use serde_json::json;
use std::time::Duration;

/// A service-info document of `artifact` with this `id`.
fn info(artifact: &str, id: &str) -> serde_json::Value {
    json!({
        "id": id,
        "name": "Example",
        "type": {"group": "org.ga4gh", "artifact": artifact, "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0"
    })
}

/// A configuration for the API root `/<root>/` on the mock server, with a
/// user agent that only this configuration sends.
fn config_below(root: &str) -> Configuration {
    let mut config =
        Configuration::new(url::Url::parse(&format!("{}/{root}/", server_url())).unwrap());
    config.user_agent = Some(format!("client-state/{root}"));
    config
}

/// A service info that names `artifact`, without a request.
fn known_service(artifact: &str) -> Service {
    Service {
        id: format!("org.example.{artifact}"),
        r#type: Box::new(ServiceType {
            artifact: artifact.into(),
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[test]
fn without_discovery_keeps_the_transport_and_service() {
    // No server listens here: the constructors must not send a request.
    let config =
        Configuration::new(url::Url::parse("https://example.invalid/ga4gh/x/v1/").unwrap());
    let transport =
        Transport::with_timeouts(&config, Duration::from_secs(3), Duration::from_secs(7)).unwrap();

    let tes = TES::without_discovery(transport.clone(), known_service("tes"));
    let wes = WES::without_discovery(transport.clone(), known_service("wes"));
    let drs = DRS::without_discovery(transport, known_service("drs"));

    for (configuration, own, service, artifact) in [
        (
            tes.configuration(),
            tes.transport().configuration(),
            tes.discovered_service_info(),
            "tes",
        ),
        (
            wes.configuration(),
            wes.transport().configuration(),
            wes.discovered_service_info(),
            "wes",
        ),
        (
            drs.configuration(),
            drs.transport().configuration(),
            drs.discovered_service_info(),
            "drs",
        ),
    ] {
        assert!(std::ptr::eq(configuration, own), "{artifact}");
        assert_eq!(configuration.base_path, config.base_path, "{artifact}");
        assert_eq!(configuration.timeouts.request, Duration::from_secs(7));
        assert_eq!(service.id, format!("org.example.{artifact}"));
        assert_eq!(service.r#type.artifact, artifact);
    }
}

#[tokio::test]
async fn every_constructor_reads_the_transports_configuration() {
    for artifact in ["tes", "wes", "drs"] {
        let root = format!("constructors/ga4gh/{artifact}/v1");
        let _info = mock("GET", format!("/{root}/service-info").as_str())
            .match_header("user-agent", format!("client-state/{root}").as_str())
            .with_body(info(artifact, "org.example.constructors").to_string())
            .create();
        let config = config_below(&root);
        let transport = Transport::new(&config);
        let configurations = match artifact {
            "tes" => vec![
                TES::new(&config).await.unwrap(),
                TES::with_transport(transport.clone()).await.unwrap(),
                TES::with_artifact(transport.clone(), "tes").await.unwrap(),
                TES::discover(&config, Duration::from_secs(60))
                    .await
                    .unwrap()
                    .into_parts()
                    .0,
            ]
            .into_iter()
            .map(|c| {
                assert!(std::ptr::eq(
                    c.configuration(),
                    c.transport().configuration()
                ));
                (
                    c.configuration().clone(),
                    c.discovered_service_info().id.clone(),
                )
            })
            .collect::<Vec<_>>(),
            "wes" => vec![
                WES::new(&config).await.unwrap(),
                WES::with_transport(transport.clone()).await.unwrap(),
                WES::with_artifact(transport.clone(), "wes").await.unwrap(),
                WES::discover(&config, Duration::from_secs(60))
                    .await
                    .unwrap()
                    .into_parts()
                    .0,
            ]
            .into_iter()
            .map(|c| {
                assert!(std::ptr::eq(
                    c.configuration(),
                    c.transport().configuration()
                ));
                (
                    c.configuration().clone(),
                    c.discovered_service_info().id.clone(),
                )
            })
            .collect(),
            _ => vec![
                DRS::new(&config).await.unwrap(),
                DRS::with_transport(transport.clone()).await.unwrap(),
                DRS::with_artifact(transport.clone(), "drs").await.unwrap(),
            ]
            .into_iter()
            .map(|c| {
                assert!(std::ptr::eq(
                    c.configuration(),
                    c.transport().configuration()
                ));
                (
                    c.configuration().clone(),
                    c.discovered_service_info().id.clone(),
                )
            })
            .collect(),
        };
        for (configuration, id) in configurations {
            assert_eq!(configuration.base_path, config.base_path, "{artifact}");
            assert_eq!(configuration.user_agent, config.user_agent, "{artifact}");
            assert_eq!(id, "org.example.constructors", "{artifact}");
        }
    }
}

#[tokio::test]
async fn tes_refresh_keeps_the_transport_and_one_response() {
    let root = "refresh-transport/ga4gh/tes/v1";
    let config = config_below(root);
    // Only the injected HTTP client sends this header, so the second
    // service-info request proves that refresh kept the transport.
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-client", "injected".parse().unwrap());
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap();
    let first = mock("GET", format!("/{root}/service-info").as_str())
        .match_header("x-client", "injected")
        .with_body(info("tes", "first").to_string())
        .expect(1)
        .create();
    let mut discovered = TES::discover_with_transport(
        Transport::with_client(&config, client),
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    first.assert();
    drop(first);
    assert_eq!(discovered.client().discovered_service_info().id, "first");
    assert_eq!(discovered.capabilities().raw["id"], "first");

    let second = mock("GET", format!("/{root}/service-info").as_str())
        .match_header("x-client", "injected")
        .with_body(info("tes", "second").to_string())
        .expect(1)
        .create();
    discovered.refresh().await.unwrap();
    second.assert();

    assert_eq!(discovered.client().discovered_service_info().id, "second");
    assert_eq!(discovered.capabilities().raw["id"], "second");
    assert_eq!(discovered.configuration().base_path, config.base_path);
    assert_eq!(discovered.configuration().user_agent, config.user_agent);
    assert!(std::ptr::eq(
        discovered.configuration(),
        discovered.transport().configuration()
    ));
}
