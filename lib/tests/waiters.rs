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
}
#[tokio::test]
async fn real_waiter_retries_poll_and_fetches_failure_logs_once() {
    use mockito::{mock, server_url};
    let first = mock("GET", "/tasks/id?view=MINIMAL")
        .with_status(503)
        .expect(1)
        .create();
    let status = mock("GET", "/tasks/id?view=MINIMAL")
        .with_body(r#"{"state":"SYSTEM_ERROR"}"#)
        .expect(1)
        .create();
    let log = mock("GET", "/tasks/id?view=FULL")
        .with_body(r#"{"state":"SYSTEM_ERROR","logs":[{"system_logs":["failed"]}]}"#)
        .expect(1)
        .create();
    let cancel = mock("POST", "/tasks/id:cancel").expect(0).create();
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
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
    use ga4gh_sdk::clients::wes::Run;
    use mockito::{mock, server_url};
    let ack = mock("POST", "/runs/id/cancel")
        .with_body(r#"{"run_id":"id"}"#)
        .expect(1)
        .create();
    let status = mock("GET", "/runs/id/status")
        .with_body(r#"{"run_id":"id","state":"CANCELING"}"#)
        .expect(1)
        .create();
    let config = Configuration::new(url::Url::parse(&server_url()).unwrap());
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
