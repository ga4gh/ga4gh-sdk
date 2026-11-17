use ga4gh_sdk::{
    clients::{
        capabilities::{CapabilitySnapshot, Support},
        tes::{models::TesTask, TES},
        wes::{models::RunRequest, WES},
    },
    utils::{configuration::Configuration, transport::Transport},
};
use mockito::{mock, server_url};
use serde_json::json;
use std::time::Duration;
fn service(artifact: &str) -> serde_json::Value {
    json!({"id":"test","name":"test","type":{"group":"org.ga4gh","artifact":artifact,"version":"1.1"},"organization":{"name":"test","url":"https://example.org"},"version":"1"})
}
#[tokio::test]
async fn discovery_caches_raw_extensions_and_refreshes_once() {
    let mut info = service("tes");
    info["extension"] = json!({"custom":true});
    let first = mock("GET", "/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
    let mut client = TES::discover(&config, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(client.capabilities.raw["extension"]["custom"], true);
    assert_eq!(client.capabilities.tes_info().unwrap().name, "test");
    assert_eq!(client.capabilities.tes_info().unwrap().name, "test");
    first.assert();
    drop(first);
    info["extension"]["custom"] = json!(false);
    let refresh = mock("GET", "/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    client.refresh().await.unwrap();
    assert_eq!(client.capabilities.raw["extension"]["custom"], false);
    refresh.assert();
}
#[tokio::test]
async fn strict_wes_contradiction_prevents_submission_but_unknown_is_advisory() {
    let mut info = service("wes");
    info["workflow_type_versions"] = json!({"CWL":{"workflow_type_version":["v1.2"]}});
    let service = mock("GET", "/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submit = mock("POST", "/runs")
        .with_body(r#"{"run_id":"r1"}"#)
        .expect(0)
        .create();
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
    let client = WES::discover(&config, Duration::from_secs(60))
        .await
        .unwrap();
    let request = RunRequest {
        workflow_type: "WDL".into(),
        workflow_type_version: "1.0".into(),
        workflow_url: "workflow.wdl".into(),
        workflow_params: Some(json!({})),
        ..Default::default()
    };
    assert_eq!(
        client.capabilities.validate_wes(&request)[0].support,
        Support::Unsupported
    );
    assert!(client
        .run_validated(request.clone(), true)
        .await
        .unwrap_err()
        .to_string()
        .contains("workflow_type"));
    service.assert();
    submit.assert();
    let unknown = CapabilitySnapshot::from_raw(json!({}), Duration::from_secs(1));
    assert_eq!(unknown.validate_wes(&request)[0].support, Support::Unknown);
}
#[tokio::test(start_paused = true)]
async fn stale_metadata_and_tes_parameter_case_strictness() {
    let snapshot = CapabilitySnapshot::from_raw(
        json!({"tesResources_backend_parameters":["VmSize"],"storage":["s3://known"]}),
        Duration::from_secs(1),
    );
    let task:TesTask=serde_json::from_value(json!({"executors":[],"resources":{"backend_parameters":{"vmsize":"large","unknown":"value"},"backend_parameters_strict":true},"inputs":[{"url":"https://unlisted.example/data","path":"/data"}]})).unwrap();
    let diagnostics = snapshot.validate_tes(&task);
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(
        diagnostics
            .iter()
            .find(|d| d.field.ends_with("vmsize"))
            .unwrap()
            .support,
        Support::Supported
    );
    assert!(
        diagnostics
            .iter()
            .find(|d| d.field.ends_with("unknown"))
            .unwrap()
            .blocks_strict
    );
    let mut advisory = task.clone();
    advisory
        .resources
        .as_mut()
        .unwrap()
        .backend_parameters_strict = Some(false);
    assert!(snapshot
        .validate_tes(&advisory)
        .iter()
        .all(|d| !d.blocks_strict));
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(snapshot
        .validate_tes(&task)
        .iter()
        .all(|d| d.support == Support::Unknown && !d.blocks_strict));
}

/// A configuration for the API root `/<root>/` on the mock server.
fn config_below(root: &str) -> Configuration {
    Configuration::new(url::Url::parse(&format!("{}/{root}/", server_url())).unwrap())
}

#[tokio::test]
async fn discovered_accessors_read_client_and_snapshot() {
    let mut info = service("tes");
    info["id"] = json!("org.example.accessors");
    let _info = mock("GET", "/accessors/ga4gh/tes/v9/service-info")
        .with_body(info.to_string())
        .create();
    let config = config_below("accessors/ga4gh/tes/v9");
    let root = config.base_path.clone();
    let transport =
        Transport::with_timeouts(&config, Duration::from_secs(3), Duration::from_secs(7)).unwrap();

    let discovered = TES::discover_with_transport(transport, Duration::from_secs(60))
        .await
        .unwrap();

    assert!(std::ptr::eq(discovered.client(), &discovered.client));
    assert!(std::ptr::eq(
        discovered.capabilities(),
        &discovered.capabilities
    ));
    assert_eq!(discovered.client().configuration().base_path, root);
    // `Deref` still reaches the client's accessors.
    assert_eq!(
        discovered.configuration().timeouts.request,
        Duration::from_secs(7)
    );
    assert_eq!(
        discovered.discovered_service_info().id,
        "org.example.accessors"
    );
    assert_eq!(discovered.capabilities().raw["id"], "org.example.accessors");

    let (client, capabilities) = discovered.into_parts();
    assert_eq!(client.configuration().base_path, root);
    assert_eq!(client.discovered_service_info().id, "org.example.accessors");
    assert_eq!(capabilities.raw["id"], "org.example.accessors");
}

#[tokio::test]
async fn refresh_replaces_service_and_snapshot_together() {
    let mut info = service("wes");
    info["id"] = json!("first");
    let first = mock("GET", "/refresh/ga4gh/wes/v9/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let mut discovered = WES::discover(
        &config_below("refresh/ga4gh/wes/v9"),
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    first.assert();
    drop(first);
    assert_eq!(discovered.discovered_service_info().id, "first");
    assert_eq!(discovered.capabilities().raw["id"], "first");

    info["id"] = json!("second");
    let second = mock("GET", "/refresh/ga4gh/wes/v9/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    discovered.refresh().await.unwrap();
    second.assert();
    drop(second);
    assert_eq!(discovered.discovered_service_info().id, "second");
    assert_eq!(discovered.capabilities().raw["id"], "second");

    // A failed refresh keeps both values.
    let failed = mock("GET", "/refresh/ga4gh/wes/v9/service-info")
        .with_status(500)
        .expect(1)
        .create();
    discovered.refresh().await.unwrap_err();
    failed.assert();
    assert_eq!(discovered.discovered_service_info().id, "second");
    assert_eq!(discovered.capabilities().raw["id"], "second");
}
