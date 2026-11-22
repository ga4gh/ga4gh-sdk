use ga4gh_sdk::{
    clients::{
        pagination::{PageCheckpoint, Pager, PagingOptions},
        serviceinfo::models::Service,
        wait::ServiceKind,
        wes::WES,
    },
    utils::{configuration::Configuration, transport::Transport},
};
use mockito::Matcher;
use serde_json::json;
use std::time::Duration;
fn transport(server: &mockito::ServerGuard, path: &str) -> Transport {
    Transport::new(&Configuration::new(
        url::Url::parse(&format!("{}{path}", server.url())).unwrap(),
    ))
}
fn options() -> PagingOptions {
    PagingOptions::new(Duration::from_secs(10))
}
#[tokio::test]
async fn pages_preserve_filters_and_opaque_tokens_across_empty_intermediate_pages() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/three/tasks")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("name_prefix".into(), "sample".into()),
            Matcher::UrlEncoded("view".into(), "FULL".into()),
        ]))
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"a+/ %"}"#)
        .expect(1)
        .create();
    let second = server
        .mock("GET", "/three/tasks")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("name_prefix".into(), "sample".into()),
            Matcher::UrlEncoded("view".into(), "FULL".into()),
            Matcher::UrlEncoded("page_token".into(), "a+/ %".into()),
        ]))
        .with_body(r#"{"tasks":[],"next_page_token":"next"}"#)
        .expect(1)
        .create();
    let third = server
        .mock("GET", "/three/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "next".into()))
        .with_body(r#"{"tasks":[{"id":"two"}],"next_page_token":""}"#)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/three"),
        ServiceKind::Tes,
        json!({"name_prefix":"sample","view":"FULL"}),
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["id"], "one");
    assert_eq!(pager.next_item().await.unwrap().unwrap()["id"], "two");
    assert!(pager.next_item().await.unwrap().is_none());
    assert!(pager.checkpoint().finished);
    first.assert();
    second.assert();
    third.assert();
}
#[tokio::test]
async fn item_consumption_is_lazy_and_checkpoints_replay_partial_pages() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/lazy/tasks")
        .with_body(r#"{"tasks":[{"id":"one"},{"id":"two"}],"next_page_token":"later"}"#)
        .expect(1)
        .create();
    let next = server
        .mock("GET", "/lazy/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "later".into()))
        .expect(0)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/lazy"),
        ServiceKind::Tes,
        json!({}),
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["id"], "one");
    assert_eq!(pager.checkpoint().next_token, None);
    drop(pager);
    first.assert();
    next.assert();
}
#[tokio::test]
async fn cycles_fail_and_later_request_failure_retains_continuation() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/cycle/tasks")
        .with_body(r#"{"tasks":[],"next_page_token":"same"}"#)
        .expect(1)
        .create();
    let second = server
        .mock("GET", "/cycle/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "same".into()))
        .with_body(r#"{"tasks":[],"next_page_token":"same"}"#)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/cycle"),
        ServiceKind::Tes,
        json!({}),
        options(),
    )
    .unwrap();
    pager.next_page().await.unwrap();
    let error = pager.next_page().await.unwrap_err();
    assert!(error.reason.contains("cycle"));
    assert_eq!(error.checkpoint.next_token.as_deref(), Some("same"));
    first.assert();
    second.assert();
    let first = server
        .mock("GET", "/failure/tasks")
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"retry"}"#)
        .expect(1)
        .create();
    let second = server
        .mock("GET", "/failure/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "retry".into()))
        .with_status(503)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/failure"),
        ServiceKind::Tes,
        json!({}),
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_page().await.unwrap().unwrap().items.len(), 1);
    assert_eq!(
        pager
            .next_page()
            .await
            .unwrap_err()
            .checkpoint
            .next_token
            .as_deref(),
        Some("retry")
    );
    first.assert();
    second.assert();
}
#[tokio::test]
async fn resume_checks_identity_and_limits_retain_a_replay_point() {
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("GET", "/resume/tasks")
        .match_query(Matcher::UrlEncoded("view".into(), "FULL".into()))
        .with_body(r#"{"tasks":[{"id":"one"},{"id":"two"}],"next_page_token":"later"}"#)
        .expect(1)
        .create();
    let transport = transport(&server, "/resume");
    let mut settings = options();
    settings.max_items = Some(1);
    let mut pager = Pager::new(
        &transport,
        ServiceKind::Tes,
        json!({"view":"FULL"}),
        settings,
    )
    .unwrap();
    let page = pager.next_page().await.unwrap().unwrap();
    assert!(page.truncated);
    assert_eq!(page.items.len(), 1);
    let checkpoint = pager.checkpoint();
    assert!(checkpoint.next_token.is_none());
    assert!(!checkpoint.finished);
    assert!(pager.next_page().await.unwrap().is_none());
    request.assert();
    assert!(Pager::resume(
        &transport,
        ServiceKind::Wes,
        json!({"view":"FULL"}),
        checkpoint.clone(),
        options()
    )
    .is_err());
    assert!(Pager::resume(
        &transport,
        ServiceKind::Tes,
        json!({"view":"MINIMAL"}),
        checkpoint.clone(),
        options()
    )
    .is_err());
    let resumed = server
        .mock("GET", "/resume/tasks")
        .match_query(Matcher::UrlEncoded("view".into(), "FULL".into()))
        .with_body(r#"{"tasks":[{"id":"one"},{"id":"two"}]}"#)
        .expect(1)
        .create();
    let mut pager = Pager::resume(
        &transport,
        ServiceKind::Tes,
        json!({"view":"FULL"}),
        checkpoint,
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_page().await.unwrap().unwrap().items.len(), 2);
    resumed.assert();
}

#[tokio::test]
async fn wes_optional_empty_pages_and_legacy_arrays_remain_lazy() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/wes/runs")
        .with_body(r#"{"next_page_token":"a+b&c"}"#)
        .expect(1)
        .create();
    let second = server
        .mock("GET", "/wes/runs")
        .match_query(Matcher::UrlEncoded("page_token".into(), "a+b&c".into()))
        .with_body(r#"{"runs":[{"run_id":"run-1","state":"RUNNING"}],"next_page_token":""}"#)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/wes"),
        ServiceKind::Wes,
        json!({}),
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["run_id"], "run-1");
    assert!(pager.next_item().await.unwrap().is_none());
    first.assert();
    second.assert();
    let legacy = server
        .mock("GET", "/legacy/runs")
        .with_body(r#"[{"run_id":"run-2"}]"#)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/legacy"),
        ServiceKind::Wes,
        json!({}),
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["run_id"], "run-2");
    assert!(pager.next_item().await.unwrap().is_none());
    legacy.assert();
}

#[test]
fn invalid_limits_and_page_tokens_fail_before_a_request() {
    let transport = Transport::new(&Configuration::new(
        url::Url::parse("https://example.org/tes/v1").unwrap(),
    ));
    let mut settings = options();
    settings.max_requests = 0;
    let error = Pager::new(&transport, ServiceKind::Tes, json!({}), settings).unwrap_err();
    assert_eq!(
        error.to_string(),
        "pagination max_requests must be positive"
    );
    let error = Pager::new(
        &transport,
        ServiceKind::Tes,
        json!({"page_token": 7}),
        options(),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "page_token must be a string");
}

#[tokio::test]
async fn malformed_pages_fail_with_the_checkpoint_of_the_page() {
    let mut server = mockito::Server::new_async().await;
    for (root, body, reason) in [
        (
            "/items",
            r#"{"tasks":"one"}"#,
            "page is missing its items array",
        ),
        (
            "/token",
            r#"{"tasks":[],"next_page_token":7}"#,
            "next_page_token must be a string",
        ),
    ] {
        let page = server
            .mock("GET", format!("{root}/tasks").as_str())
            .with_body(body)
            .expect(1)
            .create();
        let mut pager = Pager::new(
            &transport(&server, root),
            ServiceKind::Tes,
            json!({}),
            options(),
        )
        .unwrap();
        let error = pager.next_page().await.unwrap_err();
        assert_eq!(error.reason, reason);
        assert!(!error.checkpoint.finished);
        assert_eq!(error.checkpoint.next_token, None);
        page.assert();
    }
}

#[tokio::test]
async fn cancelled_operations_fail_the_page() {
    let mut server = mockito::Server::new_async().await;
    let page = server.mock("GET", "/cancelled/tasks").expect(0).create();
    let settings = options();
    settings.operation.cancellation.cancel();
    let mut pager = Pager::new(
        &transport(&server, "/cancelled"),
        ServiceKind::Tes,
        json!({}),
        settings,
    )
    .unwrap();
    let error = pager.next_page().await.unwrap_err();
    assert_eq!(error.reason, "operation canceled locally");
    page.assert();
}

#[tokio::test]
async fn pages_wait_until_the_items_of_the_last_page_are_used_up() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/partial/tasks")
        .with_body(r#"{"tasks":[{"id":"one"},{"id":"two"}],"next_page_token":"later"}"#)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport(&server, "/partial"),
        ServiceKind::Tes,
        json!({}),
        options(),
    )
    .unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["id"], "one");
    let error = pager.next_page().await.unwrap_err();
    assert_eq!(
        error.reason,
        "cannot request a page while an item page is partially consumed"
    );
    // The checkpoint replays the partly used page.
    assert_eq!(error.checkpoint.next_token, None);
    first.assert();
}

/// A resumed listing treats the token of its checkpoint as seen: a page that
/// leads back to it is a cycle.
#[tokio::test]
async fn resumed_tokens_count_as_seen() {
    let mut server = mockito::Server::new_async().await;
    let transport = transport(&server, "/resumed");
    let checkpoint = Pager::new(&transport, ServiceKind::Tes, json!({}), options())
        .unwrap()
        .checkpoint();
    let checkpoint = PageCheckpoint {
        next_token: Some("again".into()),
        ..checkpoint
    };
    let page = server
        .mock("GET", "/resumed/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "again".into()))
        .with_body(r#"{"tasks":[],"next_page_token":"again"}"#)
        .expect(1)
        .create();
    let mut pager = Pager::resume(
        &transport,
        ServiceKind::Tes,
        json!({}),
        checkpoint,
        options(),
    )
    .unwrap();
    let error = pager.next_page().await.unwrap_err();
    assert_eq!(error.reason, "continuation token cycle");
    page.assert();
}

#[tokio::test]
async fn wes_pages_send_the_page_size() {
    let mut server = mockito::Server::new_async().await;
    let sized = server
        .mock("GET", "/wes/runs")
        .match_query(Matcher::UrlEncoded("page_size".into(), "2".into()))
        .with_body(r#"{"runs":[{"run_id":"run-1"}]}"#)
        .expect(1)
        .create();
    let unsized_page = server
        .mock("GET", "/wes/runs")
        .match_query(Matcher::Missing)
        .with_body(r#"{"runs":[]}"#)
        .expect(1)
        .create();
    let wes = WES::without_discovery(transport(&server, "/wes"), Service::default());
    let mut pager = wes.pages(Some(2), options()).unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["run_id"], "run-1");
    assert!(pager.next_item().await.unwrap().is_none());
    let mut pager = wes.pages(None, options()).unwrap();
    assert!(pager.next_page().await.unwrap().unwrap().items.is_empty());
    sized.assert();
    unsized_page.assert();
}
