use ga4gh_sdk::{
    clients::{
        drs::{
            bulk::{BulkDialect, BulkKey, BulkPolicy, BulkReport, ObjectOutcome},
            models::{BulkAccessUrl, DrsObject},
            DRS,
        },
        serviceinfo::models::Service,
    },
    utils::{configuration::Configuration, operation::Operation, transport::Transport},
};
use mockito::Matcher;
use serde_json::{json, Value};
use std::{collections::BTreeSet, time::Duration};
fn client(server: &mockito::ServerGuard, path: &str) -> DRS {
    let config = Configuration::new(url::Url::parse(&format!("{}{path}", server.url())).unwrap());
    DRS::without_discovery(Transport::new(&config), Service::default())
}
fn budget() -> Operation {
    Operation::new(Duration::from_secs(5))
}
fn key(id: &str) -> BulkKey {
    BulkKey {
        object_id: id.into(),
        access_id: None,
    }
}
fn object(id: &str) -> Value {
    json!({"id":id,"self_uri":format!("drs://example.org/{id}"),"size":1,"created_time":"2026-01-01T00:00:00Z","checksums":[]})
}
#[test]
fn debug_output_does_not_disclose_access_credentials() {
    let key = BulkKey {
        object_id: "object".into(),
        access_id: Some("SENTINEL_ACCESS_ID".into()),
    };
    let mut access = BulkAccessUrl::new("https://store.example/data?signature=SENTINEL_URL".into());
    access.headers = Some(vec!["Authorization: Bearer SENTINEL_HEADER".into()]);
    let mut report = BulkReport::default();
    report.outcomes.insert(
        key.clone(),
        ObjectOutcome {
            resolved: vec![access],
            errors: vec!["malformed access response: SENTINEL_ERROR".into()],
            ..Default::default()
        },
    );
    let debug = format!("{report:?} {key:?}");
    assert!(!debug.contains("SENTINEL"), "{debug}");
    assert!(debug.contains("resolved_count: 1"), "{debug}");
    assert!(debug.contains("error_count: 1"), "{debug}");
}
#[tokio::test]
async fn mixed_reordered_outcomes_and_duplicate_input_are_correlated_by_identity() {
    let mut server = mockito::Server::new_async().await;
    let request=server.mock("POST","/mixed/objects").match_body(Matcher::Json(json!({"bulk_object_ids":["a","b","c","d"],"passports":["test-passport"]})))
 .with_body(json!({"resolved_drs_object":[object("d"),object("a")],"unresolved_drs_objects":[{"error_code":403,"object_ids":["b"]},{"error_code":404,"object_ids":["c"]}]}).to_string()).expect(1).create();
    let report = client(&server, "/mixed")
        .get_bulk_objects(
            &["d".into(), "c".into(), "a".into(), "b".into(), "a".into()],
            &["test-passport".into()],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert!(report.outcomes[&key("a")].is_success());
    assert!(report.outcomes[&key("d")].is_success());
    assert_eq!(report.outcomes[&key("b")].failure_codes, vec![403]);
    assert_eq!(report.outcomes[&key("c")].failure_codes, vec![404]);
    assert_eq!(report.dialects, BTreeSet::from([BulkDialect::Standard]));
    request.assert();
}
#[tokio::test]
async fn access_pairs_missing_and_duplicate_identities_are_never_zipped_to_positions() {
    let mut server = mockito::Server::new_async().await;
    let a = BulkKey {
        object_id: "a".into(),
        access_id: Some("x".into()),
    };
    let b = BulkKey {
        object_id: "b".into(),
        access_id: Some("y".into()),
    };
    let request=server.mock("POST","/access/objects/access").match_body(Matcher::Json(json!({"bulk_object_access_ids":[{"bulk_object_id":"a","bulk_access_ids":["x"]},{"bulk_object_id":"b","bulk_access_ids":["y"]}]})))
 .with_body(json!({"resolved_drs_object_access_urls":[{"drs_object_id":"b","drs_access_id":"y","url":"https://store/b"},{"drs_object_id":"a","url":"https://store/ambiguous"},{"drs_object_id":"b","drs_access_id":"y","url":"https://store/duplicate"}]}).to_string()).expect(1).create();
    let report = client(&server, "/access")
        .get_bulk_access_urls(
            &[b.clone(), a.clone()],
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert_eq!(report.outcomes[&b].resolved.len(), 2);
    assert!(!report.outcomes[&b].is_success());
    assert!(report.outcomes[&a].resolved.is_empty());
    assert!(!report.outcomes[&a].errors.is_empty());
    assert_eq!(report.diagnostics.len(), 2);
    request.assert();
    let mut other = a.clone();
    other.access_id = Some("another".into());
    assert!(client(&server, "/access")
        .get_bulk_access_urls(&[a, other], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap_err()
        .to_string()
        .contains("one access ID"));
}
#[tokio::test]
async fn invalid_policy_is_rejected_even_without_ids() {
    let server = mockito::Server::new_async().await;
    let client = client(&server, "/invalid-policy");
    let invalid = BulkPolicy {
        chunk_size: 0,
        ..BulkPolicy::default()
    };
    let rejected = |result: Result<(), ga4gh_sdk::Error>, call: &str| {
        let message = result.expect_err(call).to_string();
        assert!(message.contains("bulk limits"), "{call}: {message}");
    };
    rejected(
        client
            .get_bulk_objects(&[], &[], &invalid, &budget())
            .await
            .map(drop),
        "get_bulk_objects",
    );
    rejected(
        client
            .get_bulk_objects_raw(&[], &[], &invalid, &budget())
            .await
            .map(drop),
        "get_bulk_objects_raw",
    );
    rejected(
        client
            .get_bulk_access_urls(&[], &[], &invalid, &budget())
            .await
            .map(drop),
        "get_bulk_access_urls",
    );
    rejected(
        client
            .get_bulk_access_urls_raw(&[], &[], &invalid, &budget())
            .await
            .map(drop),
        "get_bulk_access_urls_raw",
    );
    // With a valid policy and without IDs, the report is empty
    assert!(client
        .get_bulk_access_urls(&[], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap()
        .outcomes
        .is_empty());
}
#[tokio::test]
async fn empty_pending_and_auth_failure_do_not_fallback() {
    let mut server = mockito::Server::new_async().await;
    let client = client(&server, "/pending");
    assert!(client
        .get_bulk_objects(&[], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap()
        .outcomes
        .is_empty());
    let single = server.mock("GET", "/pending/objects/a").expect(0).create();
    for status in [202, 401, 403] {
        let request = server
            .mock("POST", "/pending/objects")
            .with_status(status)
            .with_header("retry-after", "1")
            .expect(1)
            .create();
        let report = client
            .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
            .await
            .unwrap();
        if status == 202 {
            assert_eq!(
                report.outcomes[&key("a")]
                    .pending
                    .as_ref()
                    .unwrap()
                    .retry_after_secs,
                1
            );
        } else {
            assert_eq!(
                report.outcomes[&key("a")].failure_codes,
                vec![status as u16]
            );
        }
        request.assert();
    }
    single.assert();
}
#[tokio::test]
async fn bounded_413_splitting_preserves_siblings_and_singleton_failure_terminates() {
    let mut server = mockito::Server::new_async().await;
    let client = client(&server, "/split");
    let both = server
        .mock("POST", "/split/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a","b"]})))
        .with_status(413)
        .expect(1)
        .create();
    let a = server
        .mock("POST", "/split/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(json!({"resolved_drs_object":[object("a")]}).to_string())
        .expect(1)
        .create();
    let b = server
        .mock("POST", "/split/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["b"]})))
        .with_status(413)
        .expect(1)
        .create();
    let report = client
        .get_bulk_objects(
            &["a".into(), "b".into()],
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert!(report.outcomes[&key("a")].is_success());
    assert_eq!(report.outcomes[&key("b")].failure_codes, vec![413]);
    both.assert();
    a.assert();
    b.assert();
}
#[tokio::test]
async fn configured_service_limit_and_concurrency_are_enforced() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let server_peak = peak.clone();
    let server = tokio::spawn(async move {
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let active = active.clone();
            let peak = server_peak.clone();
            tasks.push(tokio::spawn(async move {
    let current=active.fetch_add(1,Ordering::SeqCst)+1;peak.fetch_max(current,Ordering::SeqCst);
    let mut data=Vec::new();let header_end=loop{let mut buffer=[0;4096];let n=socket.read(&mut buffer).await.unwrap();assert!(n>0);data.extend_from_slice(&buffer[..n]);if let Some(index)=data.windows(4).position(|w|w==b"\r\n\r\n"){break index+4;}};
    let headers=String::from_utf8_lossy(&data[..header_end]);let length:usize=headers.lines().find_map(|line|line.to_ascii_lowercase().strip_prefix("content-length:").map(|s|s.trim().parse().unwrap())).unwrap();
    while data.len()<header_end+length{let mut buffer=[0;4096];let n=socket.read(&mut buffer).await.unwrap();assert!(n>0);data.extend_from_slice(&buffer[..n]);}
    let request:Value=serde_json::from_slice(&data[header_end..header_end+length]).unwrap();let ids=request["bulk_object_ids"].as_array().unwrap();assert_eq!(ids.len(),1);
    tokio::time::sleep(Duration::from_millis(50)).await;
    let body=json!({"resolved_drs_object":[object(ids[0].as_str().unwrap())]}).to_string();
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();active.fetch_sub(1,Ordering::SeqCst);
   }));
        }
        for task in tasks {
            task.await.unwrap();
        }
    });
    let config = Configuration::new(url::Url::parse(&format!("http://{address}")).unwrap());
    let client = DRS::without_discovery(Transport::new(&config), Service::default());
    let mut policy = BulkPolicy::default()
        .with_service_info(&json!({"maxBulkRequestLength":1}))
        .unwrap();
    policy.concurrency = 2;
    let report = client
        .get_bulk_objects(
            &["a".into(), "b".into(), "c".into(), "d".into()],
            &[],
            &policy,
            &budget(),
        )
        .await
        .unwrap();
    assert!(report.outcomes.values().all(|o| o.is_success()));
    assert_eq!(peak.load(Ordering::SeqCst), 2);
    server.await.unwrap();
}

#[tokio::test]
async fn credential_contexts_and_global_request_bound_are_isolated() {
    let mut server = mockito::Server::new_async().await;
    let mut a = client(&server, "/context-a");
    let mut b = client(&server, "/context-b");
    for (client, token) in [(&mut a, "Bearer context-a"), (&mut b, "Bearer context-b")] {
        let mut headers = ga4gh_sdk::reqwest::header::HeaderMap::new();
        headers.insert("authorization", token.parse().unwrap());
        let transport = Transport::with_client(
            client.configuration(),
            ga4gh_sdk::reqwest::Client::builder()
                .default_headers(headers)
                .build()
                .unwrap(),
        );
        *client = DRS::without_discovery(transport, client.discovered_service_info().clone());
    }
    let request_a = server
        .mock("POST", "/context-a/objects")
        .match_header("authorization", "Bearer context-a")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(json!({"resolved_drs_object":[object("a")]}).to_string())
        .expect(1)
        .create();
    let request_b = server
        .mock("POST", "/context-b/objects")
        .match_header("authorization", "Bearer context-b")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["b"]})))
        .with_body(json!({"resolved_drs_object":[object("b")]}).to_string())
        .expect(1)
        .create();
    let a_ids = vec!["a".into()];
    let b_ids = vec!["b".into()];
    let policy = BulkPolicy::default();
    let operation = budget();
    let (a_result, b_result) = tokio::join!(
        a.get_bulk_objects(&a_ids, &[], &policy, &operation),
        b.get_bulk_objects(&b_ids, &[], &policy, &operation)
    );
    assert!(a_result.unwrap().outcomes[&key("a")].is_success());
    assert!(b_result.unwrap().outcomes[&key("b")].is_success());
    request_a.assert();
    request_b.assert();
    let first = server
        .mock("POST", "/bound/objects")
        .with_status(413)
        .expect(1)
        .create();
    let policy = BulkPolicy {
        max_requests: 1,
        ..Default::default()
    };
    let report = client(&server, "/bound")
        .get_bulk_objects(&["a".into(), "b".into()], &[], &policy, &budget())
        .await
        .unwrap();
    assert!(report
        .outcomes
        .values()
        .all(|outcome| outcome.errors.iter().any(|e| e.contains("bound exhausted"))));
    first.assert();
}

const GEN3_FOUND: &str = "dg.C78ne/b2d9ad49-979d-4de2-9f7f-b3d054593de0";
const GEN3_MISSING: &str = "dg.C78ne/00000000-0000-0000-0000-000000000000";
/// Recorded from `POST https://caninedc.org/ga4gh/drs/v1/objects` on 2026-11-10.
const GEN3_REPLY: &str = include_str!("../../tests/fixtures/drs/gen3-caninedc-bulk-objects.json");

#[tokio::test]
async fn gen3_reply_is_correlated_without_fabricating_created_time() {
    let mut server = mockito::Server::new_async().await;
    // Request IDs are sent in sorted order, not in input order.
    let request = server
        .mock("POST", "/gen3/objects")
        .match_body(Matcher::Json(
            json!({"bulk_object_ids":[GEN3_MISSING, GEN3_FOUND]}),
        ))
        .with_body(GEN3_REPLY)
        .expect(2)
        .create();
    let ids = [GEN3_FOUND.to_string(), GEN3_MISSING.to_string()];
    let client = client(&server, "/gen3");

    let typed = client
        .get_bulk_objects(&ids, &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert_eq!(typed.dialects, BTreeSet::from([BulkDialect::Gen3]));
    assert_eq!(typed.outcomes[&key(GEN3_MISSING)].failure_codes, vec![404]);
    let found = &typed.outcomes[&key(GEN3_FOUND)];
    assert!(found.resolved.is_empty());
    assert_eq!(found.errors.len(), 1, "{:?}", found.errors);
    assert!(
        found.errors[0].contains("created_time is null"),
        "{:?}",
        found.errors
    );
    assert!(typed.diagnostics.is_empty(), "{:?}", typed.diagnostics);

    let raw = client
        .get_bulk_objects_raw(&ids, &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert_eq!(raw.dialects, BTreeSet::from([BulkDialect::Gen3]));
    let found = &raw.outcomes[&key(GEN3_FOUND)];
    assert!(found.is_success());
    assert_eq!(found.resolved[0]["id"], GEN3_FOUND);
    assert!(found.resolved[0]["created_time"].is_null());
    assert_eq!(raw.outcomes[&key(GEN3_MISSING)].failure_codes, vec![404]);
    request.assert();
}

#[tokio::test]
async fn standard_field_wins_when_a_reply_uses_both_dialects() {
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("POST", "/both/objects")
        .with_body(
            json!({"resolved_drs_object":[object("a")],"resolved_drs_objects":[object("b")]})
                .to_string(),
        )
        .expect(1)
        .create();
    let report = client(&server, "/both")
        .get_bulk_objects(
            &["a".into(), "b".into()],
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert_eq!(report.dialects, BTreeSet::from([BulkDialect::Standard]));
    assert!(report.outcomes[&key("a")].is_success());
    assert!(report.outcomes[&key("b")].resolved.is_empty());
    assert!(report.outcomes[&key("b")].errors[0].contains("omitted"));
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.contains("resolved_drs_objects") && d.contains("ignored")));
    request.assert();
}

#[tokio::test]
async fn dialects_from_separate_chunks_are_reported_together() {
    let mut server = mockito::Server::new_async().await;
    let standard = server
        .mock("POST", "/mixed-dialects/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(json!({"resolved_drs_object":[object("a")]}).to_string())
        .expect(1)
        .create();
    let gen3 = server
        .mock("POST", "/mixed-dialects/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["b"]})))
        .with_body(json!({"resolved_drs_objects":[object("b")]}).to_string())
        .expect(1)
        .create();
    let policy = BulkPolicy {
        chunk_size: 1,
        concurrency: 2,
        ..Default::default()
    };
    let report = client(&server, "/mixed-dialects")
        .get_bulk_objects(&["b".into(), "a".into()], &[], &policy, &budget())
        .await
        .unwrap();

    assert!(report.outcomes[&key("a")].is_success());
    assert!(report.outcomes[&key("b")].is_success());
    assert_eq!(
        report.dialects,
        BTreeSet::from([BulkDialect::Standard, BulkDialect::Gen3])
    );
    standard.assert();
    gen3.assert();
}

#[tokio::test]
async fn starter_kit_experimental_reply_is_not_interpreted() {
    let mut server = mockito::Server::new_async().await;
    // Starter Kit DRS `1.3.0experimental` (develop 2b76a63) reads `selection`
    // and reports failures in an `unresolved_drs_object` map.
    let request = server
        .mock("POST", "/starter-kit/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(
            json!({"summary":{"requested":0,"resolved":0,"unresolved":0},
                "resolved_drs_object":[],"unresolved_drs_object":{"a":404}})
            .to_string(),
        )
        .expect(1)
        .create();
    let report = client(&server, "/starter-kit")
        .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    let outcome = &report.outcomes[&key("a")];
    assert!(outcome.failure_codes.is_empty());
    assert!(
        outcome.errors[0].contains("omitted"),
        "{:?}",
        outcome.errors
    );
    assert!(report.dialects.is_empty());
    assert!(report
        .diagnostics
        .iter()
        .any(|d| d.contains("unresolved_drs_object,")));
    request.assert();

    // Its bulk access reply is a bare array without object or access IDs.
    let access = BulkKey {
        object_id: "a".into(),
        access_id: Some("x".into()),
    };
    let request = server
        .mock("POST", "/starter-kit-access/objects/access")
        .with_body(json!([{"url":"https://store.example/a"}]).to_string())
        .expect(1)
        .create();
    let report = client(&server, "/starter-kit-access")
        .get_bulk_access_urls(
            std::slice::from_ref(&access),
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert!(report.outcomes[&access].resolved.is_empty());
    assert!(report.outcomes[&access].errors[0].contains("must be an object"));
    assert!(report.dialects.is_empty());
    request.assert();
}

#[tokio::test]
async fn gen3_field_is_read_only_for_object_lookups_with_an_array() {
    let mut server = mockito::Server::new_async().await;
    let access = BulkKey {
        object_id: "a".into(),
        access_id: Some("x".into()),
    };
    let request = server
        .mock("POST", "/gen3-access/objects/access")
        .with_body(
            json!({"resolved_drs_objects":[
                {"drs_object_id":"a","drs_access_id":"x","url":"https://store/a"}
            ]})
            .to_string(),
        )
        .expect(1)
        .create();
    let report = client(&server, "/gen3-access")
        .get_bulk_access_urls(
            std::slice::from_ref(&access),
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert!(report.outcomes[&access].resolved.is_empty());
    assert!(report.outcomes[&access].errors[0].contains("omitted"));
    assert!(report.dialects.is_empty());
    request.assert();

    let request = server
        .mock("POST", "/gen3-shape/objects")
        .with_body(json!({"resolved_drs_objects":{"id":"a"}}).to_string())
        .expect(1)
        .create();
    let report = client(&server, "/gen3-shape")
        .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert!(report.dialects.is_empty());
    assert!(report.diagnostics.iter().any(|d| d.contains("array")));
    request.assert();

    let request = server
        .mock("POST", "/gen3-null/objects")
        .with_body(
            json!({"resolved_drs_object":null,"resolved_drs_objects":[object("a")]}).to_string(),
        )
        .expect(1)
        .create();
    let report = client(&server, "/gen3-null")
        .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert!(report.outcomes[&key("a")].is_success());
    assert_eq!(report.dialects, BTreeSet::from([BulkDialect::Gen3]));
    request.assert();
}

#[tokio::test]
async fn raw_bulk_reports_keep_fields_unknown_to_the_models() {
    let mut server = mockito::Server::new_async().await;
    let mut extended = object("a");
    extended["extension"] = json!({"kept": true});
    let objects = server
        .mock("POST", "/raw/objects")
        .with_body(json!({"resolved_drs_object": [extended.clone()]}).to_string())
        .expect(1)
        .create();
    let access = server
        .mock("POST", "/raw/objects/access")
        .with_body(
            json!({"resolved_drs_object_access_urls": [{
                "drs_object_id": "a", "drs_access_id": "x",
                "url": "https://store.example/a", "extension": 1
            }]})
            .to_string(),
        )
        .expect(1)
        .create();
    let client = client(&server, "/raw");
    let report = client
        .get_bulk_objects_raw(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert_eq!(report.outcomes[&key("a")].resolved, vec![extended]);
    let pair = BulkKey {
        object_id: "a".into(),
        access_id: Some("x".into()),
    };
    let report = client
        .get_bulk_access_urls_raw(
            std::slice::from_ref(&pair),
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    assert_eq!(report.outcomes[&pair].resolved[0]["extension"], json!(1));
    assert!(client
        .get_bulk_access_urls_raw(&[key("a")], &[], &BulkPolicy::default(), &budget())
        .await
        .is_err());
    objects.assert();
    access.assert();
}
#[test]
fn service_info_without_a_bulk_limit_keeps_the_policy() {
    let policy = BulkPolicy::default()
        .with_service_info(&json!({"id":"drs"}))
        .unwrap();
    assert_eq!(policy.advertised_limit, None);
}
#[tokio::test]
async fn empty_ids_are_rejected_before_any_request() {
    let mut server = mockito::Server::new_async().await;
    let requests = server
        .mock("POST", Matcher::Any)
        .expect(0)
        .create_async()
        .await;
    let client = client(&server, "/empty-ids");
    let error = client
        .get_bulk_objects(&["".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "bulk IDs must be nonempty");
    let empty_access = BulkKey {
        object_id: "a".into(),
        access_id: Some(String::new()),
    };
    let error = client
        .get_bulk_access_urls(&[empty_access], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), "bulk IDs must be nonempty");
    requests.assert_async().await;
}
#[tokio::test]
async fn invalid_unrequested_and_conflicting_entries_are_reported() {
    let mut server = mockito::Server::new_async().await;
    let mut invalid = object("a");
    invalid.as_object_mut().unwrap().remove("size");
    let reply = json!({
        "resolved_drs_object": [invalid, object("c"), object("unrequested")],
        "unresolved_drs_objects": [
            {"error_code": 404, "object_ids": ["b", "unknown"]},
            {"error_code": 410, "object_ids": ["b"]},
            {"error_code": 500, "object_ids": ["c"]},
            {"error_code": 99, "object_ids": ["d"]},
            {"object_ids": ["d"]},
            {"error_code": 404}
        ]
    });
    let request = server
        .mock("POST", "/conflicts/objects")
        .with_body(reply.to_string())
        .expect(1)
        .create();
    let report = client(&server, "/conflicts")
        .get_bulk_objects(
            &["a".into(), "b".into(), "c".into(), "d".into()],
            &[],
            &BulkPolicy::default(),
            &budget(),
        )
        .await
        .unwrap();
    request.assert();
    // A schema error without a null created_time gets no hint about it
    let a = &report.outcomes[&key("a")];
    assert!(a.resolved.is_empty());
    assert_eq!(a.errors.len(), 1);
    assert!(a.errors[0].starts_with("invalid resolved entry"), "{a:?}");
    assert!(!a.errors[0].contains("created_time"));
    let b = &report.outcomes[&key("b")];
    assert_eq!(b.failure_codes, vec![404, 410]);
    assert_eq!(b.errors, vec!["duplicate unresolved identity"]);
    let c = &report.outcomes[&key("c")];
    assert_eq!(c.resolved.len(), 1);
    assert_eq!(c.failure_codes, vec![500]);
    assert_eq!(c.errors, vec!["identity is both resolved and unresolved"]);
    assert_eq!(
        report.outcomes[&key("d")].errors,
        vec!["response omitted requested identity"]
    );
    let mut diagnostics = report.diagnostics.clone();
    diagnostics.sort();
    assert_eq!(
        diagnostics,
        vec![
            "duplicate unresolved identity",
            "invalid grouped unresolved entry",
            "invalid grouped unresolved entry",
            "invalid grouped unresolved entry",
            "response contained an unrequested resolved identity",
            "unresolved entry has missing/unrequested/ambiguous identity",
        ]
    );
}
#[tokio::test]
async fn unresolved_entries_must_be_an_array() {
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("POST", "/unresolved/objects")
        .with_body(
            json!({"resolved_drs_object":[],"unresolved_drs_objects":{"404":["a"]}}).to_string(),
        )
        .expect(1)
        .create();
    let report = client(&server, "/unresolved")
        .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    request.assert();
    assert_eq!(
        report.diagnostics,
        vec!["unresolved entries must be an array"]
    );
    assert_eq!(
        report.outcomes[&key("a")].errors,
        vec!["response omitted requested identity"]
    );
}
/// A policy with `max_attempts` attempts, `delay` apart.
fn retrying(max_attempts: usize, delay: Duration) -> BulkPolicy {
    let mut policy = BulkPolicy::default();
    policy.retry.max_attempts = max_attempts;
    policy.retry.initial_delay = delay;
    policy.retry.max_delay = delay;
    policy
}
/// The errors of the outcome of `id`, the only requested object.
fn errors_of(report: &BulkReport<DrsObject>, id: &str) -> Vec<String> {
    assert_eq!(report.outcomes.len(), 1);
    report.outcomes[&key(id)].errors.clone()
}
/// A DRS client at a loopback address that refuses connections, and the
/// socket that holds its port: bound but not listening, so connections are
/// refused and no other server can take the port while the socket lives.
fn unreachable_client() -> (DRS, tokio::net::TcpSocket) {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind("127.0.0.1:0".parse().unwrap()).unwrap();
    let address = socket.local_addr().unwrap();
    let config = Configuration::new(url::Url::parse(&format!("http://{address}/drs")).unwrap());
    (
        DRS::without_discovery(Transport::new(&config), Service::default()),
        socket,
    )
}
#[tokio::test]
async fn network_errors_are_retried_within_the_operation() {
    let ids = ["a".to_string()];
    // Every attempt fails; each one connects again and the last error is
    // reported. The server closes each connection without answering.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = accepted.clone();
    let server = tokio::spawn(async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            drop(socket);
        }
    });
    let config = Configuration::new(url::Url::parse(&format!("http://{address}/drs")).unwrap());
    let report = DRS::without_discovery(Transport::new(&config), Service::default())
        .get_bulk_objects(&ids, &[], &retrying(2, Duration::from_millis(1)), &budget())
        .await
        .unwrap();
    server.abort();
    let errors = errors_of(&report, "a");
    assert_eq!(errors.len(), 1);
    assert!(!errors[0].contains("deadline"), "{errors:?}");
    assert_eq!(accepted.load(std::sync::atomic::Ordering::SeqCst), 2);
    let (unreachable, _port) = unreachable_client();
    // The deadline passes while waiting for the retry
    let report = unreachable
        .get_bulk_objects(
            &ids,
            &[],
            &retrying(2, Duration::from_secs(10)),
            &Operation::new(Duration::from_millis(300)),
        )
        .await
        .unwrap();
    assert_eq!(errors_of(&report, "a"), vec!["operation deadline exceeded"]);
    // An ended operation sends no request
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("POST", "/ended/objects")
        .expect(0)
        .create_async()
        .await;
    let operation = budget();
    operation.cancellation.cancel();
    let report = client(&server, "/ended")
        .get_bulk_objects(&ids, &[], &BulkPolicy::default(), &operation)
        .await
        .unwrap();
    assert_eq!(errors_of(&report, "a"), vec!["operation canceled locally"]);
    request.assert_async().await;
}
#[tokio::test]
async fn busy_answers_are_retried() {
    let mut server = mockito::Server::new_async().await;
    for (status, retry_after) in [(503, Some("0")), (429, None)] {
        let mut busy = server.mock("POST", "/busy/objects").with_status(status);
        if let Some(value) = retry_after {
            busy = busy.with_header("retry-after", value);
        }
        let busy = busy.expect(1).create();
        let ready = server
            .mock("POST", "/busy/objects")
            .with_body(json!({"resolved_drs_object":[object("a")]}).to_string())
            .expect(1)
            .create();
        let report = client(&server, "/busy")
            .get_bulk_objects(
                &["a".into()],
                &[],
                &retrying(2, Duration::from_millis(1)),
                &budget(),
            )
            .await
            .unwrap();
        assert!(report.outcomes[&key("a")].is_success(), "{status}");
        busy.assert();
        ready.assert();
        busy.remove();
        ready.remove();
    }
}
#[tokio::test]
async fn invalid_or_too_long_busy_delays_end_the_request() {
    let mut server = mockito::Server::new_async().await;
    for (retry_after, budget, expected) in [
        ("soon", Duration::from_secs(5), "invalid bulk Retry-After"),
        ("10", Duration::from_secs(2), "operation deadline exceeded"),
    ] {
        let busy = server
            .mock("POST", "/busy-delay/objects")
            .with_status(503)
            .with_header("retry-after", retry_after)
            .expect(1)
            .create();
        let report = client(&server, "/busy-delay")
            .get_bulk_objects(
                &["a".into()],
                &[],
                &retrying(2, Duration::from_millis(1)),
                &Operation::new(budget),
            )
            .await
            .unwrap();
        assert_eq!(errors_of(&report, "a"), vec![expected]);
        busy.assert();
        busy.remove();
    }
}
#[tokio::test]
async fn unreadable_bodies_become_errors_of_their_keys() {
    let mut server = mockito::Server::new_async().await;
    let mut small = BulkPolicy::default();
    small.limits.success = 8;
    let cases = [
        (
            server
                .mock("POST", "/pending-delay/objects")
                .with_status(202)
                .with_header("retry-after", "soon"),
            "/pending-delay",
            BulkPolicy::default(),
            "DRS Retry-After must be integral seconds",
        ),
        (
            server
                .mock("POST", "/not-json/objects")
                .with_body("not JSON"),
            "/not-json",
            BulkPolicy::default(),
            "invalid bulk response JSON",
        ),
        (
            server
                .mock("POST", "/too-large/objects")
                .with_body(json!({"resolved_drs_object":[object("a")]}).to_string()),
            "/too-large",
            small,
            "HTTP 200 OK response exceeds body limit of 8 bytes",
        ),
    ];
    for (mock, path, policy, expected) in cases {
        let mock = mock.expect(1).create();
        let report = client(&server, path)
            .get_bulk_objects(&["a".into()], &[], &policy, &budget())
            .await
            .unwrap();
        let errors = errors_of(&report, "a");
        assert_eq!(errors.len(), 1, "{path}");
        assert!(errors[0].contains(expected), "{path}: {errors:?}");
        mock.assert();
    }
}
#[tokio::test]
async fn the_deadline_also_bounds_reading_the_body() {
    let mut server = mockito::Server::new_async().await;
    let slow = server
        .mock("POST", "/slow/objects")
        .with_chunked_body(|writer| {
            writer.write_all(b"{")?;
            writer.flush()?;
            std::thread::sleep(Duration::from_secs(4));
            writer.write_all(b"}")
        })
        .expect(1)
        .create();
    let report = client(&server, "/slow")
        .get_bulk_objects(
            &["a".into()],
            &[],
            &BulkPolicy::default(),
            &Operation::new(Duration::from_secs(2)),
        )
        .await
        .unwrap();
    assert_eq!(errors_of(&report, "a"), vec!["operation deadline exceeded"]);
    slow.assert();
}
