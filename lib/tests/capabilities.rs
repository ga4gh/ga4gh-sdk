use ga4gh_sdk::{
    clients::{
        capabilities::{CapabilitySnapshot, Support},
        tes::{models::TesTask, TES},
        wes::{models::WesRunRequest, WES},
    },
    utils::configuration::Configuration,
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
    let request = WesRunRequest {
        workflow_type: Some("WDL".into()),
        workflow_type_version: Some("1.0".into()),
        workflow_url: Some("workflow.wdl".into()),
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
