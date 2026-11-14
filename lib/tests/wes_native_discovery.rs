use ga4gh_sdk::{clients::wes::WES, utils::configuration::Configuration};
use serde_json::json;
#[tokio::test]
async fn native_wes_10_service_info_is_usable_without_discovery_fields() {
    let info = json!({"version":"9.3.0","workflow_type_versions":{"CWL":{"workflow_type_version":["v1.2"]}},"supported_wes_versions":["1.0.0"],"supported_filesystem_protocols":["file"],"workflow_engine_versions":{"toil":"9.3.0"},"default_workflow_engine_parameters":[],"system_state_counts":{},"tags":{}});
    let m = mockito::mock("GET", "/native/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let c = Configuration::new(
        ga4gh_sdk::reqwest::Url::parse(&format!("{}/native", mockito::server_url())).unwrap(),
    );
    assert!(WES::new(&c).await.is_ok());
    m.assert();
}

#[tokio::test]
async fn native_wes_10_capability_discovery_keeps_the_original_response() {
    let info = json!({"version":"9.3.0","workflow_type_versions":{"CWL":{"workflow_type_version":["v1.2"]}},"supported_wes_versions":["1.0.0"],"supported_filesystem_protocols":["file"],"workflow_engine_versions":{"toil":"9.3.0"},"default_workflow_engine_parameters":[],"system_state_counts":{},"tags":{}});
    let m = mockito::mock("GET", "/native-discovery/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let c = Configuration::new(
        ga4gh_sdk::reqwest::Url::parse(&format!("{}/native-discovery", mockito::server_url()))
            .unwrap(),
    );
    let discovered = WES::discover(&c, std::time::Duration::from_secs(60))
        .await
        .unwrap();
    let service = discovered.client.service.as_ref().unwrap();
    assert_eq!(service.r#type.artifact, "wes");
    assert_eq!(service.r#type.version, "1.0.0");
    assert_eq!(discovered.capabilities.raw, info);
    m.assert();
}

#[tokio::test]
async fn unrelated_or_malformed_service_info_is_rejected() {
    for info in [
        json!({"version":"9.3.0"}),
        json!({"workflow_type_versions":{},"supported_wes_versions":[]}),
    ] {
        let m = mockito::mock("GET", "/invalid/service-info")
            .with_body(info.to_string())
            .create();
        let c = Configuration::new(
            ga4gh_sdk::reqwest::Url::parse(&format!("{}/invalid", mockito::server_url())).unwrap(),
        );
        assert!(WES::new(&c).await.is_err());
        m.assert();
    }
}
