use ga4gh_sdk::{
    clients::{
        pagination::{Pager, PagingOptions},
        wait::ServiceKind,
    },
    utils::{configuration::Configuration, transport::Transport},
};
use mockito::{mock, server_url, Matcher};
use serde_json::json;
use std::time::Duration;
fn transport(path: &str) -> Transport {
    Transport::new(&Configuration::new(
        url::Url::parse(&format!("{}{path}", server_url())).unwrap(),
    ))
}
fn options() -> PagingOptions {
    PagingOptions::new(Duration::from_secs(10))
}
#[tokio::test]
async fn pages_preserve_filters_and_opaque_tokens_across_empty_intermediate_pages() {
    let first = mock("GET", "/three/tasks")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("name_prefix".into(), "sample".into()),
            Matcher::UrlEncoded("view".into(), "FULL".into()),
        ]))
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"a+/ %"}"#)
        .expect(1)
        .create();
    let second = mock("GET", "/three/tasks")
        .match_query(Matcher::AllOf(vec![
            Matcher::UrlEncoded("name_prefix".into(), "sample".into()),
            Matcher::UrlEncoded("view".into(), "FULL".into()),
            Matcher::UrlEncoded("page_token".into(), "a+/ %".into()),
        ]))
        .with_body(r#"{"tasks":[],"next_page_token":"next"}"#)
        .expect(1)
        .create();
    let third = mock("GET", "/three/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "next".into()))
        .with_body(r#"{"tasks":[{"id":"two"}],"next_page_token":""}"#)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport("/three"),
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
    let first = mock("GET", "/lazy/tasks")
        .with_body(r#"{"tasks":[{"id":"one"},{"id":"two"}],"next_page_token":"later"}"#)
        .expect(1)
        .create();
    let next = mock("GET", "/lazy/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "later".into()))
        .expect(0)
        .create();
    let mut pager =
        Pager::new(&transport("/lazy"), ServiceKind::Tes, json!({}), options()).unwrap();
    assert_eq!(pager.next_item().await.unwrap().unwrap()["id"], "one");
    assert_eq!(pager.checkpoint().next_token, None);
    drop(pager);
    first.assert();
    next.assert();
}
#[tokio::test]
async fn cycles_fail_and_later_request_failure_retains_continuation() {
    let first = mock("GET", "/cycle/tasks")
        .with_body(r#"{"tasks":[],"next_page_token":"same"}"#)
        .expect(1)
        .create();
    let second = mock("GET", "/cycle/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "same".into()))
        .with_body(r#"{"tasks":[],"next_page_token":"same"}"#)
        .expect(1)
        .create();
    let mut pager =
        Pager::new(&transport("/cycle"), ServiceKind::Tes, json!({}), options()).unwrap();
    pager.next_page().await.unwrap();
    let error = pager.next_page().await.unwrap_err();
    assert!(error.reason.contains("cycle"));
    assert_eq!(error.checkpoint.next_token.as_deref(), Some("same"));
    first.assert();
    second.assert();
    let first = mock("GET", "/failure/tasks")
        .with_body(r#"{"tasks":[{"id":"one"}],"next_page_token":"retry"}"#)
        .expect(1)
        .create();
    let second = mock("GET", "/failure/tasks")
        .match_query(Matcher::UrlEncoded("page_token".into(), "retry".into()))
        .with_status(503)
        .expect(1)
        .create();
    let mut pager = Pager::new(
        &transport("/failure"),
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
    let request = mock("GET", "/resume/tasks")
        .match_query(Matcher::UrlEncoded("view".into(), "FULL".into()))
        .with_body(r#"{"tasks":[{"id":"one"},{"id":"two"}],"next_page_token":"later"}"#)
        .expect(1)
        .create();
    let transport = transport("/resume");
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
    let resumed = mock("GET", "/resume/tasks")
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
