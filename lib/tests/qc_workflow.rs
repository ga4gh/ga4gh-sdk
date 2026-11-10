#![cfg(feature = "workflow-contracts")]
use ga4gh_sdk::{
    clients::{
        wait::{WaitOptions, WaitOutcome},
        wes::{models::WesRunRequest, WorkflowAttachment, WES},
    },
    utils::configuration::Configuration,
    workflow::{
        compare::{ComparisonPolicy, Outcome},
        digest,
        results::{ExpectedChecksum, IntegrityStatus},
        InvocationIdentity, Pins, Profile,
    },
};
use serde_json::{json, Value};
use std::time::Duration;
#[tokio::test]
#[ignore = "requires two explicit Toil WES endpoints and a Docker-capable runner; see tests/run-phase2-qc.py"]
async fn real_qc_on_two_wes_instances() {
    let endpoints = std::env::var("GA4GH_QC_WES_URLS").expect("two comma-separated WES endpoints");
    let endpoints = endpoints.split(',').collect::<Vec<_>>();
    assert_eq!(endpoints.len(), 2);
    let schema = include_bytes!("../../examples/qc-demo/profile.json");
    let descriptor = include_bytes!("../../examples/qc-demo/main.cwl");
    let p = Profile::from_slice(schema, Pins::reference()).unwrap();
    let mut records = vec![];
    let mut evidence = vec![];
    for (index, endpoint) in endpoints.iter().enumerate() {
        let config = Configuration::new(url::Url::parse(endpoint).unwrap());
        let wes = WES::new(&config).await.unwrap();
        let request: WesRunRequest =
            serde_json::from_slice(include_bytes!("../../examples/qc-demo/request.json")).unwrap();
        let input_digest = digest(&serde_json::to_vec(&request.workflow_params).unwrap());
        let prepared = p
            .prepare(
                request,
                InvocationIdentity {
                    workflow: format!("public-sequence-qc@sha256:{}", digest(descriptor)),
                    descriptor_digest: Some(format!("sha256:{}", digest(descriptor))),
                },
            )
            .unwrap();
        let run = prepared
            .submit_with_attachments(
                &wes,
                vec![WorkflowAttachment {
                    filename: "main.cwl".into(),
                    content: descriptor.to_vec(),
                }],
            )
            .await
            .unwrap();
        let mut options = WaitOptions::new(Duration::from_secs(180));
        options.fetch_failure_logs = true;
        let wait = run.wait(&options).await;
        assert_eq!(wait.outcome, WaitOutcome::Complete, "{wait:?}");
        let log: Value = serde_json::from_str(
            &wes.transport
                .get(&format!("runs/{}", run.id), None)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(log["state"], "COMPLETE");
        let outputs = &log["outputs"];
        assert_eq!(outputs["metrics"]["sequence_count"], 3);
        assert_eq!(outputs["metrics"]["total_length"], 12);
        assert_eq!(outputs["metrics"]["mean_length"], 4.0);
        let mut result = p
            .project_results(
                &format!("local-node-{index}/{}", run.id),
                "COMPLETE",
                outputs,
            )
            .unwrap();
        assert!(
            result.contract_diagnostics.is_empty(),
            "{:?}",
            result.contract_diagnostics
        );
        let path = url::Url::parse(outputs["report"]["location"].as_str().unwrap())
            .unwrap()
            .to_file_path()
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let golden=b"<!doctype html><title>Sequence QC</title><h1>Sequence QC</h1><pre>{&quot;mean_length&quot;: 4.0, &quot;sequence_count&quot;: 3, &quot;total_length&quot;: 12}</pre>";
        assert_eq!(bytes, golden);
        result
            .verify_bytes(
                "/report/location",
                &bytes,
                Some(ExpectedChecksum {
                    algorithm: "sha256".into(),
                    digest: digest(golden),
                    evidence_source: "public QC golden report v1".into(),
                }),
            )
            .unwrap();
        assert_eq!(
            result.artifacts["/report/location"].integrity_status,
            IntegrityStatus::Verified
        );
        result
            .context
            .insert("workflow_digest".into(), digest(descriptor));
        result.context.insert("input_digest".into(), input_digest);
        result.context.insert(
            "image".into(),
            "python@sha256:6771159cd4fa5d9bba1258caf0b82e6b73458c694d178ad97c5e925c2d0e1a91".into(),
        );
        result
            .context
            .insert("backend".into(), format!("local-toil-{index}"));
        evidence.push(json!({"prepared":prepared,"run_log":log,"result":result,"viewer_metadata":result.html_viewer_metadata()}));
        records.push(result);
    }
    let policy: ComparisonPolicy =
        serde_json::from_slice(include_bytes!("../../examples/qc-demo/policy.json")).unwrap();
    let comparison = policy.compare(&records[0], &records[1]).unwrap();
    assert_eq!(comparison.outcome, Outcome::EquivalentUnderPolicy);
    assert!(comparison.context_drift.contains_key("backend"));
    let mut changed = records[1].clone();
    changed
        .context
        .insert("input_digest".into(), "changed-input".into());
    let drift = policy.compare(&records[0], &changed).unwrap();
    assert_eq!(drift.outcome, Outcome::Inconclusive);
    let mut missing = records[1].clone();
    missing.outputs.remove("metrics");
    assert_eq!(
        policy.compare(&records[0], &missing).unwrap().outcome,
        Outcome::Inconclusive
    );
    if let Ok(path) = std::env::var("GA4GH_QC_EVIDENCE") {
        std::fs::write(path,serde_json::to_vec_pretty(&json!({"deployment":"two local Toil WES processes; no federation claim","executions":evidence,"comparison":comparison,"changed_identity":drift})).unwrap()).unwrap();
    }
}
