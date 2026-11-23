use ga4gh_sdk::{
    clients::{
        drs::{
            resolution::{
                resolve_until, PendingResolution, Resolution, ResolutionReference, ResolveCause,
                ResolveError,
            },
            DRS,
        },
        serviceinfo::models::Service,
    },
    utils::{
        configuration::Configuration,
        operation::{Operation, OperationError},
        transport::Transport,
    },
};
use std::{cell::Cell, error::Error, time::Duration};
fn drs(server: &mockito::ServerGuard) -> DRS {
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    DRS::without_discovery(Transport::new(&config), Service::default())
}
fn reference() -> ResolutionReference {
    ResolutionReference {
        api_root: "https://drs.example/ga4gh/drs/v1".into(),
        endpoint: "objects/id".into(),
    }
}

#[tokio::test]
async fn both_single_attempts_preserve_pending_without_parsing() {
    let mut server = mockito::Server::new_async().await;
    let client = drs(&server);
    for path in ["/objects/id", "/objects/id/access/a"] {
        let request = server
            .mock("GET", path)
            .with_status(202)
            .with_header("retry-after", "1")
            .with_body("")
            .expect(1)
            .create();
        let reference = client.resolution_reference(path.trim_start_matches('/').into());
        let result: Resolution<serde_json::Value> =
            client.resolve_reference(&reference).await.unwrap();
        match result {
            Resolution::Pending(p) => {
                assert_eq!(p.retry_after_secs, 1);
                assert_eq!(p.reference, reference);
            }
            _ => panic!("202 parsed as ready"),
        }
        request.assert();
    }
    let request = server
        .mock("GET", "/objects/id")
        .with_status(202)
        .with_header("retry-after", "1")
        .create();
    assert!(client
        .get_object("id")
        .await
        .unwrap_err()
        .downcast_ref::<PendingResolution>()
        .is_some());
    request.assert();
}
#[tokio::test]
async fn malformed_delays_and_final_json_remain_errors() {
    let mut server = mockito::Server::new_async().await;
    let client = drs(&server);
    for header in [
        None,
        Some("-1"),
        Some("invalid"),
        Some("18446744073709551616"),
        Some("3601"),
    ] {
        let mut request = server.mock("GET", "/objects/id").with_status(202);
        if let Some(value) = header {
            request = request.with_header("retry-after", value);
        }
        let request = request.expect(1).create();
        assert!(client.get_object_once("id").await.is_err());
        request.assert();
    }
    for status in [200, 401, 403, 404] {
        let request = server
            .mock("GET", "/objects/id")
            .with_status(status)
            .with_body("invalid")
            .expect(1)
            .create();
        assert!(client.get_object_once("id").await.is_err());
        request.assert();
    }
}
#[tokio::test(start_paused = true)]
async fn no_early_retry_and_success_after_pending() {
    let reference = reference();
    let calls = Cell::new(0);
    let start = tokio::time::Instant::now();
    let value = resolve_until(
        reference.clone(),
        &Operation::new(Duration::from_secs(10)),
        3,
        || {
            let call = calls.get();
            calls.set(call + 1);
            let reference = reference.clone();
            async move {
                if call == 0 {
                    Ok(Resolution::Pending(PendingResolution {
                        reference,
                        retry_after_secs: 2,
                    }))
                } else {
                    assert!(tokio::time::Instant::now() - start >= Duration::from_secs(2));
                    Ok(Resolution::Ready("ready"))
                }
            }
        },
    )
    .await
    .unwrap();
    assert_eq!(value, "ready");
    assert_eq!(calls.get(), 2);
}
#[tokio::test(start_paused = true)]
async fn deadline_cancel_and_attempt_limits_retain_resume_state() {
    for cause in ["deadline", "cancel", "attempts"] {
        let reference = reference();
        let calls = Cell::new(0);
        let operation = Operation::new(Duration::from_secs(2));
        if cause == "cancel" {
            let token = operation.cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(1)).await;
                token.cancel();
            });
        }
        let result = resolve_until::<(), _, _>(
            reference.clone(),
            &operation,
            if cause == "attempts" { 1 } else { 10 },
            || {
                calls.set(calls.get() + 1);
                let reference = reference.clone();
                async move {
                    Ok(Resolution::Pending(PendingResolution {
                        reference,
                        retry_after_secs: 5,
                    }))
                }
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls.get(), 1);
        assert_eq!(result.reference, reference);
        assert!(result.last_pending.is_some());
        match (cause, result.cause) {
            ("deadline", ResolveCause::Operation(OperationError::Deadline))
            | ("cancel", ResolveCause::Operation(OperationError::Cancelled))
            | ("attempts", ResolveCause::AttemptsExhausted) => (),
            _ => panic!("wrong resolution outcome"),
        }
    }
}
#[tokio::test]
async fn immediate_200_and_resume_validation() {
    let mut server = mockito::Server::new_async().await;
    let client = drs(&server);
    let request = server
        .mock("GET", "/objects/id/access/a")
        .with_body(r#"{"url":"https://store.example/data"}"#)
        .expect(1)
        .create();
    assert_eq!(
        client.get_access_url("id", "a").await.unwrap().url,
        "https://store.example/data"
    );
    request.assert();
    assert!(client
        .resolve_reference::<serde_json::Value>(&reference())
        .await
        .is_err());
}

#[tokio::test]
async fn reference_root_is_transport_root() {
    let mut server = mockito::Server::new_async().await;
    // Regression for #292: the reference names the root of the transport
    // that received the request, including a non-default path.
    let root = format!("{}/custom/ga4gh/drs/v9", server.url());
    let config = Configuration::new(url::Url::parse(&root).unwrap());
    let client = DRS::without_discovery(Transport::new(&config), Service::default());
    let request = server
        .mock("GET", "/custom/ga4gh/drs/v9/objects/id")
        .with_status(202)
        .with_header("retry-after", "1")
        .expect(1)
        .create();
    let error = client.get_object_raw("id").await.unwrap_err();
    let pending = error
        .downcast_ref::<PendingResolution>()
        .expect("202 is pending");
    request.assert();
    request.remove();
    assert_eq!(pending.reference.api_root, format!("{root}/"));
    assert_eq!(pending.reference.endpoint, "objects/id");

    // The reference resumes against the transport root; a reference for a
    // different root is rejected before another request.
    let ready = server
        .mock("GET", "/custom/ga4gh/drs/v9/objects/id")
        .with_body(r#"{"id":"id"}"#)
        .expect(1)
        .create();
    let result: Resolution<serde_json::Value> =
        client.resolve_reference(&pending.reference).await.unwrap();
    assert!(matches!(result, Resolution::Ready(value) if value["id"] == "id"));
    ready.assert();
    let foreign = ResolutionReference {
        api_root: "https://other.example/ga4gh/drs/v1/".into(),
        endpoint: "objects/id".into(),
    };
    assert!(client
        .resolve_reference::<serde_json::Value>(&foreign)
        .await
        .is_err());
}

#[tokio::test]
async fn resume_accepts_equivalent_api_root_with_trailing_slash() {
    let mut server = mockito::Server::new_async().await;
    let root = format!("{}/ga4gh/drs/v1", server.url());
    let first_config = Configuration::new(url::Url::parse(&root).unwrap());
    let first = DRS::without_discovery(Transport::new(&first_config), Service::default());
    let reference = first.resolution_reference("objects/id".into());
    let second_config = Configuration::new(url::Url::parse(&format!("{root}/")).unwrap());
    let second = DRS::without_discovery(Transport::new(&second_config), Service::default());
    let request = server
        .mock("GET", "/ga4gh/drs/v1/objects/id")
        .with_body(r#"{"id":"id"}"#)
        .create();
    let result: Resolution<serde_json::Value> = second.resolve_reference(&reference).await.unwrap();
    assert!(matches!(result, Resolution::Ready(value) if value["id"] == "id"));
    request.assert();

    let encoded_config =
        Configuration::new(url::Url::parse("https://user:pass@example.org/a%20b/drs/v1").unwrap());
    let encoded = DRS::without_discovery(Transport::new(&encoded_config), Service::default());
    assert_eq!(
        encoded.resolution_reference("objects/id".into()).api_root,
        "https://example.org/a%20b/drs/v1/"
    );
}

#[tokio::test]
async fn metadata_and_access_resolve_202_to_200_without_an_early_retry() {
    let mut server = mockito::Server::new_async().await;
    let client = drs(&server);
    for (path, body) in [
        (
            "/objects/id",
            r#"{"id":"id","self_uri":"drs://example.org/id","size":1,"created_time":"2026-01-01T00:00:00Z","checksums":[]}"#,
        ),
        (
            "/objects/id/access/a",
            r#"{"url":"https://store.example/data"}"#,
        ),
    ] {
        let pending = server
            .mock("GET", path)
            .with_status(202)
            .with_header("retry-after", "1")
            .expect(1)
            .create();
        let ready = server
            .mock("GET", path)
            .with_status(200)
            .with_body(body)
            .expect(1)
            .create();
        let operation = Operation::new(Duration::from_secs(5));
        let start = std::time::Instant::now();
        if path.contains("/access/") {
            client
                .resolve_access_url("id", "a", &operation, 2)
                .await
                .unwrap();
        } else {
            client.resolve_object("id", &operation, 2).await.unwrap();
        }
        assert!(start.elapsed() >= Duration::from_secs(1));
        pending.assert();
        ready.assert();
    }
}

#[test]
fn resolve_errors_describe_and_expose_their_cause() {
    let error = |cause| ResolveError {
        reference: reference(),
        last_pending: None,
        cause,
    };
    let interrupted = error(ResolveCause::Operation(OperationError::Deadline));
    assert_eq!(
        interrupted.to_string(),
        "DRS resolution interrupted: operation deadline exceeded"
    );
    assert_eq!(
        interrupted
            .source()
            .and_then(|e| e.downcast_ref::<OperationError>()),
        Some(&OperationError::Deadline)
    );
    let exhausted = error(ResolveCause::AttemptsExhausted);
    assert_eq!(
        exhausted.to_string(),
        "DRS resolution attempts exhausted; reference retained"
    );
    assert!(exhausted.source().is_none());
    let failed = error(ResolveCause::Request("server answered 500".into()));
    assert_eq!(
        failed.to_string(),
        "DRS resolution request failed: server answered 500"
    );
    assert_eq!(failed.source().unwrap().to_string(), "server answered 500");
}

#[tokio::test]
async fn attempt_limits_and_ended_operations_stop_before_any_attempt() {
    for max_attempts in [0, 10_001] {
        let calls = Cell::new(0);
        let error = resolve_until::<(), _, _>(
            reference(),
            &Operation::new(Duration::from_secs(5)),
            max_attempts,
            || {
                calls.set(calls.get() + 1);
                async { Ok(Resolution::Ready(())) }
            },
        )
        .await
        .unwrap_err();
        assert_eq!(calls.get(), 0);
        assert!(error.last_pending.is_none());
        match error.cause {
            ResolveCause::Request(error) => {
                assert_eq!(error.to_string(), "resolution attempts must be 1..=10000")
            }
            _ => panic!("wrong cause for {max_attempts} attempts"),
        }
    }
    let operation = Operation::new(Duration::from_secs(5));
    operation.cancellation.cancel();
    // The attempt future is created but never polled.
    let polled = Cell::new(0);
    let error = resolve_until::<(), _, _>(reference(), &operation, 3, || async {
        polled.set(polled.get() + 1);
        Ok(Resolution::Ready(()))
    })
    .await
    .unwrap_err();
    assert_eq!(polled.get(), 0);
    assert!(error.last_pending.is_none());
    assert!(matches!(
        error.cause,
        ResolveCause::Operation(OperationError::Cancelled)
    ));
}

#[tokio::test]
async fn retry_after_must_be_visible_ascii() {
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("GET", "/objects/id")
        .with_status(202)
        .with_header("retry-after", "1\u{e9}")
        .expect(1)
        .create();
    let error = drs(&server).get_object_once("id").await.unwrap_err();
    assert!(error
        .downcast_ref::<ga4gh_sdk::reqwest::header::ToStrError>()
        .is_some());
    request.assert();
}

#[tokio::test]
async fn pending_access_url_is_an_error_of_the_plain_lookup() {
    let mut server = mockito::Server::new_async().await;
    let request = server
        .mock("GET", "/objects/id/access/a")
        .with_status(202)
        .with_header("retry-after", "7")
        .expect(1)
        .create();
    let error = drs(&server).get_access_url("id", "a").await.unwrap_err();
    let pending = error
        .downcast_ref::<PendingResolution>()
        .expect("202 is pending");
    assert_eq!(pending.retry_after_secs, 7);
    assert_eq!(pending.reference.endpoint, "objects/id/access/a");
    request.assert();
}
