//! Opt-in real-provider smoke check used by tests/interop/run.py.
//! This small test exercises SDK calls; the broader proTES suite stays upstream.
use ga4gh_sdk::clients::tes::{models::TesState, models::TesTask, Task, TES};
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::json;
use std::time::Duration;

async fn state(task: &Task, expected: TesState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        let actual = task.status().await.expect("observe owned task");
        if actual == expected {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "expected {expected:?}; observed {actual:?}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
async fn submit(tes: &TES, script: &str) -> Task {
    let task: TesTask = serde_json::from_value(json!({
        "name":"ga4gh-sdk-provider-profile",
        "inputs":[{"path":"/input/seed.txt","content":"GA4GH interoperability fixture v1\n","type":"FILE"}],
        "executors":[{"image":"alpine:3.22@sha256:2e0b81d60597f449baa56dc25d355367869d2dca4cd57e866f1e2b52af3eb55f",
                      "command":["sh","-c",script]}]
    })).unwrap();
    tes.create(task).await.expect("acknowledge real TES task")
}
#[tokio::test]
#[ignore = "requires an operator-owned disposable real TES profile; see docs/provider-profiles.md"]
async fn real_direct_tes_complete_failure_cancel() {
    let config = Configuration::new(
        url::Url::parse(&std::env::var("TES_PROFILE_API_ROOT").unwrap()).unwrap(),
    );
    let tes = TES::new(&config).await.expect("discover real TES");
    let completed = submit(
        &tes,
        "cat /input/seed.txt; echo profile-stdout; echo profile-stderr >&2",
    )
    .await;
    state(&completed, TesState::Complete).await;
    let log = tes.get_raw("FULL", &completed.id).await.unwrap();
    let log: serde_json::Value = serde_json::from_str(&log).unwrap();
    assert_eq!(log["logs"][0]["logs"][0]["exit_code"], 0);
    assert!(log["logs"][0]["logs"][0]["stdout"]
        .as_str()
        .unwrap()
        .contains("GA4GH interoperability fixture v1"));
    assert!(log["logs"][0]["logs"][0]["stderr"]
        .as_str()
        .unwrap()
        .contains("profile-stderr"));
    let failed = submit(&tes, "echo profile-failure >&2; exit 3").await;
    state(&failed, TesState::ExecutorError).await;
    let running = submit(&tes, "echo profile-started; sleep 30").await;
    state(&running, TesState::Running).await;
    running.cancel().await.expect("request cancel");
    state(&running, TesState::Canceled).await;
    // A TES CANCELED assertion is deliberately not a worker-termination claim.
}
