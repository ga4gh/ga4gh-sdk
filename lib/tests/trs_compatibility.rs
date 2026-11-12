//! The public client must select a contract once and keep it across every route.
use ga4gh_sdk::clients::trs::{
    models::ToolVersion, responses, ToolFilter, TrsOptions, TrsVersion, TRS,
};
use ga4gh_sdk::compatibility::{
    CapabilityOverride, CapabilitySupport, CompatibilityError, SelectionSource, VersionPolicy,
    VersionSelection,
};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::{BodyLimitError, BodyLimits, HttpError, Transport};
use mockito::{mock, Matcher, Mock};
use serde_json::{json, Value};
use url::Url;

fn config(prefix: &str) -> Configuration {
    Configuration::new(Url::parse(&format!("{}/{prefix}/trs/v2", mockito::server_url())).unwrap())
}
fn get(prefix: &str, route: &str, body: impl AsRef<[u8]>) -> Mock {
    mock("GET", format!("/{prefix}/trs/v2/{route}").as_str())
        .match_header("accept", "application/json")
        .with_status(200)
        .with_body(body)
        .create()
}
fn info(version: &str) -> Value {
    json!({"id":"test", "name":"Test TRS", "type":{"group":"org.ga4gh", "artifact":"trs", "version":version},
        "organization":{"name":"Test", "url":"https://example.org"}, "version":"server-99"})
}
fn version(description: Value) -> Value {
    json!({"id":"v1", "url":"https://example.org/versions/v1", "description":description,
        "descriptor_type":["CWL", "FUTURE_LANG"], "provider":{"nested":[1,null,"x"]}})
}
fn tool(version: Value) -> Value {
    json!({"id":"t", "url":"https://example.org/tools/t", "organization":"test",
        "toolclass":{"id":"workflow"}, "versions":[version], "provider":{"untouched":true}})
}
fn explicit(prefix: &str, profile: TrsVersion) -> TRS {
    TRS::without_discovery(Transport::new(&config(prefix)), profile)
}

#[test]
fn selection_is_exact_and_retains_provenance() {
    let supported = [("2.0.1", 1), ("2.0.2", 2)];
    for declared in [None, Some(""), Some("2.1.0"), Some("3.0.0")] {
        let selection =
            VersionSelection::resolve("TRS", declared, VersionPolicy::Auto, &supported, 1).unwrap();
        assert_eq!(selection.profile(), 1);
        assert_eq!(selection.declared_version(), declared);
        assert_eq!(selection.source(), SelectionSource::Baseline);
        assert!(matches!(
            VersionSelection::resolve("TRS", declared, VersionPolicy::RequireKnown, &supported, 1),
            Err(CompatibilityError::UnknownVersion { .. })
        ));
    }
    let selection = VersionSelection::resolve(
        "TRS",
        Some("2.0.1"),
        VersionPolicy::Explicit(2),
        &supported,
        1,
    )
    .unwrap();
    assert_eq!(selection.profile(), 2);
    assert_eq!(selection.declared_version(), Some("2.0.1"));
    assert_eq!(selection.source(), SelectionSource::Caller);
}

#[tokio::test]
async fn independent_clients_use_declared_profiles_and_preserve_extensions() {
    for (prefix, declared, expected) in [
        ("compat201", "2.0.1", TrsVersion::V2_0_1),
        ("compat202", "2.0.2", TrsVersion::V2_0_2),
    ] {
        let discovery = get(prefix, "service-info", info(declared).to_string());
        let payload = version(json!("Readable description"));
        let response = get(prefix, "tools/t/versions/v1", payload.to_string());
        let client = TRS::new(&config(prefix)).await.unwrap();
        assert_eq!(client.compatibility().selection().profile(), expected);
        assert_eq!(
            client.compatibility().selection().source(),
            SelectionSource::DeclaredVersion
        );
        assert_eq!(
            client.discovered_service_info().unwrap().version,
            "server-99"
        );
        let actual = client.get_version("t", "v1").await.unwrap();
        assert_eq!(actual.description(), Some("Readable description"));
        assert_eq!(serde_json::to_value(actual).unwrap(), payload);
        discovery.assert();
        response.assert();
    }
}

#[tokio::test]
async fn selected_decoder_is_used_for_every_typed_route_without_retry() {
    for (prefix, profile, succeeds) in [
        ("routes201", TrsVersion::V2_0_1, true),
        ("routes202", TrsVersion::V2_0_2, false),
    ] {
        let client = explicit(prefix, profile);
        let v = version(json!({"provider_specific": 42}));
        let t = tool(v.clone());
        let one = get(prefix, "tools/t", t.to_string());
        let versions = get(prefix, "tools/t/versions", json!([v.clone()]).to_string());
        let single_version = get(prefix, "tools/t/versions/v1", v.to_string());
        let list = mock("GET", format!("/{prefix}/trs/v2/tools").as_str())
            .with_status(200)
            .with_body(json!([t.clone()]).to_string())
            .expect(2)
            .create();
        let next = mock("GET", format!("/{prefix}/trs/v2/tools").as_str())
            .match_query(Matcher::Exact("offset=opaque%2Btoken".into()))
            .with_status(200)
            .with_body(json!([t]).to_string())
            .create();
        assert_eq!(client.get_tool("t").await.is_ok(), succeeds);
        assert_eq!(client.list_versions("t").await.is_ok(), succeeds);
        let result = client.get_version("t", "v1").await;
        if succeeds {
            let result = result.unwrap();
            assert_eq!(result.description(), None);
            assert_eq!(serde_json::to_value(result).unwrap(), v);
        } else {
            assert!(result
                .unwrap_err()
                .downcast_ref::<serde_json::Error>()
                .is_some());
        }
        assert_eq!(
            client.list_tools(&ToolFilter::default()).await.is_ok(),
            succeeds
        );
        assert_eq!(
            responses::list_tools_page(&client, None, BodyLimits::default())
                .await
                .is_ok(),
            succeeds
        );
        assert_eq!(
            responses::next_tools_page(&client, "?offset=opaque%2Btoken", BodyLimits::default())
                .await
                .is_ok(),
            succeeds
        );
        one.assert();
        versions.assert();
        single_version.assert();
        list.assert();
        next.assert();
    }
}

#[tokio::test]
async fn unknown_version_has_unverified_reads_but_no_assumed_tags() {
    let prefix = "unknown";
    let discovery = get(prefix, "service-info", info("2.1.0").to_string());
    let response = get(
        prefix,
        "tools/t/versions/v1",
        version(json!(23)).to_string(),
    );
    let no_tools = mock("GET", "/unknown/trs/v2/tools").expect(0).create();
    let client = TRS::new(&config(prefix)).await.unwrap();
    assert_eq!(
        client.compatibility().selection().source(),
        SelectionSource::Baseline
    );
    assert_eq!(
        client.compatibility().selection().declared_version(),
        Some("2.1.0")
    );
    assert_eq!(
        client.compatibility().tag_filter(),
        CapabilitySupport::Unknown
    );
    assert_eq!(
        client.get_version("t", "v1").await.unwrap().extra["description"],
        23
    );
    let error = client
        .list_tools(&ToolFilter {
            tags: vec!["genomics".into()],
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<CompatibilityError>(),
        Some(CompatibilityError::Capability {
            support: CapabilitySupport::Unknown,
            ..
        })
    ));
    assert!(
        responses::next_tools_page(&client, "?tags=genomics", BodyLimits::default())
            .await
            .is_err()
    );
    discovery.assert();
    response.assert();
    no_tools.assert();
}

#[tokio::test]
async fn strict_discovery_and_explicit_override_remain_distinct() {
    let prefix = "override";
    let discovery = mock("GET", "/override/trs/v2/service-info")
        .with_status(200)
        .with_body(info("future").to_string())
        .expect(2)
        .create();
    let error = TRS::with_options(
        &config(prefix),
        TrsOptions {
            version: VersionPolicy::RequireKnown,
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<CompatibilityError>().is_some());
    let client = TRS::with_options(
        &config(prefix),
        TrsOptions {
            version: VersionPolicy::Explicit(TrsVersion::V2_0_2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        client.compatibility().selection().declared_version(),
        Some("future")
    );
    assert_eq!(
        client.compatibility().selection().source(),
        SelectionSource::Caller
    );
    let payload = get(
        prefix,
        "tools/t/versions/v1",
        version(json!(42)).to_string(),
    );
    assert!(client.get_version("t", "v1").await.is_err());
    discovery.assert();
    payload.assert();
}

#[tokio::test]
async fn tags_require_support_on_requests_and_pagination() {
    let tags = ToolFilter {
        tags: vec!["a b".into(), "c".into()],
        ..Default::default()
    };
    let disabled = explicit("tags-disabled", TrsVersion::V2_0_2)
        .with_tag_filter_override(CapabilityOverride::Disabled);
    let denied = mock("GET", "/tags-disabled/trs/v2/tools")
        .expect(0)
        .create();
    assert!(disabled.list_tools(&tags).await.is_err());
    assert!(
        responses::list_tools_page(&disabled, tags.query().unwrap(), BodyLimits::default())
            .await
            .is_err()
    );
    assert!(
        responses::next_tools_page(&disabled, "?%74ags=a", BodyLimits::default())
            .await
            .is_err()
    );
    denied.assert();
    let client = explicit("tags-backport", TrsVersion::V2_0_1)
        .with_tag_filter_override(CapabilityOverride::Enabled);
    let allowed = mock("GET", "/tags-backport/trs/v2/tools")
        .match_query(Matcher::UrlEncoded("tags".into(), "a b,c".into()))
        .with_status(200)
        .with_body("[]")
        .expect(3)
        .create();
    client.list_tools(&tags).await.unwrap();
    responses::list_tools_page(&client, tags.query().unwrap(), BodyLimits::default())
        .await
        .unwrap();
    responses::next_tools_page(&client, "?tags=a%20b%2Cc", BodyLimits::default())
        .await
        .unwrap();
    assert_eq!(
        client.compatibility().selection().profile(),
        TrsVersion::V2_0_1
    );
    assert_eq!(
        client.compatibility().tag_filter(),
        CapabilitySupport::CallerEnabled
    );
    allowed.assert();
}

#[tokio::test]
async fn discovery_failure_is_not_an_automatic_bypass() {
    for (prefix, status, body) in [
        ("noinfo", 404, "missing"),
        ("denied", 401, "denied"),
        ("brokeninfo", 200, "{}"),
    ] {
        let discovery = mock("GET", format!("/{prefix}/trs/v2/service-info").as_str())
            .with_status(status)
            .with_body(body)
            .create();
        let result = TRS::with_options(
            &config(prefix),
            TrsOptions {
                version: VersionPolicy::Explicit(TrsVersion::V2_0_1),
                ..Default::default()
            },
        )
        .await;
        let error = result.unwrap_err();
        if status != 200 {
            assert_eq!(
                error.downcast_ref::<HttpError>().unwrap().status.as_u16(),
                status as u16
            );
        }
        let client = explicit(prefix, TrsVersion::V2_0_1);
        assert!(client.discovered_service_info().is_none());
        let tools = get(prefix, "tools", "[]");
        client.list_tools(&ToolFilter::default()).await.unwrap();
        discovery.assert();
        tools.assert();
    }
}

#[tokio::test]
async fn malformed_json_and_invalid_utf8_are_errors_not_repaired_data() {
    for (index, body) in [
        b"{broken".as_slice(),
        b"{\"url\":\"https://example.org\",\"id\":\"\xff\"}",
        b"{}",
        b"[]",
    ]
    .iter()
    .enumerate()
    {
        let prefix = format!("malformed-{index}");
        let response = get(&prefix, "tools/t/versions/v1", body);
        let error = explicit(&prefix, TrsVersion::V2_0_2)
            .get_version("t", "v1")
            .await
            .unwrap_err();
        if index == 1 {
            assert!(error.downcast_ref::<std::str::Utf8Error>().is_some());
        } else {
            assert!(error.downcast_ref::<serde_json::Error>().is_some());
        }
        response.assert();
    }
    let body = info("2.0.1").to_string().replace("Test TRS", "MARKER");
    let mut invalid = body.into_bytes();
    let at = invalid.windows(6).position(|s| s == b"MARKER").unwrap();
    invalid[at] = 255;
    let discovery = get("bad-utf8-info", "service-info", invalid);
    assert!(TRS::new(&config("bad-utf8-info"))
        .await
        .unwrap_err()
        .downcast_ref::<std::str::Utf8Error>()
        .is_some());
    discovery.assert();
}

#[tokio::test]
async fn bounded_bodies_and_http_status_survive_the_selected_decoder() {
    let response = mock("GET", "/http-error/trs/v2/tools/t")
        .with_status(503)
        .with_body("unavailable")
        .create();
    let error = explicit("http-error", TrsVersion::V2_0_2)
        .get_tool("t")
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpError>().unwrap().status.as_u16(),
        503
    );
    response.assert();
    let response = get(
        "limited",
        "tools",
        json!([tool(version(json!("long description")))]).to_string(),
    );
    let client = explicit("limited", TrsVersion::V2_0_2);
    let error = responses::list_tools_page(
        &client,
        None,
        BodyLimits {
            success: 8,
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(error.downcast_ref::<BodyLimitError>().is_some());
    response.assert();
}

#[tokio::test]
async fn legacy_transport_helpers_keep_baseline_and_containment() {
    let transport = Transport::new(&config("legacy-page"));
    let payload = tool(version(json!(42)));
    let response = mock("GET", "/legacy-page/trs/v2/tools")
        .with_status(200)
        .with_body(json!([payload.clone()]).to_string())
        .expect(2)
        .create();
    let page = responses::list_tools_page(&transport, None, BodyLimits::default())
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&page.tools[0]).unwrap(), payload);
    responses::list_tools_page(
        &transport,
        Some(json!({"tags":null})),
        BodyLimits::default(),
    )
    .await
    .unwrap();
    assert!(responses::next_tools_page(
        &transport,
        "https://example.org/tools",
        BodyLimits::default()
    )
    .await
    .is_err());
    assert!(responses::list_tools_page(
        &transport,
        Some(json!({"tags":"a"})),
        BodyLimits::default()
    )
    .await
    .is_err());
    response.assert();
}

#[test]
fn old_public_tool_version_struct_literal_still_compiles() {
    let version = ToolVersion {
        author: None,
        name: None,
        url: "https://example.org".into(),
        id: "v1".into(),
        is_production: None,
        images: None,
        descriptor_type: None,
        descriptor_type_version: None,
        containerfile: None,
        meta_version: None,
        verified: None,
        verified_source: None,
        signed: None,
        included_apps: None,
        extra: Default::default(),
    };
    assert_eq!(version.description(), None);
}

#[tokio::test]
async fn simultaneous_connections_keep_independent_profiles() {
    let old = explicit("simultaneous201", TrsVersion::V2_0_1);
    let new = explicit("simultaneous202", TrsVersion::V2_0_2);
    let payload = version(json!(42)).to_string();
    let old_response = get("simultaneous201", "tools/t/versions/v1", &payload);
    let new_response = get("simultaneous202", "tools/t/versions/v1", &payload);
    let (old_result, new_result) =
        tokio::join!(old.get_version("t", "v1"), new.get_version("t", "v1"));
    assert_eq!(old_result.unwrap().extra["description"], 42);
    assert!(new_result
        .unwrap_err()
        .downcast_ref::<serde_json::Error>()
        .is_some());
    old_response.assert();
    new_response.assert();
}

#[tokio::test]
async fn common_operations_and_zip_are_available_under_both_profiles() {
    use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
    for (prefix, profile) in [
        ("common201", TrsVersion::V2_0_1),
        ("common202", TrsVersion::V2_0_2),
    ] {
        let client = explicit(prefix, profile);
        let classes = get(prefix, "toolClasses", r#"[{"id":"workflow"}]"#);
        let files = get(
            prefix,
            "tools/t/versions/v1/CWL/files",
            r#"[{"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR"}]"#,
        );
        let descriptor = get(
            prefix,
            "tools/t/versions/v1/CWL/descriptor",
            r#"{"content":"class: Workflow"}"#,
        );
        let tests = get(
            prefix,
            "tools/t/versions/v1/CWL/tests",
            r#"[{"content":"{}"}]"#,
        );
        let archive = mock(
            "GET",
            format!("/{prefix}/trs/v2/tools/t/versions/v1/CWL/files").as_str(),
        )
        .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
        .with_status(200)
        .with_body(b"PK\x05\x06archive")
        .create();
        assert_eq!(client.list_tool_classes().await.unwrap().len(), 1);
        assert_eq!(
            client
                .list_files("t", "v1", DescriptorType::Cwl)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            client
                .get_descriptor("t", "v1", DescriptorTypeWithPlain::Cwl)
                .await
                .unwrap()
                .content
                .as_deref(),
            Some("class: Workflow")
        );
        assert_eq!(
            client
                .list_tests("t", "v1", DescriptorTypeWithPlain::Cwl)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            client
                .get_files_zip("t", "v1", DescriptorType::Cwl, 1024)
                .await
                .unwrap(),
            b"PK\x05\x06archive"
        );
        classes.assert();
        files.assert();
        descriptor.assert();
        tests.assert();
        archive.assert();
    }
}

#[tokio::test]
async fn nullable_descriptions_are_preserved_under_both_profiles() {
    for (prefix, profile) in [
        ("nullable201", TrsVersion::V2_0_1),
        ("nullable202", TrsVersion::V2_0_2),
    ] {
        let payload = version(Value::Null);
        let response = get(prefix, "tools/t/versions/v1", payload.to_string());
        let actual = explicit(prefix, profile)
            .get_version("t", "v1")
            .await
            .unwrap();
        assert_eq!(actual.description(), None);
        assert_eq!(serde_json::to_value(actual).unwrap(), payload);
        response.assert();
    }
}
