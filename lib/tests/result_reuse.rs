#![cfg(feature = "workflow-contracts")]
use ga4gh_sdk::workflow::{
    digest,
    reuse::{
        AccessFailure, ArtifactRead, ArtifactReference, ContentIdentity, DependencyUse,
        DeterminismContract, ExecutionOrigin, ReasonCode, ReceiptTrust, RecipeIdentity,
        ReferenceIdentity, ReuseReceipt, ReuseReport, ReuseStatus, TypedValue,
    },
    Pins, Profile,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};

fn receipt() -> ReuseReceipt {
    let profile = Profile::from_slice(&serde_json::to_vec(&json!({
        "$schema":"https://json-schema.org/draft/2020-12/schema", "type":"object",
        "properties":{
            "inputs":{"type":"object"},
            "outputs":{"type":"object", "properties":{"report":{"type":"string","format":"uri"}}, "required":["report"]}
        }
    })).unwrap(), Pins::reference()).unwrap();
    let recipe = RecipeIdentity {
        version: 1,
        determinism: DeterminismContract {
            opted_in: true,
            author: "reference-preprocess author".into(),
            revision: "1".into(),
            network: DependencyUse::None,
            secrets: DependencyUse::None,
            other_nondeterminism: false,
        },
        executable_sha256: digest(b"script"),
        container_image: format!("python@sha256:{}", digest(b"container")),
        command: vec!["python".into(), "preprocess.py".into(), "--k=3".into()],
        environment: BTreeMap::from([("LC_ALL".into(), "C".into())]),
        configuration: json!({"sort":"lexical","nested":{"b":2,"a":1}}),
        parameters: BTreeMap::from([("k".into(), TypedValue::Unsigned(3))]),
        inputs: BTreeMap::from([(
            "inputs/reference.fa".into(),
            ContentIdentity {
                sha256: digest(b">test\nACGT\n"),
                size_bytes: 11,
            },
        )]),
        reference_versions: BTreeMap::from([(
            "reference".into(),
            ReferenceIdentity {
                version: "synthetic-v1".into(),
                sha256: digest(b">test\nACGT\n"),
            },
        )]),
        output_schema_sha256: profile.schema_digest().into(),
        output_paths: BTreeMap::from([("/report".into(), "results/kmers.json".into())]),
        runtime_compatibility: BTreeMap::from([
            ("os".into(), "linux".into()),
            ("architecture".into(), "aarch64".into()),
            ("runtime".into(), "CPython-3.11".into()),
        ]),
        invocation_fingerprint: None,
    };
    let result = profile
        .project_results(
            "https://a.example/tes/v1/tasks/original-1",
            "COMPLETE",
            &json!({"report":"drs://a.example/kmers"}),
        )
        .unwrap();
    ReuseReceipt::new(
        recipe,
        ExecutionOrigin {
            service: "https://a.example/tes/v1".into(),
            run_id: "original-1".into(),
        },
        "authorized-provider-a-export".into(),
        result,
        BTreeMap::from([(
            "/report".into(),
            ArtifactReference {
                relative_path: "results/kmers.json".into(),
                stable_reference: "drs://a.example/kmers".into(),
                size_bytes: 4,
                sha256: digest(b"data"),
            },
        )]),
    )
    .unwrap()
}
fn trust(receipt: &ReuseReceipt) -> ReceiptTrust {
    ReceiptTrust::Pinned {
        receipt_sha256: receipt.content_digest().unwrap(),
        original_service: receipt.original.service.clone(),
        evidence_source: "authorized workspace import".into(),
    }
}
fn read(_: &ArtifactReference) -> Result<ArtifactRead, AccessFailure> {
    Ok(ArtifactRead {
        bytes: b"data".to_vec(),
        bytes_transferred: 0,
    })
}
fn reason(report: &ReuseReport, code: ReasonCode) {
    assert!(report.reasons.iter().any(|r| r.code == code), "{report:?}");
}
#[test]
fn import_export_preserves_typed_record_paths_sizes_original_and_fresh_evidence() {
    let receipt = receipt();
    let imported = ReuseReceipt::import(&receipt.export().unwrap()).unwrap();
    assert_eq!(
        receipt.content_digest().unwrap(),
        imported.content_digest().unwrap()
    );
    let reused = imported
        .reuse_for(
            "https://b.example/tes/v1",
            &receipt.recipe,
            &trust(&receipt),
            32,
            read,
        )
        .unwrap();
    assert_eq!(reused.original.run_id, "original-1");
    assert_eq!(reused.original.service, "https://a.example/tes/v1");
    assert_eq!(reused.result.run_reference, receipt.result.run_reference);
    assert_eq!(
        reused.artifacts["/report"].relative_path,
        "results/kmers.json"
    );
    assert_eq!(reused.measurements.bytes_revalidated, 4);
    assert_eq!(reused.measurements.bytes_transferred, 0);
    assert_eq!(
        reused.result.artifacts["/report"]
            .computed_sha256
            .as_deref(),
        Some(digest(b"data").as_str())
    );
    assert!(serde_json::to_value(reused)
        .unwrap()
        .get("new_run_id")
        .is_none());
}
#[test]
fn every_execution_relevant_change_invalidates_without_reading_artifacts() {
    let original = receipt();
    let mut recipes = vec![];
    let mut changed = original.recipe.clone();
    changed.inputs.values_mut().next().unwrap().sha256 = digest(b"changed");
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.inputs.values_mut().next().unwrap().size_bytes += 1;
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    let input = changed.inputs.remove("inputs/reference.fa").unwrap();
    changed.inputs.insert("inputs/renamed.fa".into(), input);
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed
        .parameters
        .insert("k".into(), TypedValue::Unsigned(4));
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed
        .parameters
        .insert("k".into(), TypedValue::String("3".into()));
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.executable_sha256 = digest(b"new binary");
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.container_image = format!("python@sha256:{}", digest(b"new image"));
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed
        .reference_versions
        .get_mut("reference")
        .unwrap()
        .version = "v2".into();
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed
        .output_paths
        .insert("/report".into(), "renamed/report.json".into());
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed
        .runtime_compatibility
        .insert("architecture".into(), "x86_64".into());
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.environment.insert("TZ".into(), "UTC".into());
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.configuration["sort"] = json!("other");
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.command.push("--other".into());
    recipes.push(changed);
    let mut changed = original.recipe.clone();
    changed.output_schema_sha256 = digest(b"new schema");
    recipes.push(changed);
    for changed in recipes {
        assert_ne!(changed.fingerprint().unwrap(), original.recipe_fingerprint);
        let report = original.assess(&changed, &trust(&original), 32, |_| {
            panic!("must not read for a different task")
        });
        assert_eq!(report.status, ReuseStatus::Ineligible);
        reason(&report, ReasonCode::RecipeChanged);
    }
}
#[test]
fn denial_missing_corrupt_size_and_unknown_access_remain_distinct() {
    let receipt = receipt();
    let trust = trust(&receipt);
    for (access, code, status) in [
        (
            AccessFailure::Denied,
            ReasonCode::ArtifactAccessDenied,
            ReuseStatus::Ineligible,
        ),
        (
            AccessFailure::Missing,
            ReasonCode::ArtifactMissing,
            ReuseStatus::Ineligible,
        ),
        (
            AccessFailure::Unverified,
            ReasonCode::ArtifactAccessUnverified,
            ReuseStatus::Unverified,
        ),
    ] {
        let report = receipt
            .reuse_for("provider-b", &receipt.recipe, &trust, 32, |_| {
                Err(access.clone())
            })
            .unwrap_err();
        assert_eq!(report.status, status);
        reason(&report, code);
    }
    let report = receipt.assess(&receipt.recipe, &trust, 32, |_| {
        Ok(ArtifactRead {
            bytes: b"evil".to_vec(),
            bytes_transferred: 4,
        })
    });
    reason(&report, ReasonCode::ArtifactDigestMismatch);
    assert_eq!(report.measurements.bytes_revalidated, 0);
    assert_eq!(report.measurements.bytes_transferred, 4);
    reason(
        &receipt.assess(&receipt.recipe, &trust, 32, |_| {
            Ok(ArtifactRead {
                bytes: b"longer".to_vec(),
                bytes_transferred: 0,
            })
        }),
        ReasonCode::ArtifactSizeMismatch,
    );
}
#[test]
fn explicit_reuse_rechecks_current_access_after_an_eligible_assessment() {
    let receipt = receipt();
    let trust = trust(&receipt);
    let reads = AtomicUsize::new(0);
    let report = receipt.assess(&receipt.recipe, &trust, 32, |a| {
        reads.fetch_add(1, Ordering::SeqCst);
        read(a)
    });
    assert_eq!(report.status, ReuseStatus::Eligible);
    let denied = receipt
        .reuse_for("provider-b", &receipt.recipe, &trust, 32, |_| {
            reads.fetch_add(1, Ordering::SeqCst);
            Err(AccessFailure::Denied)
        })
        .unwrap_err();
    reason(&denied, ReasonCode::ArtifactAccessDenied);
    assert_eq!(reads.load(Ordering::SeqCst), 2);
}
#[test]
fn receipt_claims_and_matching_digests_cannot_supply_their_own_trust() {
    let receipt = receipt();
    let unknown = receipt.assess(&receipt.recipe, &ReceiptTrust::Unknown, 32, |_| {
        panic!("must establish trust first")
    });
    assert_eq!(unknown.status, ReuseStatus::Unverified);
    reason(&unknown, ReasonCode::TrustUnknown);
    let untrusted = receipt.assess(&receipt.recipe, &ReceiptTrust::Untrusted, 32, read);
    assert_eq!(untrusted.status, ReuseStatus::Ineligible);
    reason(&untrusted, ReasonCode::SourceUntrusted);
    let mut tampered = receipt.clone();
    tampered.original.run_id = "forged".into();
    reason(
        &tampered.assess(&receipt.recipe, &trust(&receipt), 32, read),
        ReasonCode::ReceiptNotTrusted,
    );
    let mut tampered = receipt.clone();
    tampered.recipe.command.push("--forged".into());
    reason(
        &tampered.assess(&tampered.recipe, &trust(&tampered), 32, read),
        ReasonCode::FingerprintMismatch,
    );
}
#[test]
fn malformed_stochastic_incomplete_and_unsafe_layouts_never_reuse() {
    let original = receipt();
    for dependency in [DependencyUse::Dependent, DependencyUse::Undeclared] {
        let mut changed = original.clone();
        changed.recipe.determinism.network = dependency.clone();
        reason(
            &changed.assess(&changed.recipe, &trust(&changed), 32, read),
            ReasonCode::NondeterministicDependency,
        );
        changed.recipe.determinism.network = DependencyUse::None;
        changed.recipe.determinism.secrets = dependency;
        reason(
            &changed.assess(&changed.recipe, &trust(&changed), 32, read),
            ReasonCode::NondeterministicDependency,
        );
    }
    let mut changed = original.clone();
    changed.recipe.determinism.other_nondeterminism = true;
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::NondeterministicDependency,
    );
    let mut changed = original.clone();
    changed.recipe.determinism.opted_in = false;
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::RecipeNotOptedIn,
    );
    let mut changed = original.clone();
    changed.result.execution_state = "RUNNING".into();
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::NotComplete,
    );
    let mut changed = original.clone();
    changed.result.schema_digest = digest(b"wrong");
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::InvalidOutputs,
    );
    let mut changed = original.clone();
    changed.recipe.container_image = "python:latest".into();
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::InvalidIdentity,
    );
    let mut changed = original.clone();
    changed
        .recipe
        .output_paths
        .insert("/report".into(), "../escape".into());
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::InvalidIdentity,
    );
    let mut changed = original.clone();
    changed
        .artifacts
        .get_mut("/report")
        .unwrap()
        .stable_reference = "https://x.example/object?secret=credential".into();
    reason(
        &changed.assess(&changed.recipe, &trust(&changed), 32, read),
        ReasonCode::ArtifactManifestMismatch,
    );
    reason(
        &original.assess(&original.recipe, &trust(&original), 3, |_| {
            panic!("manifest exceeds bound")
        }),
        ReasonCode::ArtifactTooLarge,
    );
}

/// Opt-in real TES adapters: provider A runs one pinned CPU container; B accepts no task.
#[test]
#[ignore = "run examples/result-reuse/run.py with a cached image and Docker context"]
fn real_local_result_reuse() {
    use ga4gh_sdk::{
        clients::{
            drs::{models::access_method::Type, uri::AccessPlan},
            tes::{
                models::{TesState, TesTask},
                TES,
            },
        },
        utils::{configuration::Configuration, operation::Operation},
    };
    use std::{path::PathBuf, time::Duration};
    let providers =
        std::env::var("GA4GH_REUSE_PROVIDERS").expect("use examples/result-reuse/run.py");
    let roots: Vec<_> = providers.split(',').collect();
    assert_eq!(roots.len(), 2);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let http = ga4gh_sdk::reqwest::Client::builder()
        .redirect(ga4gh_sdk::reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap();
    let api_a = format!("{}/ga4gh/tes/v1", roots[0]);
    let api_b = format!("{}/ga4gh/tes/v1", roots[1]);
    let a = runtime
        .block_on(TES::new(&Configuration::new(
            url::Url::parse(&api_a).unwrap(),
        )))
        .unwrap();
    let b = runtime
        .block_on(TES::new(&Configuration::new(
            url::Url::parse(&api_b).unwrap(),
        )))
        .unwrap();
    let task:TesTask = serde_json::from_value(json!({
        "name":"deterministic-synthetic-reference-kmers", "executors":[{
            "image":"python@sha256:6771159cd4fa5d9bba1258caf0b82e6b73458c694d178ad97c5e925c2d0e1a91",
            "command":["python","/workspace/preprocess.py","/workspace/inputs/reference.fa","/workspace/results/kmers.json","3"],
            "env":{"LC_ALL":"C","PYTHONHASHSEED":"0"}
        }],
        "inputs":[{"url":format!("{}/fixtures/reference.fa",roots[0]),"path":"/workspace/inputs/reference.fa","type":"FILE"}],
        "outputs":[{"url":format!("{}/artifacts/kmers.json",roots[0]),"path":"/workspace/results/kmers.json","type":"FILE"}]
    })).unwrap();
    let run = runtime.block_on(a.create(task.clone())).unwrap();
    assert_eq!(runtime.block_on(run.status()).unwrap(), TesState::Complete);
    let manifest: serde_json::Value = runtime.block_on(async {
        http.get(format!("{}/manifest", roots[0]))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap()
    });
    assert_eq!(
        manifest["sha256"],
        digest(include_bytes!(
            "../../examples/result-reuse/expected-kmers.json"
        ))
    );
    let profile = Profile::from_slice(
        include_bytes!("../../examples/result-reuse/profile.json"),
        Pins::reference(),
    )
    .unwrap();
    let mut identity = receipt().recipe;
    identity.determinism = DeterminismContract {
        opted_in: true,
        author: "SDK synthetic reference preprocessing recipe author".into(),
        revision: "examples/result-reuse/preprocess.py:v1".into(),
        network: DependencyUse::None,
        secrets: DependencyUse::None,
        other_nondeterminism: false,
    };
    identity.executable_sha256 =
        digest(include_bytes!("../../examples/result-reuse/preprocess.py"));
    assert_eq!(manifest["executable_sha256"], identity.executable_sha256);
    identity.container_image = manifest["image"].as_str().unwrap().into();
    identity.command = task.executors.as_ref().unwrap()[0].command.clone();
    identity.environment = BTreeMap::from([
        ("LC_ALL".into(), "C".into()),
        ("PYTHONHASHSEED".into(), "0".into()),
    ]);
    identity.configuration = json!({"sequence_boundaries":"separate", "alphabet":"uppercase-ACGT", "sort":"ASCII", "working_directory":"/workspace"});
    identity.inputs = BTreeMap::from([(
        "inputs/reference.fa".into(),
        ContentIdentity {
            sha256: digest(include_bytes!("../../examples/result-reuse/reference.fa")),
            size_bytes: include_bytes!("../../examples/result-reuse/reference.fa").len() as u64,
        },
    )]);
    identity.reference_versions = BTreeMap::from([(
        "reference".into(),
        ReferenceIdentity {
            version: "synthetic-v1".into(),
            sha256: digest(include_bytes!("../../examples/result-reuse/reference.fa")),
        },
    )]);
    identity.output_schema_sha256 = profile.schema_digest().into();
    identity.output_paths = BTreeMap::from([("/histogram".into(), "results/kmers.json".into())]);
    identity.runtime_compatibility = serde_json::from_value(manifest["runtime"].clone()).unwrap();
    assert_eq!(identity.runtime_compatibility["os"], "linux");
    assert_eq!(identity.runtime_compatibility["runtime"], "CPython-3.13");
    assert!(
        ["aarch64", "x86_64"].contains(&identity.runtime_compatibility["architecture"].as_str())
    );
    assert_eq!(
        manifest["runtime"],
        serde_json::to_value(&identity.runtime_compatibility).unwrap()
    );
    let artifact_url = format!("{}/artifacts/kmers.json", roots[0]);
    let plan = AccessPlan {
        object_id: "synthetic-kmers".into(),
        method: Type::Https,
        region: None,
        url: url::Url::parse(&artifact_url).unwrap(),
        headers: Default::default(),
    };
    let mut result = profile
        .project_results(
            &format!("{api_a}/tasks/{}", run.id),
            "COMPLETE",
            &json!({"histogram":artifact_url}),
        )
        .unwrap();
    // Compose the existing result verifier and selected access-plan contract.
    runtime
        .block_on(result.verify_access_plan(
            "/histogram",
            &plan,
            Some(ga4gh_sdk::workflow::results::ExpectedChecksum {
                algorithm: "sha256".into(),
                digest: manifest["sha256"].as_str().unwrap().into(),
                evidence_source: "authorized local provider A manifest".into(),
            }),
            &Operation::new(Duration::from_secs(5)),
            65536,
        ))
        .unwrap();
    let produced = ReuseReceipt::new(
        identity.clone(),
        ExecutionOrigin {
            service: api_a.clone(),
            run_id: run.id.clone(),
        },
        "authorized-local-provider-a-export".into(),
        result,
        BTreeMap::from([(
            "/histogram".into(),
            ArtifactReference {
                relative_path: "results/kmers.json".into(),
                stable_reference: artifact_url,
                size_bytes: manifest["size_bytes"].as_u64().unwrap(),
                sha256: manifest["sha256"].as_str().unwrap().into(),
            },
        )]),
    )
    .unwrap();
    let evidence_path = PathBuf::from(std::env::var("GA4GH_REUSE_EVIDENCE").unwrap());
    let receipt_path = evidence_path.with_file_name("receipt.json");
    std::fs::write(&receipt_path, produced.export().unwrap()).unwrap();
    let imported = ReuseReceipt::import(&std::fs::read(&receipt_path).unwrap()).unwrap();
    // A user-controlled local workspace authorizes this exact export, independently
    // of any trust claim carried by the imported receipt itself.
    let current_trust = trust(&produced);
    let stats = |root: &str| {
        runtime.block_on(async {
            http.get(format!("{root}/control"))
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        })
    };
    let before_a = stats(roots[0]);
    let before_b = stats(roots[1]);
    let reader = |artifact: &ArtifactReference| {
        runtime.block_on(async {
            let response = http
                .get(&artifact.stable_reference)
                .send()
                .await
                .map_err(|_| AccessFailure::Unverified)?;
            match response.status().as_u16() {
                403 | 401 => return Err(AccessFailure::Denied),
                404 => return Err(AccessFailure::Missing),
                200 => (),
                _ => return Err(AccessFailure::Unverified),
            }
            let bytes = ga4gh_sdk::utils::transport::read_bounded(response, 65536)
                .await
                .map_err(|_| AccessFailure::Unverified)?
                .body;
            let bytes_transferred = bytes.len() as u64;
            Ok(ArtifactRead {
                bytes,
                bytes_transferred,
            })
        })
    };
    let reused = imported
        .reuse_for(&api_b, &identity, &current_trust, 65536, reader)
        .unwrap();
    assert_eq!(reused.original.service, api_a);
    assert_eq!(reused.original.run_id, run.id);
    let after_a = stats(roots[0]);
    let after_b = stats(roots[1]);
    assert_eq!(before_a["container_executions"], 1);
    assert_eq!(after_a["container_executions"], 1);
    assert_eq!(before_b["accepted_tasks"], 0);
    assert_eq!(after_b["accepted_tasks"], 0);
    assert_eq!(runtime.block_on(b.list_tasks(None)).unwrap().tasks.len(), 0);
    let payload_delta = after_a["artifact_payload_bytes_sent"].as_u64().unwrap()
        - before_a["artifact_payload_bytes_sent"].as_u64().unwrap();
    assert_eq!(payload_delta, reused.measurements.bytes_transferred);
    assert_eq!(payload_delta, reused.measurements.bytes_revalidated);
    let mut changed = identity.clone();
    changed
        .parameters
        .insert("k".into(), TypedValue::Unsigned(4));
    let changed = imported
        .reuse_for(&api_b, &changed, &current_trust, 65536, reader)
        .unwrap_err();
    reason(&changed, ReasonCode::RecipeChanged);
    let control = |mode: &str| {
        runtime.block_on(async {
            http.post(format!("{}/control", roots[0]))
                .json(&json!({"mode":mode}))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
        })
    };
    control("revoked");
    let denied = imported
        .reuse_for(&api_b, &identity, &current_trust, 65536, reader)
        .unwrap_err();
    reason(&denied, ReasonCode::ArtifactAccessDenied);
    control("corrupt");
    let corrupt = imported
        .reuse_for(&api_b, &identity, &current_trust, 65536, reader)
        .unwrap_err();
    reason(&corrupt, ReasonCode::ArtifactDigestMismatch);
    control("missing");
    let missing = imported
        .reuse_for(&api_b, &identity, &current_trust, 65536, reader)
        .unwrap_err();
    reason(&missing, ReasonCode::ArtifactMissing);
    control("available");
    let untrusted = imported
        .reuse_for(&api_b, &identity, &ReceiptTrust::Untrusted, 65536, reader)
        .unwrap_err();
    reason(&untrusted, ReasonCode::SourceUntrusted);
    let evidence = json!({
        "evidence_kind":"real_local_single_container_tes", "image":manifest["image"],"runtime":manifest["runtime"],
        "receipt_path":receipt_path,"provider_a_before_substitution":before_a,"provider_a_after_substitution":after_a,"provider_b_before":before_b,"provider_b_after":after_b,
        "reused_result":reused,"executions_avoided":1,"basis":"one prepared provider B task replaced; measured B accepted_tasks delta=0 and A container_executions=1",
        "reuse_artifact_payload_bytes_sent":payload_delta,"changed_parameter":changed,"revoked_access":denied,"corrupt_artifact":corrupt,"deleted_artifact":missing,"untrusted_source":untrusted,
        "final_provider_a":stats(roots[0]),"final_provider_b":stats(roots[1]),"boundary":"two cooperating loopback TES adapters; not a cloud provider, federated identity or production TES conformance test"
    });
    std::fs::write(
        &evidence_path,
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    println!("one container execution, zero provider B tasks; {} bytes revalidated/transferred; evidence {}",payload_delta,evidence_path.display());
}
