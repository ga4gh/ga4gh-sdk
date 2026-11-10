use ga4gh_sdk::{
    clients::{
        drs::{
            bulk::{BulkDialect, BulkKey, BulkPolicy, BulkReport, ObjectOutcome},
            models::BulkAccessUrl,
            DRS,
        },
        serviceinfo::models::Service,
    },
    utils::{configuration::Configuration, operation::Operation, transport::Transport},
};
use mockito::{mock, server_url, Matcher};
use serde_json::{json, Value};
use std::{collections::BTreeSet, time::Duration};
fn client(path: &str) -> DRS {
    let config = Configuration::new(url::Url::parse(&format!("{}{path}", server_url())).unwrap());
    DRS {
        transport: Transport::new(&config),
        config,
        service: Ok(Service::default()),
    }
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
    let request=mock("POST","/mixed/objects").match_body(Matcher::Json(json!({"bulk_object_ids":["a","b","c","d"],"passports":["test-passport"]})))
 .with_body(json!({"resolved_drs_object":[object("d"),object("a")],"unresolved_drs_objects":[{"error_code":403,"object_ids":["b"]},{"error_code":404,"object_ids":["c"]}]}).to_string()).expect(1).create();
    let report = client("/mixed")
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
    let a = BulkKey {
        object_id: "a".into(),
        access_id: Some("x".into()),
    };
    let b = BulkKey {
        object_id: "b".into(),
        access_id: Some("y".into()),
    };
    let request=mock("POST","/access/objects/access").match_body(Matcher::Json(json!({"bulk_object_access_ids":[{"bulk_object_id":"a","bulk_access_ids":["x"]},{"bulk_object_id":"b","bulk_access_ids":["y"]}]})))
 .with_body(json!({"resolved_drs_object_access_urls":[{"drs_object_id":"b","drs_access_id":"y","url":"https://store/b"},{"drs_object_id":"a","url":"https://store/ambiguous"},{"drs_object_id":"b","drs_access_id":"y","url":"https://store/duplicate"}]}).to_string()).expect(1).create();
    let report = client("/access")
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
    assert!(client("/access")
        .get_bulk_access_urls(&[a, other], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap_err()
        .to_string()
        .contains("one access ID"));
}
#[tokio::test]
async fn empty_pending_and_auth_failure_do_not_fallback() {
    let client = client("/pending");
    assert!(client
        .get_bulk_objects(&[], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap()
        .outcomes
        .is_empty());
    let single = mock("GET", "/pending/objects/a").expect(0).create();
    for status in [202, 401, 403] {
        let request = mock("POST", "/pending/objects")
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
    let client = client("/split");
    let both = mock("POST", "/split/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a","b"]})))
        .with_status(413)
        .expect(1)
        .create();
    let a = mock("POST", "/split/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(json!({"resolved_drs_object":[object("a")]}).to_string())
        .expect(1)
        .create();
    let b = mock("POST", "/split/objects")
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
    let client = DRS {
        transport: Transport::new(&config),
        config,
        service: Ok(Service::default()),
    };
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
    let mut a = client("/context-a");
    let mut b = client("/context-b");
    for (client, token) in [(&mut a, "Bearer context-a"), (&mut b, "Bearer context-b")] {
        let mut headers = ga4gh_sdk::reqwest::header::HeaderMap::new();
        headers.insert("authorization", token.parse().unwrap());
        client.transport = Transport::with_client(
            &client.config,
            ga4gh_sdk::reqwest::Client::builder()
                .default_headers(headers)
                .build()
                .unwrap(),
        );
    }
    let request_a = mock("POST", "/context-a/objects")
        .match_header("authorization", "Bearer context-a")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(json!({"resolved_drs_object":[object("a")]}).to_string())
        .expect(1)
        .create();
    let request_b = mock("POST", "/context-b/objects")
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
    let first = mock("POST", "/bound/objects")
        .with_status(413)
        .expect(1)
        .create();
    let policy = BulkPolicy {
        max_requests: 1,
        ..Default::default()
    };
    let report = client("/bound")
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
    // Request IDs are sent in sorted order, not in input order.
    let request = mock("POST", "/gen3/objects")
        .match_body(Matcher::Json(
            json!({"bulk_object_ids":[GEN3_MISSING, GEN3_FOUND]}),
        ))
        .with_body(GEN3_REPLY)
        .expect(2)
        .create();
    let ids = [GEN3_FOUND.to_string(), GEN3_MISSING.to_string()];
    let client = client("/gen3");

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
    let request = mock("POST", "/both/objects")
        .with_body(
            json!({"resolved_drs_object":[object("a")],"resolved_drs_objects":[object("b")]})
                .to_string(),
        )
        .expect(1)
        .create();
    let report = client("/both")
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
async fn starter_kit_experimental_reply_is_not_interpreted() {
    // Starter Kit DRS `1.3.0experimental` (develop 2b76a63) reads `selection`
    // and reports failures in an `unresolved_drs_object` map.
    let request = mock("POST", "/starter-kit/objects")
        .match_body(Matcher::Json(json!({"bulk_object_ids":["a"]})))
        .with_body(
            json!({"summary":{"requested":0,"resolved":0,"unresolved":0},
                "resolved_drs_object":[],"unresolved_drs_object":{"a":404}})
            .to_string(),
        )
        .expect(1)
        .create();
    let report = client("/starter-kit")
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
    let request = mock("POST", "/starter-kit-access/objects/access")
        .with_body(json!([{"url":"https://store.example/a"}]).to_string())
        .expect(1)
        .create();
    let report = client("/starter-kit-access")
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
    let access = BulkKey {
        object_id: "a".into(),
        access_id: Some("x".into()),
    };
    let request = mock("POST", "/gen3-access/objects/access")
        .with_body(
            json!({"resolved_drs_objects":[
                {"drs_object_id":"a","drs_access_id":"x","url":"https://store/a"}
            ]})
            .to_string(),
        )
        .expect(1)
        .create();
    let report = client("/gen3-access")
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

    let request = mock("POST", "/gen3-shape/objects")
        .with_body(json!({"resolved_drs_objects":{"id":"a"}}).to_string())
        .expect(1)
        .create();
    let report = client("/gen3-shape")
        .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert!(report.dialects.is_empty());
    assert!(report.diagnostics.iter().any(|d| d.contains("array")));
    request.assert();

    let request = mock("POST", "/gen3-null/objects")
        .with_body(
            json!({"resolved_drs_object":null,"resolved_drs_objects":[object("a")]}).to_string(),
        )
        .expect(1)
        .create();
    let report = client("/gen3-null")
        .get_bulk_objects(&["a".into()], &[], &BulkPolicy::default(), &budget())
        .await
        .unwrap();
    assert!(report.outcomes[&key("a")].is_success());
    assert_eq!(report.dialects, BTreeSet::from([BulkDialect::Gen3]));
    request.assert();
}
