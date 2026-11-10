use ga4gh_sdk::{
    clients::{
        drs::{
            resolution::{
                resolve_until, PendingResolution, Resolution, ResolutionReference, ResolveCause,
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
use mockito::{mock, server_url};
use std::{cell::Cell, time::Duration};
fn drs() -> DRS {
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
    DRS {
        transport: Transport::new(&config),
        config,
        service: Ok(Service::default()),
    }
}
fn reference() -> ResolutionReference {
    ResolutionReference {
        api_root: "https://drs.example/ga4gh/drs/v1".into(),
        endpoint: "objects/id".into(),
    }
}

#[tokio::test]
async fn both_single_attempts_preserve_pending_without_parsing() {
    let client = drs();
    for path in ["/objects/id", "/objects/id/access/a"] {
        let request = mock("GET", path)
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
    let request = mock("GET", "/objects/id")
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
    let client = drs();
    for header in [
        None,
        Some("-1"),
        Some("invalid"),
        Some("18446744073709551616"),
        Some("3601"),
    ] {
        let mut request = mock("GET", "/objects/id").with_status(202);
        if let Some(value) = header {
            request = request.with_header("retry-after", value);
        }
        let request = request.expect(1).create();
        assert!(client.get_object_once("id").await.is_err());
        request.assert();
    }
    for status in [200, 401, 403, 404] {
        let request = mock("GET", "/objects/id")
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
    let client = drs();
    let request = mock("GET", "/objects/id/access/a")
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
async fn metadata_and_access_resolve_202_to_200_without_an_early_retry() {
    let client = drs();
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
        let pending = mock("GET", path)
            .with_status(202)
            .with_header("retry-after", "1")
            .expect(1)
            .create();
        let ready = mock("GET", path)
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
