use ga4gh_sdk::{
    reqwest::Method,
    utils::{
        configuration::Configuration,
        operation::{
            retry_after, AmbiguousSubmissionError, Operation, OperationError, ReplaySafety,
        },
        transport::{RequestOptions, Transport},
    },
};
use mockito::{mock, server_url};
use std::time::{Duration, SystemTime};
fn transport() -> Transport {
    Transport::new(&Configuration::new(url::Url::parse(&server_url()).unwrap()))
}
#[test]
fn retry_after_supports_delta_and_date() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784111777);
    assert_eq!(retry_after("2", now).unwrap(), Duration::from_secs(2));
    let date = httpdate::fmt_http_date(now + Duration::from_secs(5));
    assert_eq!(retry_after(&date, now).unwrap(), Duration::from_secs(5));
    for value in ["-1", "junk", "18446744073709551616"] {
        assert!(retry_after(value, now).is_err());
    }
}
#[tokio::test]
async fn safe_reads_retry_503_but_permissions_do_not() {
    for status in [429, 503, 401, 403] {
        let path = format!("/status{status}");
        let first = mock("GET", path.as_str())
            .with_status(status)
            .with_header("retry-after", "0")
            .expect(1)
            .create();
        let second = mock("GET", path.as_str())
            .with_status(200)
            .with_body("ready")
            .expect(if status == 429 || status == 503 { 1 } else { 0 })
            .create();
        // mockito uses the first unfulfilled matching mock.
        let mut options = RequestOptions::new(Duration::from_secs(2));
        options.safety = ReplaySafety::ReadOnly;
        options.retry.max_attempts = 2;
        options.retry.initial_delay = Duration::from_millis(5);
        let result = transport()
            .request_with_policy(Method::GET, &path, None, None, &options)
            .await;
        if status == 429 || status == 503 {
            assert_eq!(result.unwrap(), "ready");
        } else {
            assert!(result.is_err());
        }
        first.assert();
        second.assert();
    }
}
#[tokio::test]
async fn dropped_submission_is_ambiguous_and_never_replayed() {
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        drop(socket);
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let transport = Transport::new(&Configuration::new(
        url::Url::parse(&format!("http://{address}")).unwrap(),
    ));
    let mut options = RequestOptions::new(Duration::from_secs(2));
    options.retry.max_attempts = 3;
    let error = transport
        .request_with_policy(
            Method::POST,
            "tasks",
            Some(serde_json::json!({})),
            None,
            &options,
        )
        .await
        .unwrap_err();
    assert!(
        error.downcast_ref::<AmbiguousSubmissionError>().is_some(),
        "{error}"
    );
    server.await.unwrap();
}
#[tokio::test]
async fn stalled_headers_and_body_share_a_deadline() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for headers in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            if headers {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nx")
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let transport = Transport::new(&Configuration::new(
            url::Url::parse(&format!("http://{address}")).unwrap(),
        ));
        let mut options = RequestOptions::new(Duration::from_millis(100));
        options.safety = ReplaySafety::ReadOnly;
        let start = std::time::Instant::now();
        let error = transport
            .request_with_policy(Method::GET, "stall", None, None, &options)
            .await
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<OperationError>(),
            Some(&OperationError::Deadline)
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        server.abort();
    }
}
#[tokio::test(start_paused = true)]
async fn cancellation_and_absolute_budget_apply_during_sleep() {
    let operation = Operation::new(Duration::from_secs(3));
    let token = operation.cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        token.cancel();
    });
    assert_eq!(
        operation.sleep(Duration::from_secs(10)).await,
        Err(OperationError::Cancelled)
    );
    let operation = Operation::new(Duration::from_secs(3));
    operation.sleep(Duration::from_secs(2)).await.unwrap();
    assert_eq!(
        operation.sleep(Duration::from_secs(2)).await,
        Err(OperationError::Deadline)
    );
}

#[tokio::test]
async fn cancellation_during_retry_after_sends_no_more_requests() {
    let first = mock("GET", "/backoff")
        .with_status(503)
        .with_header("retry-after", "10")
        .expect(1)
        .create();
    let mut options = RequestOptions::new(Duration::from_secs(20));
    options.safety = ReplaySafety::ReadOnly;
    options.retry.max_attempts = 3;
    let token = options.operation.cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
    });
    let error = transport()
        .request_with_policy(Method::GET, "backoff", None, None, &options)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<OperationError>(),
        Some(&OperationError::Cancelled)
    );
    first.assert();
}

#[tokio::test]
async fn read_only_post_can_retry_but_nonreplayable_get_cannot() {
    let first = mock("POST", "/lookup").with_status(503).expect(1).create();
    let second = mock("POST", "/lookup")
        .with_status(200)
        .with_body("ready")
        .expect(1)
        .create();
    let mut options = RequestOptions::new(Duration::from_secs(2));
    options.safety = ReplaySafety::ReadOnly;
    options.retry.max_attempts = 2;
    options.retry.initial_delay = Duration::from_millis(5);
    assert_eq!(
        transport()
            .request_with_policy(
                Method::POST,
                "lookup",
                Some(serde_json::json!({"ids":["a"]})),
                None,
                &options
            )
            .await
            .unwrap(),
        "ready"
    );
    first.assert();
    second.assert();
    let request = mock("GET", "/nonreplayable")
        .with_status(503)
        .expect(1)
        .create();
    options.safety = ReplaySafety::NonReplayable;
    assert!(transport()
        .request_with_policy(Method::GET, "nonreplayable", None, None, &options)
        .await
        .is_err());
    request.assert();
}
