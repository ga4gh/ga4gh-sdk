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
use mockito::{mock, server_url};
use serde_json::json;
use std::time::Duration;
fn budget() -> Operation {
    Operation::new(Duration::from_secs(3))
}
fn resolver() -> DrsResolver {
    let transport = Transport::new(&Configuration::new(
        url::Url::parse(&format!("{}/gateway/drs", server_url())).unwrap(),
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
    let id = "a/b %%2F\"";
    let data = format!("{}/object-store", server_url());
    let metadata=mock("GET","/gateway/drs/objects/a%2Fb%20%25%252F%22")
  .with_body(object(id,json!([{"type":"https","access_url":{"url":format!("{data}?signature=object-secret"),"headers":["Authorization: Bearer object-token"]}}])).to_string()).expect(1).create();
    let download = mock("GET", "/object-store").expect(0).create();
    let plan = match resolver()
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
    let metadata = mock("GET", "/gateway/drs/objects/id")
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
    let access = mock("GET", "/gateway/drs/objects/id/access/a%2Fb")
        .with_body(r#"{"url":"https://store.example/object","headers":["X-Access: object"]}"#)
        .expect(1)
        .create();
    let preferences = AccessPreferences {
        protocols: vec![Type::Https, Type::S3],
        regions: vec!["east".into(), "west".into()],
        method_index: None,
    };
    let plan = resolver()
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
    let pending = mock("GET", "/gateway/drs/objects/id")
        .with_status(202)
        .with_header("retry-after", "1")
        .expect(1)
        .create();
    assert!(matches!(
        resolver()
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
    drop(pending);
    for (methods, expected) in [
        (json!([{"type":"https","available":false}]), "Unavailable"),
        (
            json!([{"type":"s3","access_url":{"url":"s3://bucket/key"}}]),
            "UnsupportedMethods",
        ),
    ] {
        let request = mock("GET", "/gateway/drs/objects/id")
            .with_body(object("id", methods).to_string())
            .expect(1)
            .create();
        let error = resolver()
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
    let error = resolver()
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
