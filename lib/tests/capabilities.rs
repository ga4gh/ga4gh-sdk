use ga4gh_sdk::{
    clients::{
        capabilities::{CapabilitySnapshot, Support},
        tes::{models::TesTask, TES},
        wes::{models::RunRequest, WES},
    },
    utils::{configuration::Configuration, transport::Transport},
};
use serde_json::json;
use std::time::Duration;
fn service(artifact: &str) -> serde_json::Value {
    json!({"id":"test","name":"test","type":{"group":"org.ga4gh","artifact":artifact,"version":"1.1"},"organization":{"name":"test","url":"https://example.org"},"version":"1"})
}
#[tokio::test]
async fn discovery_caches_raw_extensions_and_refreshes_once() {
    let mut server = mockito::Server::new_async().await;
    let mut info = service("tes");
    info["extension"] = json!({"custom":true});
    let first = server
        .mock("GET", "/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let mut client = TES::discover(&config, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(client.capabilities().raw["extension"]["custom"], true);
    assert_eq!(client.capabilities().tes_info().unwrap().name, "test");
    assert_eq!(client.capabilities().tes_info().unwrap().name, "test");
    first.assert();
    first.remove();
    info["extension"]["custom"] = json!(false);
    let refresh = server
        .mock("GET", "/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    client.refresh().await.unwrap();
    assert_eq!(client.capabilities().raw["extension"]["custom"], false);
    refresh.assert();
}
#[tokio::test]
async fn strict_wes_contradiction_prevents_submission_but_unknown_is_advisory() {
    let mut server = mockito::Server::new_async().await;
    let mut info = service("wes");
    info["workflow_type_versions"] = json!({"CWL":{"workflow_type_version":["v1.2"]}});
    let service = server
        .mock("GET", "/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submit = server
        .mock("POST", "/runs")
        .with_body(r#"{"run_id":"r1"}"#)
        .expect(0)
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
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
        client.capabilities().validate_wes(&request)[0].support,
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
fn config_below(server: &mockito::ServerGuard, root: &str) -> Configuration {
    Configuration::new(url::Url::parse(&format!("{}/{root}/", server.url())).unwrap())
}

#[tokio::test]
async fn discovered_accessors_read_client_and_snapshot() {
    let mut server = mockito::Server::new_async().await;
    let mut info = service("tes");
    info["id"] = json!("org.example.accessors");
    let _info = server
        .mock("GET", "/accessors/ga4gh/tes/v9/service-info")
        .with_body(info.to_string())
        .create();
    let config = config_below(&server, "accessors/ga4gh/tes/v9");
    let root = config.base_path.clone();
    let transport =
        Transport::with_timeouts(&config, Duration::from_secs(3), Duration::from_secs(7)).unwrap();

    let discovered = TES::discover_with_transport(transport, Duration::from_secs(60))
        .await
        .unwrap();

    // `Deref` reaches the same client as the accessor.
    assert!(std::ptr::eq(discovered.client(), &*discovered));
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
    let mut server = mockito::Server::new_async().await;
    let mut info = service("wes");
    info["id"] = json!("first");
    let first = server
        .mock("GET", "/refresh/ga4gh/wes/v9/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let mut discovered = WES::discover(
        &config_below(&server, "refresh/ga4gh/wes/v9"),
        Duration::from_secs(60),
    )
    .await
    .unwrap();
    first.assert();
    first.remove();
    assert_eq!(discovered.discovered_service_info().id, "first");
    assert_eq!(discovered.capabilities().raw["id"], "first");

    info["id"] = json!("second");
    let second = server
        .mock("GET", "/refresh/ga4gh/wes/v9/service-info")
        .with_body(info.to_string())
        .expect(1)
        .create();
    discovered.refresh().await.unwrap();
    second.assert();
    second.remove();
    assert_eq!(discovered.discovered_service_info().id, "second");
    assert_eq!(discovered.capabilities().raw["id"], "second");

    // A failed refresh keeps both values.
    let failed = server
        .mock("GET", "/refresh/ga4gh/wes/v9/service-info")
        .with_status(500)
        .expect(1)
        .create();
    discovered.refresh().await.unwrap_err();
    failed.assert();
    assert_eq!(discovered.discovered_service_info().id, "second");
    assert_eq!(discovered.capabilities().raw["id"], "second");
}

/// A run request for `type_version` of CWL, with the engine and engine
/// version given.
fn cwl_request(
    type_version: &str,
    engine: Option<&str>,
    engine_version: Option<&str>,
) -> RunRequest {
    RunRequest {
        workflow_type: "CWL".into(),
        workflow_type_version: type_version.into(),
        workflow_url: "workflow.cwl".into(),
        workflow_params: Some(json!({})),
        workflow_engine: engine.map(Into::into),
        workflow_engine_version: engine_version.map(Into::into),
        ..Default::default()
    }
}

/// The support of `field` in the diagnostics of `request`; panics if the
/// field wasn't checked.
fn support_of(snapshot: &CapabilitySnapshot, request: &RunRequest, field: &str) -> Support {
    snapshot
        .validate_wes(request)
        .into_iter()
        .find(|d| d.field == field)
        .unwrap_or_else(|| panic!("no diagnostic for {field}"))
        .support
}

#[test]
fn wes_type_version_is_checked_against_the_advertised_list() {
    let advertised = CapabilitySnapshot::from_raw(
        json!({"workflow_type_versions":{"CWL":{"workflow_type_version":["v1.2"]}}}),
        Duration::from_secs(60),
    );
    let field = "workflow_type_version";
    assert_eq!(
        support_of(&advertised, &cwl_request("v1.2", None, None), field),
        Support::Supported
    );
    let unsupported = advertised
        .validate_wes(&cwl_request("v1.0", None, None))
        .into_iter()
        .find(|d| d.field == field)
        .unwrap();
    assert_eq!(unsupported.support, Support::Unsupported);
    assert!(unsupported.blocks_strict);
    assert_eq!(unsupported.detail, "requested version v1.0");
    // An empty list or a language without versions advertises nothing
    for raw in [
        json!({"workflow_type_versions":{"CWL":{"workflow_type_version":[]}}}),
        json!({"workflow_type_versions":{"CWL":{}}}),
    ] {
        let snapshot = CapabilitySnapshot::from_raw(raw, Duration::from_secs(60));
        assert_eq!(
            support_of(&snapshot, &cwl_request("v1.2", None, None), field),
            Support::Unknown
        );
    }
}

#[test]
fn wes_engine_is_checked_against_the_advertised_engines() {
    let advertised = CapabilitySnapshot::from_raw(
        json!({"workflow_engine_versions":{"cwltool":{"workflow_engine_version":["3.1"]}}}),
        Duration::from_secs(60),
    );
    let field = "workflow_engine";
    assert_eq!(
        support_of(
            &advertised,
            &cwl_request("v1.2", Some("cwltool"), None),
            field
        ),
        Support::Supported
    );
    let unsupported = advertised
        .validate_wes(&cwl_request("v1.2", Some("toil"), None))
        .into_iter()
        .find(|d| d.field == field)
        .unwrap();
    assert_eq!(unsupported.support, Support::Unsupported);
    assert!(unsupported.blocks_strict);
    assert_eq!(unsupported.detail, "requested engine toil");
    // Without an engine version, the engine version isn't checked
    assert!(advertised
        .validate_wes(&cwl_request("v1.2", Some("cwltool"), None))
        .iter()
        .all(|d| d.field != "workflow_engine_version"));
    // Missing or empty engine maps advertise nothing
    for raw in [json!({}), json!({"workflow_engine_versions":{}})] {
        let snapshot = CapabilitySnapshot::from_raw(raw, Duration::from_secs(60));
        assert_eq!(
            support_of(
                &snapshot,
                &cwl_request("v1.2", Some("cwltool"), None),
                field
            ),
            Support::Unknown
        );
    }
}

#[test]
fn wes_engine_version_accepts_lists_and_single_strings() {
    let field = "workflow_engine_version";
    let cases = [
        // WES 1.1: a list of versions
        (
            json!({"workflow_engine_version":["3.1"]}),
            "3.1",
            Support::Supported,
        ),
        (
            json!({"workflow_engine_version":["3.1"]}),
            "3.0",
            Support::Unsupported,
        ),
        // WES 1.0: a single string
        (json!("3.1"), "3.1", Support::Supported),
        (json!("3.1"), "3.0", Support::Unsupported),
        // Nothing usable advertised
        (
            json!({"workflow_engine_version":[]}),
            "3.1",
            Support::Unknown,
        ),
        (json!({}), "3.1", Support::Unknown),
    ];
    for (advertised, version, expected) in cases {
        let snapshot = CapabilitySnapshot::from_raw(
            json!({"workflow_engine_versions":{"cwltool":advertised}}),
            Duration::from_secs(60),
        );
        let request = cwl_request("v1.2", Some("cwltool"), Some(version));
        let diagnostic = snapshot
            .validate_wes(&request)
            .into_iter()
            .find(|d| d.field == field)
            .unwrap();
        assert_eq!(diagnostic.support, expected, "{advertised} vs {version}");
        assert_eq!(diagnostic.blocks_strict, expected == Support::Unsupported);
        assert_eq!(
            diagnostic.detail,
            format!("requested engine version {version}")
        );
    }
    // An unadvertised engine leaves its version unknown
    let snapshot = CapabilitySnapshot::from_raw(
        json!({"workflow_engine_versions":{"cwltool":"3.1"}}),
        Duration::from_secs(60),
    );
    assert_eq!(
        support_of(
            &snapshot,
            &cwl_request("v1.2", Some("toil"), Some("3.1")),
            field
        ),
        Support::Unknown
    );
}

#[test]
fn wes_info_parses_only_wes_service_info() {
    let mut info = service("wes");
    info["workflow_type_versions"] = json!({"CWL":{"workflow_type_version":["v1.2"]}});
    info["supported_wes_versions"] = json!(["1.1.0"]);
    info["supported_filesystem_protocols"] = json!(["https"]);
    info["workflow_engine_versions"] = json!({"cwltool":{"workflow_engine_version":["3.1"]}});
    info["default_workflow_engine_parameters"] = json!([]);
    info["system_state_counts"] = json!({});
    info["auth_instructions_url"] = json!("https://example.org/auth");
    info["tags"] = json!({});
    let snapshot = CapabilitySnapshot::from_raw(info, Duration::from_secs(60));
    let parsed = snapshot.wes_info().unwrap();
    assert_eq!(parsed.supported_wes_versions, ["1.1.0"]);
    assert!(parsed.workflow_type_versions.contains_key("CWL"));
    let tes = CapabilitySnapshot::from_raw(service("tes"), Duration::from_secs(60));
    tes.wes_info().unwrap_err();
}

#[test]
fn tes_parameters_without_advertised_names_are_unknown() {
    let snapshot = CapabilitySnapshot::from_raw(json!({}), Duration::from_secs(60));
    let task: TesTask = serde_json::from_value(json!({
        "executors": [],
        "resources": {"backend_parameters": {"VmSize": "large"}, "backend_parameters_strict": true}
    }))
    .unwrap();
    let diagnostics = snapshot.validate_tes(&task);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].field, "resources.backend_parameters.VmSize");
    assert_eq!(diagnostics[0].support, Support::Unknown);
    assert!(!diagnostics[0].blocks_strict);
    // Tasks without resources or backend parameters give no diagnostics
    for task in [
        json!({"executors": []}),
        json!({"executors": [], "resources": {"cpu_cores": 1}}),
    ] {
        let task: TesTask = serde_json::from_value(task).unwrap();
        assert!(snapshot.validate_tes(&task).is_empty());
    }
}

#[tokio::test]
async fn tes_create_validated_submits_unless_strictly_blocked() {
    let mut server = mockito::Server::new_async().await;
    let mut info = service("tes");
    info["tesResources_backend_parameters"] = json!(["VmSize"]);
    let _info = server
        .mock("GET", "/service-info")
        .with_body(info.to_string())
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let client = TES::discover(&config, Duration::from_secs(60))
        .await
        .unwrap();
    let blocked: TesTask = serde_json::from_value(json!({
        "executors": [{"image": "alpine", "command": ["true"]}],
        "resources": {"backend_parameters": {"unknown": "x"}, "backend_parameters_strict": true}
    }))
    .unwrap();
    let no_submit = server.mock("POST", "/tasks").expect(0).create();
    let error = client
        .create_validated(blocked.clone(), true)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("submission preflight: resources.backend_parameters.unknown"));
    no_submit.assert();
    no_submit.remove();

    let submit = server
        .mock("POST", "/tasks")
        .with_body(r#"{"id":"t1"}"#)
        .expect(2)
        .create();
    // Not strict: the blocking diagnostic is advisory
    assert_eq!(
        client.create_validated(blocked, false).await.unwrap().id,
        "t1"
    );
    // Strict without blocking diagnostics
    let allowed: TesTask = serde_json::from_value(json!({
        "executors": [{"image": "alpine", "command": ["true"]}],
        "resources": {"backend_parameters": {"vmsize": "large"}, "backend_parameters_strict": true}
    }))
    .unwrap();
    assert_eq!(
        client.create_validated(allowed, true).await.unwrap().id,
        "t1"
    );
    submit.assert();
}

#[tokio::test]
async fn wes_run_validated_submits_supported_requests() {
    let mut server = mockito::Server::new_async().await;
    let mut info = service("wes");
    info["workflow_type_versions"] = json!({"CWL":{"workflow_type_version":["v1.2"]}});
    let _info = server
        .mock("GET", "/service-info")
        .with_body(info.to_string())
        .create();
    let submit = server
        .mock("POST", "/runs")
        .with_body(r#"{"run_id":"r1"}"#)
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let client = WES::discover(&config, Duration::from_secs(60))
        .await
        .unwrap();
    let run = client
        .run_validated(cwl_request("v1.2", None, None), true)
        .await
        .unwrap();
    assert_eq!(run.id, "r1");
    submit.assert();
}
