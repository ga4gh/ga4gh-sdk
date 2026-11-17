//! The metadata contract of the generic Service Info client, tested through
//! `ServiceInfo::get()` and `get_at()` against local HTTP fixtures; see
//! `docs/api_compatibility.md#service-info`.

use ga4gh_sdk::clients::serviceinfo::{models::Service, ServiceInfo};
use ga4gh_sdk::utils::{
    configuration::Configuration,
    transport::{BodyLimitError, HttpError},
};
use ga4gh_sdk::Error;
use mockito::mock;
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

fn config(prefix: &str) -> Configuration {
    Configuration::new(Url::parse(&format!("{}/{prefix}", mockito::server_url())).unwrap())
}

fn client(prefix: &str) -> ServiceInfo {
    ServiceInfo::new(&config(prefix)).unwrap()
}

/// A document with every required field and no optional one.
fn minimal() -> Value {
    json!({
        "id": "org.example.service",
        "name": "Example service",
        "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0"
    })
}

/// Serves `body` at `path` below `/prefix`. Mockito's server is shared by all
/// tests, so each case needs a prefix of its own.
fn serve(prefix: &str, path: &str, body: impl AsRef<[u8]>) -> mockito::Mock {
    mock("GET", format!("/{prefix}/{path}").as_str())
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(body)
        .create()
}

async fn get(prefix: &str, body: impl AsRef<[u8]>) -> ga4gh_sdk::Result<Service> {
    let response = serve(prefix, "service-info", body);
    let result = client(prefix).get().await;
    response.assert();
    result
}

/// A change to a valid document that makes it invalid.
type Change = fn(&mut Value);

fn parse_error(error: &Error) -> &serde_json::Error {
    match error {
        Error::Parse(error) => error,
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[tokio::test]
async fn reads_all_specified_fields() {
    let body = json!({
        "id": "org.example.tes",
        "name": "Example TES",
        "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
        "description": "Runs tasks",
        "organization": {"name": "Example", "url": "https://example.org"},
        "contactUrl": "mailto:support@example.org",
        "documentationUrl": "https://docs.example.org",
        "createdAt": "2024-01-01T00:00:00Z",
        "updatedAt": "2024-06-01T00:00:00Z",
        "environment": "test",
        "version": "2.3.4"
    });
    let service = get("si-all", body.to_string()).await.unwrap();
    assert_eq!(service.id, "org.example.tes");
    assert_eq!(service.name, "Example TES");
    assert_eq!(
        (
            service.r#type.group.as_str(),
            service.r#type.artifact.as_str(),
            service.r#type.version.as_str()
        ),
        ("org.ga4gh", "tes", "1.1.0")
    );
    assert_eq!(service.description.as_deref(), Some("Runs tasks"));
    assert_eq!(service.organization.name, "Example");
    assert_eq!(service.organization.url, "https://example.org");
    assert_eq!(
        service.contact_url.as_deref(),
        Some("mailto:support@example.org")
    );
    assert_eq!(
        service.documentation_url.as_deref(),
        Some("https://docs.example.org")
    );
    assert_eq!(service.created_at.as_deref(), Some("2024-01-01T00:00:00Z"));
    assert_eq!(service.updated_at.as_deref(), Some("2024-06-01T00:00:00Z"));
    assert_eq!(service.environment.as_deref(), Some("test"));
    assert_eq!(service.version, "2.3.4");
    assert!(service.extra.is_empty());
    assert_eq!(serde_json::to_value(&service).unwrap(), body);
}

#[tokio::test]
async fn get_at_reads_another_path_below_the_api_root() {
    let response = serve("si-at", "v2/service-info", minimal().to_string());
    let service = client("si-at").get_at("v2/service-info").await.unwrap();
    assert_eq!(service.id, "org.example.service");
    response.assert();
}

#[tokio::test]
async fn custom_types_and_unknown_versions_are_returned_as_declared() {
    // `version` is the service software; `type.version` is the version of
    // the API named by `type.group` and `type.artifact`. Neither selects a
    // decoder, and neither has to be a known GA4GH value or semantic version.
    for (index, (group, artifact, api_version, version)) in [
        ("org.ga4gh", "tes", "99.0.0", "1.0.0"),
        ("org.ga4gh", "wes", "1.1", "2024-05-01"),
        (
            "org.example",
            "custom-api",
            "0.1.0-alpha+build.5",
            "3f2c1e9",
        ),
        ("com.example.legacy", "TES", "v1", "unversioned"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut body = minimal();
        body["type"] = json!({"group": group, "artifact": artifact, "version": api_version});
        body["version"] = json!(version);
        let prefix = format!("si-custom-{index}");
        let service = get(&prefix, body.to_string()).await.unwrap();
        assert_eq!(service.r#type.group, group);
        assert_eq!(service.r#type.artifact, artifact);
        assert_eq!(service.r#type.version, api_version);
        assert_eq!(service.version, version);
    }
}

#[tokio::test]
async fn inconsistent_identity_is_not_judged_by_the_generic_client() {
    // A name that suggests another API, an empty ID and the same value for
    // both versions: the generic client neither rejects nor corrects them.
    // Service-specific clients make their own compatibility decision.
    let mut body = minimal();
    body["id"] = json!("");
    body["name"] = json!("Example WES");
    body["type"]["version"] = json!("1.0.0");
    let service = get("si-inconsistent", body.to_string()).await.unwrap();
    assert_eq!(service.id, "");
    assert_eq!(service.name, "Example WES");
    assert_eq!(service.r#type.artifact, "tes");
    assert_eq!(service.r#type.version, service.version);
}

#[tokio::test]
async fn top_level_and_nested_extensions_are_preserved() {
    let mut body = minimal();
    body["storage"] = json!(["s3://bucket", {"region": "eu", "tiers": [1, 2]}]);
    body["x-provider"] = json!({"features": {"gpu": true}, "limits": null});
    body["empty"] = json!(null);
    body["type"]["x-profile"] = json!("strict");
    body["type"]["aliases"] = json!(["task-execution"]);
    body["organization"]["logo"] = json!({"href": "https://example.org/logo.svg"});
    let service = get("si-extensions", body.to_string()).await.unwrap();
    assert_eq!(service.extra["storage"], body["storage"]);
    assert_eq!(service.extra["x-provider"], body["x-provider"]);
    assert_eq!(service.extra["empty"], Value::Null);
    assert_eq!(service.r#type.extra["x-profile"], "strict");
    assert_eq!(service.r#type.extra["aliases"], json!(["task-execution"]));
    assert_eq!(
        service.organization.extra["logo"]["href"],
        "https://example.org/logo.svg"
    );
    // Writing the model back yields the same JSON value.
    assert_eq!(serde_json::to_value(&service).unwrap(), body);
}

#[tokio::test]
async fn omitted_and_null_optional_fields_both_read_as_none() {
    let optional = [
        "description",
        "contactUrl",
        "documentationUrl",
        "createdAt",
        "updatedAt",
        "environment",
    ];
    let omitted = get("si-omitted", minimal().to_string()).await.unwrap();
    let mut body = minimal();
    for field in optional {
        body[field] = Value::Null;
    }
    let null = get("si-null", body.to_string()).await.unwrap();
    for service in [&omitted, &null] {
        assert_eq!(service.description, None);
        assert_eq!(service.contact_url, None);
        assert_eq!(service.documentation_url, None);
        assert_eq!(service.created_at, None);
        assert_eq!(service.updated_at, None);
        assert_eq!(service.environment, None);
        assert!(service.extra.is_empty());
    }
    // The typed model can't tell `null` from an omitted field: both are
    // written back as omitted.
    assert_eq!(omitted, null);
    assert_eq!(serde_json::to_value(&null).unwrap(), minimal());
}

#[tokio::test]
async fn missing_or_null_required_fields_are_parse_errors() {
    // The JSON pointer of the object, its required field, and whether the
    // field is set to `null` (which isn't a value of a required field)
    // instead of being removed.
    let cases = [
        ("", "id", false),
        ("", "name", false),
        ("", "type", false),
        ("", "organization", false),
        ("", "version", false),
        ("/type", "group", false),
        ("/type", "artifact", false),
        ("/type", "version", false),
        ("/organization", "name", false),
        ("/organization", "url", false),
        ("", "version", true),
        ("/type", "version", true),
    ];
    for (index, (parent, field, null)) in cases.into_iter().enumerate() {
        let mut body = minimal();
        let object = body.pointer_mut(parent).unwrap().as_object_mut().unwrap();
        let expected = if null {
            object.insert(field.to_owned(), Value::Null);
            "invalid type: null".to_owned()
        } else {
            object.remove(field);
            format!("missing field `{field}`")
        };
        let error = get(&format!("si-missing-{index}"), body.to_string())
            .await
            .unwrap_err();
        let message = parse_error(&error).to_string();
        assert!(
            message.contains(&expected),
            "{parent}/{field} (null: {null}): {message}"
        );
    }
}

#[tokio::test]
async fn wrong_field_types_are_parse_errors() {
    let cases: [Change; 9] = [
        |b| b["id"] = json!(42),
        |b| b["version"] = json!(1.0),
        |b| b["type"] = json!("org.ga4gh:tes:1.1.0"),
        |b| b["type"]["version"] = json!(1.1),
        |b| b["organization"] = json!(["Example"]),
        |b| b["organization"]["url"] = json!({"href": "https://example.org"}),
        |b| b["description"] = json!({"en": "Runs tasks"}),
        // Unlike the TRS client's scoped exception for one registry, the
        // generic client doesn't drop numeric timestamps.
        |b| b["createdAt"] = json!(1_700_000_000),
        |b| b["environment"] = json!(true),
    ];
    for (index, change) in cases.into_iter().enumerate() {
        let mut body = minimal();
        change(&mut body);
        let error = get(&format!("si-type-{index}"), body.to_string())
            .await
            .unwrap_err();
        assert!(
            parse_error(&error).to_string().contains("invalid type"),
            "case {index}: {error}"
        );
    }
}

#[tokio::test]
async fn malformed_json_is_a_parse_error() {
    let complete = minimal().to_string();
    for (index, body) in [
        String::new(),
        "{".to_owned(),
        "not json".to_owned(),
        "[]".to_owned(),
        "null".to_owned(),
        complete[..complete.len() - 1].to_owned(),
        format!("{complete} trailing"),
        // A byte order mark isn't JSON (RFC 8259, section 8.1).
        format!("\u{feff}{complete}"),
    ]
    .into_iter()
    .enumerate()
    {
        let error = get(&format!("si-json-{index}"), body).await.unwrap_err();
        parse_error(&error);
    }
}

#[tokio::test]
async fn invalid_utf8_is_an_error_not_replaced_text() {
    let mut in_required = minimal();
    in_required["name"] = json!("MARKER");
    let mut in_extension = minimal();
    in_extension["x-note"] = json!("MARKER");
    for (index, body) in [in_required, in_extension].into_iter().enumerate() {
        let mut bytes = body.to_string().into_bytes();
        let at = bytes.windows(6).position(|s| s == b"MARKER").unwrap();
        bytes[at] = 0xff;
        let error = get(&format!("si-utf8-{index}"), bytes).await.unwrap_err();
        assert!(
            error.downcast_ref::<std::str::Utf8Error>().is_some(),
            "case {index}: {error:?}"
        );
    }
}

#[tokio::test]
async fn service_client_discovery_rejects_invalid_utf8_too() {
    use ga4gh_sdk::clients::{drs::DRS, tes::TES, wes::WES};
    // TES, WES and DRS, and capability discovery, read their service info with
    // the same request.
    let max_age = Duration::from_secs(60);
    for (case, artifact) in [
        ("tes-new", "tes"),
        ("wes-new", "wes"),
        ("drs-new", "drs"),
        ("tes-discover", "tes"),
        ("wes-discover", "wes"),
    ] {
        let mut body = minimal();
        body["type"]["artifact"] = json!(artifact);
        let mut bytes = body.to_string().into_bytes();
        let at = bytes.windows(7).position(|s| s == b"Example").unwrap();
        bytes[at] = 0xff;
        let prefix = format!("si-discovery-{case}");
        let response = serve(&prefix, "service-info", &bytes);
        let config = config(&prefix);
        let error = match case {
            "tes-new" => TES::new(&config).await.err(),
            "wes-new" => WES::new(&config).await.err(),
            "drs-new" => DRS::new(&config).await.err(),
            "tes-discover" => TES::discover(&config, max_age).await.err(),
            "wes-discover" => WES::discover(&config, max_age).await.err(),
            other => unreachable!("no discovery case {other}"),
        }
        .unwrap();
        assert!(
            error.downcast_ref::<std::str::Utf8Error>().is_some(),
            "{case}: {error:?}"
        );
        response.assert();
    }
}

#[tokio::test]
async fn http_and_authentication_failures_keep_status_and_body() {
    for status in [401, 403, 404, 500, 503] {
        let prefix = format!("si-http-{status}");
        let response = mock("GET", format!("/{prefix}/service-info").as_str())
            .match_header("authorization", "Bearer service-token")
            .with_status(status)
            .with_body(format!("{{\"msg\":\"status {status}\"}}"))
            .create();
        let config = config(&prefix).with_bearer_access_token("service-token".into());
        let error = ServiceInfo::new(&config).unwrap().get().await.unwrap_err();
        let http = match &error {
            Error::Http(http) => http,
            other => panic!("expected an HTTP error, got {other:?}"),
        };
        assert_eq!(http.status.as_u16(), status as u16);
        assert_eq!(http.body, format!("{{\"msg\":\"status {status}\"}}"));
        assert!(error.downcast_ref::<HttpError>().is_some());
        // The token isn't part of the error.
        assert!(!format!("{error} {error:?}").contains("service-token"));
        response.assert();
    }
}

#[tokio::test]
async fn an_error_body_over_the_limit_keeps_its_status() {
    // For example a login page of a proxy, larger than the 16 KiB default.
    for status in [401, 403, 404] {
        let prefix = format!("si-large-{status}");
        let response = mock("GET", format!("/{prefix}/service-info").as_str())
            .with_status(status)
            .with_body("x".repeat(20 * 1024))
            .create();
        let error = client(&prefix).get().await.unwrap_err();
        let limit = error.downcast_ref::<BodyLimitError>().unwrap();
        assert_eq!(limit.status.as_u16(), status as u16);
        assert!(error.downcast_ref::<HttpError>().is_none());
        response.assert();
    }
}

#[tokio::test]
async fn a_failed_parse_is_not_retried() {
    let response = mock("GET", "/si-once/service-info")
        .with_status(200)
        .with_body("{}")
        .expect(1)
        .create();
    client("si-once").get().await.unwrap_err();
    response.assert();
}
