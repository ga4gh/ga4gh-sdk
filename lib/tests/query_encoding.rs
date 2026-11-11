//! Query strings that the transport and the pager actually send.
//!
//! Each case goes through a public request path to a mock server that matches
//! the raw query string exactly.
use ga4gh_sdk::{
    clients::tes::{models::ListTasksParams, TES},
    clients::{
        pagination::{Pager, PagingOptions},
        wait::ServiceKind,
    },
    utils::{configuration::Configuration, query::encode_query, transport::Transport},
};
use mockito::{mock, server_url, Matcher};
use serde_json::{json, Value};
use std::time::Duration;

fn transport(path: &str) -> Transport {
    Transport::new(&Configuration::new(
        url::Url::parse(&format!("{}{path}", server_url())).unwrap(),
    ))
}

/// Cases that every path encodes in the same way: parameters and the exact
/// query string they produce.
fn shared_cases() -> Vec<(&'static str, Value, &'static str)> {
    vec![
        (
            "reserved",
            json!({"q": "a b&c=d/é+%?#", "a b": "c"}),
            "a+b=c&q=a+b%26c%3Dd%2F%C3%A9%2B%25%3F%23",
        ),
        (
            "scalars",
            json!({"b": true, "f": 1.5, "n": -10, "s": ""}),
            "b=true&f=1.5&n=-10&s=",
        ),
        ("omitted", json!({"a": null, "e": [], "k": "v"}), "k=v"),
        (
            "repeated",
            json!({"tag_key": ["a", "b"], "tag_value": ["x", ""], "page_size": 2}),
            "page_size=2&tag_key=a&tag_key=b&tag_value=x&tag_value=",
        ),
    ]
}

/// Values outside the shared policy: objects, nested lists and lists with `null`.
fn unsupported_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("null-item", json!({"tag_value": ["x", null, "y"]})),
        ("object", json!({"filter": {"a": 1}})),
        ("nested-list", json!({"k": [["a"]]})),
        ("object-item", json!({"k": [{"a": 1}]})),
    ]
}

#[tokio::test]
async fn transport_sends_shared_cases() {
    for (name, params, query) in shared_cases() {
        // The Python bindings send `encode_query`'s output; it must match.
        assert_eq!(encode_query(&params).unwrap(), query, "{name}");
        let request = mock("GET", format!("/qt-{name}/tasks").as_str())
            .match_query(Matcher::Exact(query.to_owned()))
            .with_body("ok")
            .expect(1)
            .create();
        let body = transport(&format!("/qt-{name}"))
            .get("tasks", Some(params))
            .await
            .unwrap();
        assert_eq!(body, "ok", "{name}");
        request.assert();
    }
}

#[tokio::test]
async fn transport_rejects_unsupported_values_before_sending() {
    for (name, params) in unsupported_cases() {
        let request = mock("GET", format!("/qtu-{name}/tasks").as_str())
            .match_query(Matcher::Any)
            .expect(0)
            .create();
        let error = transport(&format!("/qtu-{name}"))
            .get("tasks", Some(params))
            .await
            .unwrap_err();
        let error = error
            .downcast_ref::<std::io::Error>()
            .unwrap_or_else(|| panic!("{name}: {error}"));
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput, "{name}");
        assert!(
            error.to_string().starts_with("query parameter '"),
            "{name}: {error}"
        );
        request.assert();
    }
}

async fn first_page(path: &str, query: Value) -> Result<(), String> {
    let mut pager = Pager::new(
        &transport(path),
        ServiceKind::Tes,
        query,
        PagingOptions::new(Duration::from_secs(10)),
    )
    .map_err(|error| error.to_string())?;
    pager
        .next_page()
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tokio::test]
async fn pager_sends_shared_cases() {
    for (name, params, query) in shared_cases() {
        let request = mock("GET", format!("/qp-{name}/tasks").as_str())
            .match_query(Matcher::Exact(query.to_owned()))
            .with_body(r#"{"tasks":[]}"#)
            .expect(1)
            .create();
        first_page(&format!("/qp-{name}"), params).await.unwrap();
        request.assert();
    }
}

#[tokio::test]
async fn pager_encodes_its_token_like_other_parameters() {
    let request = mock("GET", "/qp-token/tasks")
        .match_query(Matcher::Exact("a=1&page_token=t%2F%2B+%25&z=2".to_owned()))
        .with_body(r#"{"tasks":[]}"#)
        .expect(1)
        .create();
    first_page(
        "/qp-token",
        json!({"a": "1", "z": 2, "page_token": "t/+ %"}),
    )
    .await
    .unwrap();
    request.assert();
}

#[tokio::test]
async fn pager_rejects_unsupported_values_before_sending() {
    for (name, params) in unsupported_cases() {
        let request = mock("GET", format!("/qpu-{name}/tasks").as_str())
            .match_query(Matcher::Any)
            .expect(0)
            .create();
        let error = first_page(&format!("/qpu-{name}"), params)
            .await
            .unwrap_err();
        assert!(error.starts_with("query parameter '"), "{name}: {error}");
        request.assert();
    }
}

#[tokio::test]
async fn tes_pages_repeat_tags_in_order() {
    let request = mock("GET", "/qp-tags/tasks")
        .match_query(Matcher::Exact(
            "page_size=2&tag_key=team+a&tag_key=x%2Fy&tag_value=&tag_value=1%262".to_owned(),
        ))
        .with_body(r#"{"tasks":[]}"#)
        .expect(1)
        .create();
    let tes = TES {
        config: Configuration::new(url::Url::parse(&format!("{}/qp-tags", server_url())).unwrap()),
        service: Err("unused".into()),
        transport: transport("/qp-tags"),
    };
    let params = ListTasksParams {
        tag_key: Some(vec!["team a".into(), "x/y".into()]),
        tag_value: Some(vec!["".into(), "1&2".into()]),
        page_size: Some(2),
        name_prefix: None,
        state: None,
        page_token: None,
        view: None,
    };
    let mut pager = tes
        .pages(Some(params), PagingOptions::new(Duration::from_secs(10)))
        .unwrap();
    pager.next_page().await.unwrap();
    request.assert();
}
