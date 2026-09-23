#![cfg(feature = "wes_docker_integration_tests")]

use ga4gh_sdk::clients::wes::models::{WesRunRequest, WesState};
use ga4gh_sdk::clients::wes::{Run, WES};
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::json;
use std::path::Path;
use std::time::Duration;
use tokio::time::{sleep, timeout, Instant};
use url::Url;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

fn request(workflow_url: &str, sleep_seconds: u64) -> WesRunRequest {
    WesRunRequest {
        workflow_params: Some(json!({
            "sleep_seconds": sleep_seconds,
            "message": "sdk-integration",
        })),
        workflow_type: Some("NEXTFLOW".into()),
        workflow_type_version: Some("21.04.0".into()),
        workflow_url: Some(workflow_url.into()),
        ..Default::default()
    }
}

async fn wait_for_completion(run: &Run, deadline: Instant) {
    let mut last_observation = String::from("no status received");
    while Instant::now() < deadline {
        match timeout(REQUEST_TIMEOUT, run.status()).await {
            Ok(Ok(WesState::Complete)) => return,
            Ok(Ok(WesState::ExecutorError | WesState::SystemError | WesState::Canceled)) => {
                panic!("run {} failed: {:?}", run.id, run.log().await);
            }
            Ok(Ok(state)) => last_observation = state.to_string(),
            Ok(Err(err)) => last_observation = err.to_string(),
            Err(_) => last_observation = "status request timed out".into(),
        }
        sleep(Duration::from_secs(2)).await;
    }
    panic!(
        "run {} did not complete before the deadline; last observation: {last_observation}",
        run.id
    );
}

fn has_task_start_marker(directory: &Path, depth: usize) -> bool {
    if depth == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name == ".command.begin")
        {
            return true;
        }
        if path.is_dir() && has_task_start_marker(&path, depth - 1) {
            return true;
        }
    }
    false
}

async fn wait_for_task_start(run: &Run, work_dir: &Path) {
    let id = &run.id;
    let job_dir = work_dir
        .join("wes_runs")
        .join(&id[0..2])
        .join(&id[2..4])
        .join(&id[4..6])
        .join(id);
    let deadline = Instant::now() + Duration::from_secs(90);
    while Instant::now() < deadline {
        if has_task_start_marker(&job_dir.join("work"), 4) {
            return;
        }
        sleep(Duration::from_secs(1)).await;
    }
    let log = std::fs::read_to_string(job_dir.join(".nextflow.log"))
        .unwrap_or_else(|err| format!("cannot read Nextflow log: {err}"));
    panic!(
        "run {} did not start a task before the deadline; Nextflow log:\n{}",
        id, log
    );
}

#[tokio::test]
async fn starter_kit_wes_101_client_round_trip() {
    let base = std::env::var("WES_DOCKER_BASE_URL")
        .expect("run through tests/run-wes-docker-integration.sh");
    let revision = std::env::var("WES_DOCKER_FIXTURE_REVISION")
        .expect("run through tests/run-wes-docker-integration.sh");
    let work_dir = std::path::PathBuf::from(
        std::env::var("WES_DOCKER_WORK_DIR")
            .expect("run through tests/run-wes-docker-integration.sh"),
    );
    let workflow_url = format!("https://github.com/ga4gh-sdk/wes-fixture/tree/{revision}");
    let run_timeout = std::env::var("WES_RUN_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(300);
    let config = Configuration::new(Url::parse(&base).expect("valid WES API base URL"));

    let wes = timeout(REQUEST_TIMEOUT, WES::new(&config))
        .await
        .expect("service discovery timed out")
        .expect("SDK could not discover Starter Kit WES");
    let service = wes.service.as_ref().expect("service info was parsed");
    assert_eq!(service.r#type.artifact, "wes");
    assert_eq!(service.r#type.version, "1.0.1");
    assert_eq!(service.id, "org.ga4gh.sdk.wes.integration");

    let run = timeout(REQUEST_TIMEOUT, wes.run_workflow(request(&workflow_url, 0)))
        .await
        .expect("RunWorkflow request timed out")
        .expect("SDK could not submit short workflow");
    assert!(!run.id.is_empty());
    println!("submitted short run {}", run.id);

    let listed = timeout(REQUEST_TIMEOUT, wes.list_runs(None, None))
        .await
        .expect("ListRuns request timed out")
        .expect("SDK could not parse Starter Kit run list");
    // Starter Kit 0.2.0 returns a hard-coded array, so it cannot list this run.
    assert!(listed.runs.is_some());

    wait_for_completion(&run, Instant::now() + Duration::from_secs(run_timeout)).await;
    let log = timeout(REQUEST_TIMEOUT, run.log())
        .await
        .expect("GetRunLog request timed out")
        .expect("SDK could not read completed run log");
    assert_eq!(log.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(log.state, Some(WesState::Complete));
    assert_eq!(
        log.request.as_ref().and_then(|r| r.workflow_url.as_deref()),
        Some(workflow_url.as_str())
    );
    assert_eq!(
        log.run_log.as_ref().and_then(|entry| entry.exit_code),
        Some(0)
    );
    let outputs = log
        .outputs
        .as_ref()
        .and_then(|value| value.as_object())
        .expect("completed run has an outputs object");
    let result_url = outputs
        .get("result.txt")
        .and_then(|value| value.as_str())
        .expect("completed workflow exposes result.txt");
    let result_path = Url::parse(result_url)
        .expect("output is a URL")
        .to_file_path()
        .expect("output is a local file URL");
    assert_eq!(
        std::fs::read_to_string(result_path)
            .expect("read workflow output")
            .trim(),
        "sdk-integration"
    );
    println!(
        "completed run {} with {} output entries",
        run.id,
        outputs.len()
    );

    // The same local fixture sleeps for two minutes in a separate run.
    let long_run = timeout(
        REQUEST_TIMEOUT,
        wes.run_workflow(request(&workflow_url, 120)),
    )
    .await
    .expect("long RunWorkflow request timed out")
    .expect("SDK could not submit long workflow");
    assert_ne!(long_run.id, run.id);
    println!("submitted long run {}", long_run.id);
    wait_for_task_start(&long_run, &work_dir).await;

    // Starter Kit 0.2.0's cancel controller returns null without canceling.
    // The SDK must report this clearly, rather than claim success.
    let cancel_error = timeout(REQUEST_TIMEOUT, long_run.cancel())
        .await
        .expect("CancelRun request timed out")
        .expect_err("Starter Kit 0.2.0 unexpectedly implemented cancellation");
    assert!(
        cancel_error
            .to_string()
            .contains("cancellation cannot be confirmed"),
        "{cancel_error}"
    );
    println!("Starter Kit cancellation limitation confirmed: {cancel_error}");
}
