use ga4gh_sdk::clients::{
    drs::DRS, registry::ServiceRegistry, serviceinfo::ServiceInfo, tes::TES, trs::TRS, wes::WES,
};
use ga4gh_sdk::utils::configuration::Configuration;
use mockito::mock;
use reqwest::{header, Client};
use serde_json::json;
use url::Url;

fn config(prefix: &str) -> Configuration {
    Configuration::new(Url::parse(&format!("{}/{prefix}", mockito::server_url())).unwrap())
        .with_bearer_access_token("service-token".into())
}

fn service_info(artifact: &str) -> String {
    json!({
        "id": format!("org.example.{artifact}"),
        "name": "Protected service",
        "type": {"group":"org.ga4gh", "artifact":artifact, "version":"2.0.2"},
        "organization":{"name":"Example", "url":"https://example.org"},
        "version":"1.0.0"
    })
    .to_string()
}

fn protected_get(path: &str, body: String) -> mockito::Mock {
    mock("GET", path)
        .match_header("authorization", "Bearer service-token")
        .with_body(body)
        .create()
}

#[tokio::test]
async fn service_info_tes_and_drs_use_auth_for_discovery_and_operations() {
    let info = protected_get("/auth-info/service-info", service_info("test"));
    ServiceInfo::new(&config("auth-info"))
        .unwrap()
        .get()
        .await
        .unwrap();
    info.assert();

    let tes_info = protected_get("/auth-tes/service-info", service_info("tes"));
    let tasks = protected_get("/auth-tes/tasks", json!({"tasks": []}).to_string());
    TES::new(&config("auth-tes"))
        .await
        .unwrap()
        .list_tasks(None)
        .await
        .unwrap();
    tes_info.assert();
    tasks.assert();

    let drs_info = protected_get("/auth-drs/service-info", service_info("drs"));
    let object = protected_get(
        "/auth-drs/objects/object-1",
        json!({"id":"object-1"}).to_string(),
    );
    let body = DRS::new(&config("auth-drs"))
        .await
        .unwrap()
        .get_object_raw("object-1")
        .await
        .unwrap();
    assert!(body.contains("object-1"));
    drs_info.assert();
    object.assert();
}

#[tokio::test]
async fn wes_trs_and_registry_use_auth_for_discovery_and_operations() {
    let wes_info = protected_get(
        "/auth-wes/service-info",
        json!({
            "workflow_type_versions":{"CWL":{"workflow_type_version":["v1.2"]}},
            "supported_wes_versions":["1.0.0"],
            "supported_filesystem_protocols":["file"],
            "workflow_engine_versions":{},
            "default_workflow_engine_parameters":[],
            "system_state_counts":{},
            "tags":{}
        })
        .to_string(),
    );
    let runs = protected_get("/auth-wes/runs", json!({"runs": []}).to_string());
    WES::new(&config("auth-wes"))
        .await
        .unwrap()
        .list_runs(None, None)
        .await
        .unwrap();
    wes_info.assert();
    runs.assert();

    let trs_info = protected_get("/auth-trs/service-info", service_info("trs"));
    let classes = protected_get("/auth-trs/toolClasses", "[]".into());
    TRS::new(&config("auth-trs"))
        .await
        .unwrap()
        .list_tool_classes()
        .await
        .unwrap();
    trs_info.assert();
    classes.assert();

    let registry_info = protected_get("/auth-registry/service-info", service_info("registry"));
    let services = protected_get("/auth-registry/services", "[]".into());
    let registry = ServiceRegistry::new(&config("auth-registry"));
    registry.service_info().await.unwrap();
    registry.list_services().await.unwrap();
    registry_info.assert();
    services.assert();
}

#[tokio::test]
async fn tes_and_drs_keep_caller_owned_client_through_discovery_and_operation() {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        "x-caller-policy",
        header::HeaderValue::from_static("preserved"),
    );
    let client = Client::builder().default_headers(headers).build().unwrap();
    let tes_info = mock("GET", "/injected-tes/service-info")
        .match_header("x-caller-policy", "preserved")
        .with_body(service_info("tes"))
        .create();
    let tasks = mock("GET", "/injected-tes/tasks")
        .match_header("x-caller-policy", "preserved")
        .with_body(json!({"tasks": []}).to_string())
        .create();
    let transport = ga4gh_sdk::utils::transport::Transport::with_client(
        &Configuration::new(
            Url::parse(&format!("{}/injected-tes", mockito::server_url())).unwrap(),
        ),
        client.clone(),
    );
    TES::with_transport(transport)
        .await
        .unwrap()
        .list_tasks(None)
        .await
        .unwrap();
    tes_info.assert();
    tasks.assert();

    let drs_info = mock("GET", "/injected-drs/service-info")
        .match_header("x-caller-policy", "preserved")
        .with_body(service_info("drs"))
        .create();
    let object = mock("GET", "/injected-drs/objects/x")
        .match_header("x-caller-policy", "preserved")
        .with_body(json!({"id": "x"}).to_string())
        .create();
    let transport = ga4gh_sdk::utils::transport::Transport::with_client(
        &Configuration::new(
            Url::parse(&format!("{}/injected-drs", mockito::server_url())).unwrap(),
        ),
        client,
    );
    DRS::with_transport(transport)
        .await
        .unwrap()
        .get_object_raw("x")
        .await
        .unwrap();
    drs_info.assert();
    object.assert();
}
