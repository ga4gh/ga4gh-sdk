use ga4gh_sdk::{
    clients::{
        tes::Task,
        wait::{observe, RemoteHandle, ServiceKind, WaitOptions, WaitOutcome},
    },
    utils::{configuration::Configuration, transport::Transport},
};
use serde_json::json;
use std::{cell::Cell, time::Duration};
fn handle(service: ServiceKind) -> RemoteHandle {
    RemoteHandle {
        service,
        api_root: "https://example.org/tes/v1/".into(),
        id: "opaque / % id".into(),
    }
}
#[tokio::test(start_paused = true)]
async fn state_sequences_and_terminal_failures() {
    for kind in [ServiceKind::Tes, ServiceKind::Wes] {
        for (state, outcome) in [
            ("COMPLETE", WaitOutcome::Complete),
            ("EXECUTOR_ERROR", WaitOutcome::ExecutorError),
            ("SYSTEM_ERROR", WaitOutcome::SystemError),
            ("PREEMPTED", WaitOutcome::Preempted),
            ("CANCELED", WaitOutcome::Canceled),
        ] {
            let calls = Cell::new(0);
            let states = ["QUEUED", "RUNNING", "CANCELING", state];
            let result = observe(
                handle(kind),
                &WaitOptions::new(Duration::from_secs(20)),
                || {
                    let index = calls.get();
                    calls.set(index + 1);
                    async move { Ok(json!({"state":states[index]})) }
                },
            )
            .await;
            assert_eq!(result.outcome, outcome);
            assert_eq!(result.last_state.as_deref(), Some(state));
            assert_eq!(calls.get(), 4);
        }
    }
}
#[tokio::test(start_paused = true)]
async fn unknown_states_deadline_and_interruption_keep_handle() {
    for cancel in [false, true] {
        let options = WaitOptions::new(Duration::from_secs(3));
        if cancel {
            let token = options.operation.cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                token.cancel();
            });
        }
        let expected = handle(ServiceKind::Tes);
        let result = observe(expected.clone(), &options, || async {
            Ok(json!({"state":"FUTURE_STATE"}))
        })
        .await;
        assert_eq!(result.handle, expected);
        assert_eq!(result.last_state.as_deref(), Some("FUTURE_STATE"));
        assert_eq!(
            result.outcome,
            if cancel {
                WaitOutcome::Interrupted
            } else {
                WaitOutcome::Deadline
            }
        );
    }
}
#[tokio::test(start_paused = true)]
async fn deadline_during_a_poll_ends_the_wait() {
    // The poll never answers, so the operation's deadline ends it; the outer
    // timeout fails the test instead of hanging it if the deadline doesn't.
    let result = tokio::time::timeout(
        Duration::from_secs(60),
        observe(
            handle(ServiceKind::Wes),
            &WaitOptions::new(Duration::from_secs(3)),
            std::future::pending::<ga4gh_sdk::Result<serde_json::Value>>,
        ),
    )
    .await
    .expect("the deadline ends the wait");
    assert_eq!(result.outcome, WaitOutcome::Deadline);
    assert_eq!(result.last_state, None);
    assert_eq!(result.error.as_deref(), Some("operation deadline exceeded"));
}
#[test]
fn resume_descriptor_is_credential_free_and_checked() {
    let mut config =
        Configuration::new(url::Url::parse("https://user:password@example.org/tes/v1").unwrap());
    config.bearer_access_token = Some("TOKEN".into());
    let task = Task::new("id".into(), Transport::new(&config));
    let descriptor = task.remote_handle();
    let serialized = serde_json::to_string(&descriptor).unwrap();
    for secret in ["password", "TOKEN", "user:"] {
        assert!(!serialized.contains(secret));
    }
    assert_eq!(descriptor.task(&config).unwrap().id, "id");
    assert!(descriptor.run(&config).is_err());
    let other = Configuration::new(url::Url::parse("https://other.example/tes/v1").unwrap());
    assert!(descriptor.task(&other).is_err());

    let double_slash = Configuration::new(url::Url::parse("https://example.org/tes/v1//").unwrap());
    assert!(descriptor.task(&double_slash).is_err());
}
#[tokio::test]
async fn real_waiter_retries_poll_and_fetches_failure_logs_once() {
    let mut server = mockito::Server::new_async().await;
    let first = server
        .mock("GET", "/tasks/id?view=MINIMAL")
        .with_status(503)
        .expect(1)
        .create();
    let status = server
        .mock("GET", "/tasks/id?view=MINIMAL")
        .with_body(r#"{"state":"SYSTEM_ERROR"}"#)
        .expect(1)
        .create();
    let log = server
        .mock("GET", "/tasks/id?view=FULL")
        .with_body(r#"{"state":"SYSTEM_ERROR","logs":[{"system_logs":["failed"]}]}"#)
        .expect(1)
        .create();
    let cancel = server.mock("POST", "/tasks/id:cancel").expect(0).create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let task = Task::new("id".into(), Transport::new(&config));
    let mut options = WaitOptions::new(Duration::from_secs(3));
    options.retry.max_attempts = 2;
    options.retry.initial_delay = Duration::from_millis(5);
    options.fetch_failure_logs = true;
    let result = task.wait(&options).await;
    assert_eq!(result.outcome, WaitOutcome::SystemError);
    assert!(result.details.is_some());
    first.assert();
    status.assert();
    log.assert();
    cancel.assert();
}

#[tokio::test]
async fn cancellation_acknowledgement_is_not_terminal_cancellation() {
    let mut server = mockito::Server::new_async().await;
    use ga4gh_sdk::clients::wes::Run;
    let ack = server
        .mock("POST", "/runs/id/cancel")
        .with_body(r#"{"run_id":"id"}"#)
        .expect(1)
        .create();
    let status = server
        .mock("GET", "/runs/id/status")
        .with_body(r#"{"run_id":"id","state":"CANCELING"}"#)
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let run = Run::new("id".into(), Transport::new(&config));
    let result = run
        .cancel_and_wait(&WaitOptions::new(Duration::from_millis(100)))
        .await
        .unwrap();
    assert_eq!(result.outcome, WaitOutcome::Deadline);
    assert_eq!(result.last_state.as_deref(), Some("CANCELING"));
    assert_eq!(result.handle.id, "id");
    ack.assert();
    status.assert();
}

#[test]
fn remote_handles_build_wes_runs() {
    let config = Configuration::new(url::Url::parse("https://example.org/wes/v1").unwrap());
    let run = ga4gh_sdk::clients::wes::Run::new("id".into(), Transport::new(&config));
    let handle = run.remote_handle();
    assert_eq!(handle.run(&config).unwrap().remote_handle(), handle);
    assert!(handle.task(&config).is_err());
}

#[tokio::test]
async fn unordered_intervals_fail_without_a_poll() {
    for (poll_interval, max_interval) in [
        (Duration::ZERO, Duration::from_secs(1)),
        (Duration::from_secs(2), Duration::from_secs(1)),
    ] {
        let mut options = WaitOptions::new(Duration::from_secs(10));
        options.poll_interval = poll_interval;
        options.max_interval = max_interval;
        let polls = Cell::new(0);
        let result = observe(handle(ServiceKind::Tes), &options, || {
            polls.set(polls.get() + 1);
            async { Ok(json!({"state":"COMPLETE"})) }
        })
        .await;
        assert_eq!(polls.get(), 0);
        assert_eq!(result.outcome, WaitOutcome::PollingFailed);
        assert_eq!(
            result.error.as_deref(),
            Some("wait intervals must be positive and ordered")
        );
    }
}

/// How a wait ends when a poll fails or gives no state, and when its
/// operation is cancelled before the first poll.
#[tokio::test]
async fn failed_polls_end_the_wait_with_their_outcome() {
    use ga4gh_sdk::utils::operation::OperationError;
    for (error, outcome) in [
        (OperationError::Deadline, WaitOutcome::Deadline),
        (OperationError::Cancelled, WaitOutcome::Interrupted),
    ] {
        let result = observe(
            handle(ServiceKind::Wes),
            &WaitOptions::new(Duration::from_secs(10)),
            || async move { Err(ga4gh_sdk::Error::other(error)) },
        )
        .await;
        assert_eq!(result.outcome, outcome);
        assert_eq!(result.error, Some(error.to_string()));
    }
    let result = observe(
        handle(ServiceKind::Wes),
        &WaitOptions::new(Duration::from_secs(10)),
        || async { Ok(json!({"state": 1})) },
    )
    .await;
    assert_eq!(result.outcome, WaitOutcome::PollingFailed);
    assert_eq!(
        result.error.as_deref(),
        Some("poll response has no string state")
    );
    let options = WaitOptions::new(Duration::from_secs(10));
    options.operation.cancellation.cancel();
    let polls = Cell::new(0);
    let result = observe(handle(ServiceKind::Wes), &options, || {
        polls.set(polls.get() + 1);
        async { Ok(json!({"state":"COMPLETE"})) }
    })
    .await;
    assert_eq!(result.outcome, WaitOutcome::Interrupted);
    assert_eq!(result.error.as_deref(), Some("operation canceled locally"));
}

#[tokio::test]
async fn poll_responses_that_are_not_json_fail_the_wait() {
    let mut server = mockito::Server::new_async().await;
    let status = server
        .mock("GET", "/tasks/id?view=MINIMAL")
        .with_body("not JSON")
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let task = Task::new("id".into(), Transport::new(&config));
    let result = task.wait(&WaitOptions::new(Duration::from_secs(10))).await;
    assert_eq!(result.outcome, WaitOutcome::PollingFailed);
    assert!(result.error.is_some());
    status.assert();
}

/// A failure log that can't be read keeps the outcome and says why in
/// `error`: an unparsable body, an HTTP error, or a cancellation while it
/// is fetched.
#[tokio::test]
async fn unreadable_failure_logs_keep_the_outcome() {
    for case in ["parse", "status", "cancel"] {
        let mut server = mockito::Server::new_async().await;
        let mut options = WaitOptions::new(Duration::from_secs(10));
        options.fetch_failure_logs = true;
        let status = server
            .mock("GET", "/tasks/id?view=MINIMAL")
            .with_body(r#"{"state":"EXECUTOR_ERROR"}"#)
            .expect(1)
            .create();
        let log = server.mock("GET", "/tasks/id?view=FULL");
        let log = match case {
            "parse" => log.with_body("not JSON"),
            "status" => log.with_status(500),
            _ => {
                let token = options.operation.cancellation.clone();
                log.with_body_from_request(move |_| {
                    token.cancel();
                    b"{}".to_vec()
                })
            }
        }
        .expect(1)
        .create();
        let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
        let task = Task::new("id".into(), Transport::new(&config));
        let result = task.wait(&options).await;
        assert_eq!(result.outcome, WaitOutcome::ExecutorError);
        assert!(result.details.is_none());
        let error = result.error.unwrap();
        let expected = match case {
            "parse" => "failure detail parse: ",
            "status" => "failure detail lookup: Request failed with status: 500",
            _ => "failure detail lookup: operation canceled locally",
        };
        assert!(error.starts_with(expected), "{case}: {error}");
        status.assert();
        log.assert();
    }
}

#[tokio::test]
async fn tasks_cancel_then_wait() {
    let mut server = mockito::Server::new_async().await;
    let cancel = server
        .mock("POST", "/tasks/id:cancel")
        .with_body("{}")
        .expect(1)
        .create();
    let status = server
        .mock("GET", "/tasks/id?view=MINIMAL")
        .with_body(r#"{"state":"CANCELED"}"#)
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server.url()).unwrap());
    let task = Task::new("id".into(), Transport::new(&config));
    let result = task
        .cancel_and_wait(&WaitOptions::new(Duration::from_secs(10)))
        .await
        .unwrap();
    assert_eq!(result.outcome, WaitOutcome::Canceled);
    cancel.assert();
    status.assert();
}
