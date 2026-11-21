use ga4gh_sdk::{
    clients::drs::{
        models::access_method::Type,
        uri::{
            AccessChoiceError, AccessPreferences, AccessResolution, DrsResolver, DrsUri, UriError,
        },
    },
    utils::{
        configuration::Configuration,
        operation::{Operation, OperationError},
        transport::Transport,
    },
};
use serde_json::json;
use std::time::Duration;
fn budget() -> Operation {
    Operation::new(Duration::from_secs(3))
}
fn resolver(server: &mockito::ServerGuard) -> DrsResolver {
    let transport = Transport::new(&Configuration::new(
        url::Url::parse(&format!("{}/gateway/drs", server.url())).unwrap(),
    ));
    DrsResolver::default()
        .with_service("drs.example", transport)
        .unwrap()
}
fn object(id: &str, methods: serde_json::Value) -> serde_json::Value {
    json!({"id":id,"self_uri":"drs://drs.example/id","size":1,"created_time":"2026-01-01T00:00:00Z","checksums":[],"access_methods":methods})
}
#[test]
fn hostname_uri_decodes_exactly_once_and_rejects_unsupported_forms() {
    let uri = DrsUri::parse("drs://DRS.EXAMPLE/a%2Fb%20%25%252F%22").unwrap();
    assert_eq!(uri.authority, "drs.example");
    assert_eq!(uri.object_id, "a/b %%2F\"");
    assert_eq!(
        DrsUri::parse("drs://dg.4503:accession/with/slash"),
        Err(UriError::UnsupportedCompactIdentifier)
    );
    assert_eq!(
        DrsUri::parse("https://drs.example/id"),
        Err(UriError::UnsupportedScheme)
    );
    // A port or an IPv6 host isn't a compact identifier. Digits after a ':'
    // and before a '/' count as a port, also where they could be a compact
    // accession with a slash.
    for uri in [
        "drs://drs.example:8443/id",
        "drs://drs.example:443/id",
        "drs://dg.4503:123/456",
        "drs://[::1]:8443/id",
        "drs://[::1]:/id",
        "drs://[::1]x/id",
        "drs://[::1/id",
        "drs://[::1]",
        "drs://[2001:DB8:0:0::1]/id",
    ] {
        assert_eq!(DrsUri::parse(uri), Err(UriError::Malformed), "{uri}");
    }
    for uri in [
        "drs://dg.4503:accession",
        "drs://dg.4503:123456",
        "drs://dg.4503:abc/with/slash",
    ] {
        assert_eq!(
            DrsUri::parse(uri),
            Err(UriError::UnsupportedCompactIdentifier),
            "{uri}"
        );
    }
    // A resolver service cannot be registered for an IPv6 literal or a port.
    let transport = || {
        Transport::new(&Configuration::new(
            url::Url::parse("http://127.0.0.1:9/ga4gh/drs/v1").unwrap(),
        ))
    };
    assert_eq!(
        DrsResolver::default()
            .with_service("[::1]", transport())
            .err(),
        Some(UriError::Malformed)
    );
    assert_eq!(
        DrsResolver::default()
            .with_service("drs.example:8443", transport())
            .err(),
        Some(UriError::Malformed)
    );
    for uri in [
        "drs://drs.example",
        "drs://drs.example/",
        "drs://drs.example/a/b",
        "drs://drs.example/%ZZ",
        "drs://drs.example/%FF",
        "drs://drs.example/%2e%2e",
        "drs://user@host/id",
        "drs://host/id?query",
    ] {
        assert_eq!(DrsUri::parse(uri), Err(UriError::Malformed), "{uri}");
    }
}
#[tokio::test]
async fn explicit_gateway_mapping_direct_url_and_access_headers_do_not_download() {
    let mut server = mockito::Server::new_async().await;
    let id = "a/b %%2F\"";
    let data = format!("{}/object-store", server.url());
    let metadata=server.mock("GET","/gateway/drs/objects/a%2Fb%20%25%252F%22")
  .with_body(object(id,json!([{"type":"https","access_url":{"url":format!("{data}?signature=object-secret"),"headers":["Authorization: Bearer object-token"]}}])).to_string()).expect(1).create();
    let download = server.mock("GET", "/object-store").expect(0).create();
    let plan = match resolver(&server)
        .access_plan_once(
            "drs://drs.example/a%2Fb%20%25%252F%22",
            &AccessPreferences::default(),
            &budget(),
        )
        .await
        .unwrap()
    {
        AccessResolution::Ready(plan) => plan,
        _ => panic!("unexpected pending"),
    };
    assert_eq!(plan.object_id, id);
    assert_eq!(plan.headers["authorization"], "Bearer object-token");
    let debug = format!("{plan:?}");
    assert!(!debug.contains("object-secret"));
    assert!(!debug.contains("object-token"));
    metadata.assert();
    download.assert();
}
#[tokio::test]
async fn access_id_negotiation_and_ordered_protocol_region_preferences() {
    let mut server = mockito::Server::new_async().await;
    let metadata = server
        .mock("GET", "/gateway/drs/objects/id")
        .with_body(
            object(
                "id",
                json!([
                 {"type":"s3","region":"west","access_url":{"url":"s3://bucket/object"}},
                 {"type":"https","region":"east","access_id":"a/b"},
                 {"type":"https","region":"west","access_id":"ignored"}
                ]),
            )
            .to_string(),
        )
        .expect(1)
        .create();
    let access = server
        .mock("GET", "/gateway/drs/objects/id/access/a%2Fb")
        .with_body(r#"{"url":"https://store.example/object","headers":["X-Access: object"]}"#)
        .expect(1)
        .create();
    let preferences = AccessPreferences {
        protocols: vec![Type::Https, Type::S3],
        regions: vec!["east".into(), "west".into()],
        method_index: None,
    };
    let plan = resolver(&server)
        .access_plan("drs://drs.example/id", &preferences, &budget(), 2)
        .await
        .unwrap();
    assert_eq!(plan.region.as_deref(), Some("east"));
    assert_eq!(plan.headers["x-access"], "object");
    metadata.assert();
    access.assert();
}
#[tokio::test]
async fn pending_unavailable_unsupported_and_cancel_are_distinct() {
    let mut server = mockito::Server::new_async().await;
    let pending = server
        .mock("GET", "/gateway/drs/objects/id")
        .with_status(202)
        .with_header("retry-after", "1")
        .expect(1)
        .create();
    assert!(matches!(
        resolver(&server)
            .access_plan_once(
                "drs://drs.example/id",
                &AccessPreferences::default(),
                &budget()
            )
            .await
            .unwrap(),
        AccessResolution::Pending(_)
    ));
    pending.assert();
    pending.remove();
    for (methods, expected) in [
        (json!([{"type":"https","available":false}]), "Unavailable"),
        (
            json!([{"type":"s3","access_url":{"url":"s3://bucket/key"}}]),
            "UnsupportedMethods",
        ),
    ] {
        let request = server
            .mock("GET", "/gateway/drs/objects/id")
            .with_body(object("id", methods).to_string())
            .expect(1)
            .create();
        let error = resolver(&server)
            .access_plan_once(
                "drs://drs.example/id",
                &AccessPreferences {
                    protocols: vec![Type::Https],
                    ..Default::default()
                },
                &budget(),
            )
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<AccessChoiceError>().is_some());
        assert!(error.to_string().contains(expected));
        request.assert();
    }
    let operation = budget();
    operation.cancellation.cancel();
    let error = resolver(&server)
        .access_plan_once(
            "drs://drs.example/id",
            &AccessPreferences::default(),
            &operation,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<OperationError>(),
        Some(&OperationError::Cancelled)
    );
}
#[test]
fn uri_errors_explain_themselves_and_the_default_root_follows_the_host() {
    for (error, text) in [
        (UriError::UnsupportedScheme, "unsupported DRS URI scheme"),
        (
            UriError::UnsupportedCompactIdentifier,
            "compact DRS identifiers are not supported by this hostname resolver",
        ),
        (UriError::Malformed, "malformed hostname-based DRS URI"),
    ] {
        assert_eq!(error.to_string(), text);
    }
    // Only unreserved characters and percent escapes form an ID
    assert_eq!(
        DrsUri::parse("drs://drs.example/a+b"),
        Err(UriError::Malformed)
    );
    assert_eq!(
        DrsUri::parse("drs://DRS.Example/id")
            .unwrap()
            .default_api_root()
            .as_str(),
        "https://drs.example/ga4gh/drs/v1"
    );
}
#[tokio::test]
async fn unmapped_hosts_get_an_anonymous_client_at_the_default_root() {
    // A cancelled operation stops before any request leaves the client
    let operation = budget();
    operation.cancellation.cancel();
    let error = DrsResolver::default()
        .access_plan_once(
            "drs://unmapped.invalid/id",
            &AccessPreferences::default(),
            &operation,
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<OperationError>(),
        Some(&OperationError::Cancelled)
    );
}
#[tokio::test]
async fn method_index_overrides_the_preferences() {
    let mut server = mockito::Server::new_async().await;
    let _metadata = server
        .mock("GET", "/gateway/drs/objects/id")
        .with_body(
            object(
                "id",
                json!([
                    {"type":"https","access_url":{"url":"https://store.example/first"}},
                    {"type":"s3","region":"west","access_url":{"url":"s3://bucket/second"}},
                    {"type":"https","available":false,"access_url":{"url":"https://store.example/third"}}
                ]),
            )
            .to_string(),
        )
        .create();
    let resolver = resolver(&server);
    let preferences = |index| AccessPreferences {
        protocols: vec![Type::Https],
        method_index: Some(index),
        ..Default::default()
    };
    // The index wins over the protocol preference
    let plan = resolver
        .access_plan("drs://drs.example/id", &preferences(1), &budget(), 1)
        .await
        .unwrap();
    assert_eq!(plan.method, Type::S3);
    assert_eq!(plan.region.as_deref(), Some("west"));
    assert_eq!(plan.url.as_str(), "s3://bucket/second");
    for (index, expected) in [(2, "Unavailable"), (3, "InvalidOverride")] {
        let error = resolver
            .access_plan("drs://drs.example/id", &preferences(index), &budget(), 1)
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<AccessChoiceError>().is_some());
        assert!(error.to_string().contains(expected), "{error}");
    }
}
#[tokio::test]
async fn invalid_access_urls_and_headers_are_rejected() {
    let mut server = mockito::Server::new_async().await;
    let resolver = resolver(&server);
    for (access_url, expected) in [
        (
            json!({"url":"mailto:someone@example.org"}),
            "InvalidAccessUrl",
        ),
        (
            json!({"url":"https://store.example/data","headers":["Bad Name: value"]}),
            "InvalidAccessHeaders",
        ),
        (
            json!({"url":"https://store.example/data","headers":["X-Access: line\u{7f}break"]}),
            "InvalidAccessHeaders",
        ),
    ] {
        let request = server
            .mock("GET", "/gateway/drs/objects/id")
            .with_body(object("id", json!([{"type":"https","access_url":access_url}])).to_string())
            .expect(1)
            .create();
        let error = resolver
            .access_plan(
                "drs://drs.example/id",
                &AccessPreferences::default(),
                &budget(),
                1,
            )
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<AccessChoiceError>().is_some());
        assert!(error.to_string().contains(expected), "{error}");
        request.assert();
        request.remove();
    }
}
#[tokio::test]
async fn single_step_plans_resolve_access_ids_and_keep_them_pending() {
    let mut server = mockito::Server::new_async().await;
    let _metadata = server
        .mock("GET", "/gateway/drs/objects/id")
        .with_body(object("id", json!([{"type":"https","access_id":"a1"}])).to_string())
        .create();
    let resolver = resolver(&server);
    let pending = server
        .mock("GET", "/gateway/drs/objects/id/access/a1")
        .with_status(202)
        .with_header("retry-after", "1")
        .expect(1)
        .create();
    match resolver
        .access_plan_once(
            "drs://drs.example/id",
            &AccessPreferences::default(),
            &budget(),
        )
        .await
        .unwrap()
    {
        AccessResolution::Pending(pending) => {
            assert_eq!(pending.reference.endpoint, "objects/id/access/a1");
            assert_eq!(pending.retry_after_secs, 1);
        }
        AccessResolution::Ready(_) => panic!("pending access URL reported as ready"),
    }
    pending.assert();
    pending.remove();
    let ready = server
        .mock("GET", "/gateway/drs/objects/id/access/a1")
        .with_body(r#"{"url":"https://store.example/object"}"#)
        .expect(1)
        .create();
    match resolver
        .access_plan_once(
            "drs://drs.example/id",
            &AccessPreferences::default(),
            &budget(),
        )
        .await
        .unwrap()
    {
        AccessResolution::Ready(plan) => {
            assert_eq!(plan.url.as_str(), "https://store.example/object")
        }
        AccessResolution::Pending(_) => panic!("ready access URL reported as pending"),
    }
    ready.assert();
}
#[tokio::test]
async fn both_planners_reject_other_objects_and_methods_without_access() {
    let mut server = mockito::Server::new_async().await;
    let resolver = resolver(&server);
    for (body, expected) in [
        (
            object("other", json!([{"type":"https","access_id":"a1"}])),
            "identifies another object",
        ),
        (
            object("id", json!([{"type":"https"}])),
            "UnsupportedMethods",
        ),
    ] {
        let request = server
            .mock("GET", "/gateway/drs/objects/id")
            .with_body(body.to_string())
            .expect(2)
            .create();
        let once = resolver
            .access_plan_once(
                "drs://drs.example/id",
                &AccessPreferences::default(),
                &budget(),
            )
            .await
            .unwrap_err();
        assert!(once.to_string().contains(expected), "{once}");
        let waiting = resolver
            .access_plan(
                "drs://drs.example/id",
                &AccessPreferences::default(),
                &budget(),
                1,
            )
            .await
            .unwrap_err();
        assert!(waiting.to_string().contains(expected), "{waiting}");
        request.assert();
        request.remove();
    }
}
