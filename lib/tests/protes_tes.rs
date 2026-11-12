//! Runs the TES client against proTES, a TES gateway that relays tasks to
//! Funnel and requires access tokens; see `tests/run-protes-integration.sh`,
//! which starts both services and sets the environment variables below.
#![cfg(feature = "protes_integration_tests")]

use ga4gh_sdk::clients::tes::models::ListTasksParams;
use ga4gh_sdk::clients::tes::{TesView, TES};
use ga4gh_sdk::clients::wait::{WaitOptions, WaitOutcome};
use ga4gh_sdk::reqwest::StatusCode;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::HttpError;
use serde_json::{json, Value};
use std::time::Duration;
use url::Url;

/// Time limit for a task to finish, including the image pull of Funnel.
const TASK_BUDGET: Duration = Duration::from_secs(180);
/// Tag that marks the tasks of this suite, for the list filter.
const SUITE_TAG: &str = "sdk.suite";

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is not set; run this suite with tests/run-protes-integration.sh")
    })
}

/// Configuration of the API root of proTES.
fn config() -> Configuration {
    let base_path = Url::parse(&env("PROTES_TES_URL")).expect("PROTES_TES_URL is not a URL");
    Configuration::new(base_path)
}

/// Client of a user of proTES.
async fn authorized_tes() -> TES {
    tes_with_token(&env("PROTES_ACCESS_TOKEN")).await
}

/// Client that sends an access token, from the configuration.
async fn tes_with_token(token: &str) -> TES {
    TES::new(&config().with_oauth_access_token(token.into()))
        .await
        .expect("TES client")
}

/// A task without inputs or outputs, which needs no storage.
fn task(name: &str, command: &[&str]) -> Value {
    json!({
        "name": name,
        "tags": {SUITE_TAG: name},
        "executors": [{"image": "alpine:3.22", "command": command}],
    })
}

#[tokio::test]
async fn task_runs_to_completion_through_the_gateway() {
    let tes = authorized_tes().await;
    let task = tes
        .create_value(task("complete", &["echo", "hello from proTES"]))
        .await
        .expect("task creation");

    let result = task.wait(&WaitOptions::new(TASK_BUDGET)).await;
    assert_eq!(result.outcome, WaitOutcome::Complete, "{result:?}");

    let full = tes
        .get(TesView::Full, &task.id)
        .await
        .expect("task in view FULL");
    let tags = full.tags.expect("tags of the task");
    // proTES records the TES instance that runs a task in this tag
    assert_eq!(
        tags.get("gateway.backend").map(String::as_str),
        Some(env("PROTES_BACKEND_URL").as_str()),
        "proTES did not relay the task to Funnel: {tags:?}"
    );
    let stdout = full
        .logs
        .as_deref()
        .and_then(|logs| logs.last())
        .and_then(|log| log.logs.first())
        .and_then(|log| log.stdout.as_deref());
    assert_eq!(stdout, Some("hello from proTES\n"));

    let listed = tes
        .list_tasks(Some(ListTasksParams {
            name_prefix: None,
            state: None,
            tag_key: Some(vec![SUITE_TAG.into()]),
            tag_value: Some(vec!["complete".into()]),
            page_size: None,
            page_token: None,
            view: Some(TesView::Minimal),
        }))
        .await
        .expect("task list");
    assert!(
        listed
            .tasks
            .iter()
            .any(|listed| listed.id.as_deref() == Some(task.id.as_str())),
        "task {} not listed: {listed:?}",
        task.id
    );
}

#[tokio::test]
async fn canceled_task_ends_canceled() {
    let tes = authorized_tes().await;
    let task = tes
        .create_value(task("cancel", &["sleep", "600"]))
        .await
        .expect("task creation");

    let result = task
        .cancel_and_wait(&WaitOptions::new(TASK_BUDGET))
        .await
        .expect("cancellation");
    assert_eq!(result.outcome, WaitOutcome::Canceled, "{result:?}");
}

/// Asserts that a request failed with HTTP 401.
fn assert_unauthorized(error: &(dyn std::error::Error + 'static)) {
    let status = error.downcast_ref::<HttpError>().map(|error| error.status);
    assert_eq!(status, Some(StatusCode::UNAUTHORIZED), "{error}");
}

#[tokio::test]
async fn requests_without_valid_access_token_are_unauthorized() {
    // The service info is public; tasks are not
    let anonymous = TES::new(&config()).await.expect("TES client");
    let invalid = tes_with_token("not-a-token").await;

    for tes in [anonymous, invalid] {
        let error = tes.list_tasks(None).await.expect_err("list");
        assert_unauthorized(error.as_ref());
        let error = tes
            .create_value(task("unauthorized", &["true"]))
            .await
            .expect_err("creation");
        assert_unauthorized(error.as_ref());
    }
}
