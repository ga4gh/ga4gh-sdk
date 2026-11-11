use ga4gh_sdk::{
    reqwest::Method,
    utils::{
        configuration::Configuration,
        operation::{AmbiguousSubmissionError, OperationError},
        transport::{HttpError, RequestOptions, Transport},
    },
};
use std::time::Duration;
use tokio::{io::AsyncReadExt, net::TcpListener};

async fn submit(
    transport: &Transport,
    options: &RequestOptions,
    streamed: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    if streamed {
        transport
            .send_once(
                transport
                    .request_builder(Method::POST, "tasks")
                    .unwrap()
                    .body("{}"),
                &options.operation,
                options.limits,
            )
            .await
    } else {
        transport
            .request_with_policy(
                Method::POST,
                "tasks",
                Some(serde_json::json!({})),
                None,
                options,
            )
            .await
    }
}

#[tokio::test]
async fn interruption_before_submission_is_inspectable_and_sends_no_request() {
    for streamed in [false, true] {
        for expected in [OperationError::Cancelled, OperationError::Deadline] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let transport = Transport::new(&Configuration::new(
                url::Url::parse(&format!("http://{address}")).unwrap(),
            ));
            let mut options = RequestOptions::new(Duration::from_secs(10));
            options.retry.max_attempts = 3;
            match expected {
                OperationError::Cancelled => options.operation.cancellation.cancel(),
                OperationError::Deadline => {
                    options.operation.deadline = tokio::time::Instant::now()
                }
            }
            let error = submit(&transport, &options, streamed).await.unwrap_err();
            assert_eq!(
                error.downcast_ref::<OperationError>(),
                Some(&expected),
                "{error}"
            );
            assert!(error.downcast_ref::<AmbiguousSubmissionError>().is_none());
            assert!(
                tokio::time::timeout(Duration::from_millis(50), listener.accept())
                    .await
                    .is_err()
            );
        }
    }
}

#[tokio::test]
async fn cancellation_after_submission_starts_is_ambiguous_without_replay() {
    for streamed in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let transport = Transport::new(&Configuration::new(
            url::Url::parse(&format!("http://{address}")).unwrap(),
        ));
        let mut options = RequestOptions::new(Duration::from_secs(10));
        options.retry.max_attempts = 3;
        let cancellation = options.operation.cancellation.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut data = [0; 4096];
            assert!(socket.read(&mut data).await.unwrap() > 0);
            // Cancellation races with response waiting only after a real request
            // has reached the server; keep the connection open without replying.
            cancellation.cancel();
            assert!(
                tokio::time::timeout(Duration::from_millis(100), listener.accept())
                    .await
                    .is_err()
            );
            drop(socket);
        });
        let error = submit(&transport, &options, streamed).await.unwrap_err();
        assert!(
            error.downcast_ref::<AmbiguousSubmissionError>().is_some(),
            "{error}"
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn definite_http_rejection_preserves_status_and_body_without_replay() {
    for streamed in [false, true] {
        let rejection = mockito::mock("POST", "/tasks")
            .with_status(403)
            .with_body("permission denied")
            .expect(1)
            .create();
        let transport = Transport::new(&Configuration::new(
            url::Url::parse(&mockito::server_url()).unwrap(),
        ));
        let mut options = RequestOptions::new(Duration::from_secs(10));
        options.retry.max_attempts = 3;
        let error = submit(&transport, &options, streamed).await.unwrap_err();
        let http = error
            .downcast_ref::<HttpError>()
            .expect("structured HTTP error");
        assert_eq!(http.status.as_u16(), 403);
        assert_eq!(http.body, "permission denied");
        rejection.assert();
    }
}
