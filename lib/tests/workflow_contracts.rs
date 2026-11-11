#![cfg(feature = "workflow-contracts")]
use ga4gh_sdk::clients::wes::models::RunRequest;
use ga4gh_sdk::workflow::{InvocationIdentity, Pins, Profile};
use serde_json::{json, Value};
fn schema() -> Value {
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "type":"object", "$defs":{"choice":{"type":"string","enum":["fast","full"]}}, "properties":{"inputs":{"type":"object","properties":{"mode":{"$ref":"#/$defs/choice"},"nested":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}},"required":["mode","nested"]},"outputs":{"type":"object","properties":{"count":{"type":"integer"},"report":{"type":"string","format":"uri","contentMediaType":"text/html","x-ga4gh-ui-widget":"html-viewer"}},"required":["count","report"]}}})
}
fn profile(value: Value) -> Profile {
    Profile::from_slice(&serde_json::to_vec(&value).unwrap(), Pins::reference()).unwrap()
}
#[test]
fn validates_root_refs_and_preserves_values_without_defaults() {
    let p = profile(schema());
    let params = json!({"mode":"fast","nested":{"n":4}});
    assert!(p.validate_inputs(&params).is_empty());
    let errors = p.validate_inputs(&json!({"mode":"slow","nested":{"n":"bad"}}));
    assert!(errors
        .iter()
        .any(|e| e.instance_location == "/mode" && e.schema_location.contains("enum")));
    assert!(errors.iter().any(|e| e.instance_location == "/nested/n"));
    let req = RunRequest {
        workflow_url: "workflow.cwl".into(),
        workflow_type: "CWL".into(),
        workflow_type_version: "v1.2".into(),
        workflow_params: Some(params.clone()),
        ..Default::default()
    };
    let prepared = p
        .prepare(
            req,
            InvocationIdentity {
                workflow: "qc@sha256:abc".into(),
                descriptor_digest: Some("sha256:descriptor".into()),
            },
        )
        .unwrap();
    assert_eq!(prepared.request().workflow_params, Some(params));
    assert!(!serde_json::to_value(prepared.request())
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("contract"));
    assert!(p
        .validate_inputs(&json!({}))
        .iter()
        .any(|e| e.message.contains("required")));
}
#[test]
fn rejects_unresolved_refs_unknown_required_vocab_and_invalid_schema() {
    let mut s = schema();
    s["properties"]["inputs"]["properties"]["mode"] =
        json!({"$ref":"https://missing.invalid/no.json"});
    assert!(Profile::from_slice(&serde_json::to_vec(&s).unwrap(), Pins::reference()).is_err());
    let mut s = schema();
    s["$vocabulary"] = json!({"https://unknown.invalid/vocab":true});
    assert!(Profile::from_slice(&serde_json::to_vec(&s).unwrap(), Pins::reference()).is_err());
    let mut s = schema();
    s["properties"]["inputs"]["type"] = json!(42);
    assert!(Profile::from_slice(&serde_json::to_vec(&s).unwrap(), Pins::reference()).is_err());
}
#[test]
fn consumes_pinned_reference_fixture_offline() {
    let p = Profile::from_slice(
        include_bytes!("../../tests/fixtures/io-profile/example_workflow.schema.json"),
        Pins::reference(),
    )
    .unwrap();
    assert!(p
        .validate_inputs(&json!({"sample_manifest":"https://example.org/samples.csv"}))
        .is_empty());
    assert!(!p.validate_inputs(&json!({})).is_empty());
}
use ga4gh_sdk::workflow::compare::{Comparator, ComparisonPolicy, Outcome};
use ga4gh_sdk::workflow::results::{
    AccessStatus, ContractStatus, ExpectedChecksum, IntegrityStatus,
};
use std::collections::BTreeMap;
#[test]
fn complete_state_remains_complete_when_delivery_missing_or_invalid() {
    let p = profile(schema());
    let record = p
        .project_results("wes/node/run1", "COMPLETE", &json!({"count":"bad"}))
        .unwrap();
    assert_eq!(record.execution_state, "COMPLETE");
    assert_eq!(
        record.outputs["count"].contract_status,
        ContractStatus::Invalid
    );
    assert_eq!(
        record.outputs["report"].contract_status,
        ContractStatus::Missing
    );
    assert!(!record.contract_diagnostics.is_empty());
    assert_eq!(record.html_viewer_metadata()[0]["widget"], "html-viewer");
}
#[test]
fn artifacts_distinguish_expected_changed_unverified_and_missing() {
    let p = profile(schema());
    let mut r = p
        .project_results(
            "one",
            "COMPLETE",
            &json!({"count":1,"report":"drs://example.org/report"}),
        )
        .unwrap();
    let expected = ExpectedChecksum {
        algorithm: "sha256".into(),
        digest: ga4gh_sdk::workflow::digest(b"original"),
        evidence_source: "pinned-manifest@abc".into(),
    };
    r.verify_bytes("/report", b"original", None).unwrap();
    assert_eq!(
        r.artifacts["/report"].integrity_status,
        IntegrityStatus::Unverified
    );
    r.verify_bytes("/report", b"changed", Some(expected.clone()))
        .unwrap();
    assert_eq!(
        r.artifacts["/report"].integrity_status,
        IntegrityStatus::Mismatch
    );
    r.verify_bytes("/report", b"original", Some(expected))
        .unwrap();
    assert_eq!(
        r.artifacts["/report"].integrity_status,
        IntegrityStatus::Verified
    );
    assert_eq!(
        r.artifacts["/report"].stable_identity.as_deref(),
        Some("drs://example.org/report")
    );
    r.verify_bytes("/missing", b"x", None).unwrap();
    assert_eq!(r.artifacts["/missing"].access_status, AccessStatus::Missing);
}
#[test]
fn durable_metadata_excludes_signed_urls_and_nested_preserves_values() {
    let p = profile(schema());
    let r=p.project_results("one","COMPLETE",&json!({"count":1,"report":"https://user:password@example.org/report?token=secret#cap"})).unwrap();
    let bytes = serde_json::to_string(&r).unwrap();
    assert!(!bytes.contains("password"));
    assert!(!bytes.contains("secret"));
    assert_eq!(
        r.value_at("/report"),
        Some(&json!("https://example.org/report"))
    );
}
fn policy(abs: f64, rel: f64) -> ComparisonPolicy {
    ComparisonPolicy {
        id: "qc-v1".into(),
        version: 1,
        outputs: BTreeMap::from([(
            "/count".into(),
            Comparator::JsonMetrics {
                version: 1,
                absolute_tolerance: abs,
                relative_tolerance: rel,
                ignored_pointers: vec![],
                unordered_arrays: false,
                max_nodes: 100,
            },
        )]),
        required_context: vec!["workflow_digest".into(), "schema_digest".into()],
    }
}
fn records(
    a: Value,
    b: Value,
) -> (
    ga4gh_sdk::workflow::results::ResultRecord,
    ga4gh_sdk::workflow::results::ResultRecord,
) {
    let mut s = schema();
    s["properties"]["outputs"]["properties"]["count"] = json!({"type":"number"});
    let p = profile(s);
    let mut a = p
        .project_results(
            "a",
            "COMPLETE",
            &json!({"count":a,"report":"https://example.org/qc"}),
        )
        .unwrap();
    let mut b = p
        .project_results(
            "b",
            "COMPLETE",
            &json!({"count":b,"report":"https://example.org/qc"}),
        )
        .unwrap();
    a.context.insert("workflow_digest".into(), "abc".into());
    b.context = a.context.clone();
    (a, b)
}
#[test]
fn numeric_zero_boundary_relative_and_nonfinite_policy() {
    let (a, b) = records(json!(0), json!(0.5));
    assert_eq!(
        policy(0.5, 0.).compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
    assert_eq!(
        policy(0.49, 0.).compare(&a, &b).unwrap().outcome,
        Outcome::Different
    );
    let (a, b) = records(json!(100), json!(101));
    assert_eq!(
        policy(0., 0.01).compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
    assert!(policy(f64::NAN, 0.).compare(&a, &b).is_err());
    let (a, b) = records(json!(0), Value::Null);
    assert_eq!(
        policy(0., 0.).compare(&a, &b).unwrap().outcome,
        Outcome::Inconclusive
    );
}
#[test]
fn identity_drift_and_missing_evidence_are_separate_from_metric_agreement() {
    let (mut a, mut b) = records(json!(3), json!(3));
    a.context.insert("backend".into(), "node-a".into());
    b.context.insert("backend".into(), "node-b".into());
    let report = policy(0., 0.).compare(&a, &b).unwrap();
    assert_eq!(report.outcome, Outcome::EquivalentUnderPolicy);
    assert!(report.context_drift.contains_key("backend"));
    b.context.insert("workflow_digest".into(), "changed".into());
    let report = policy(0., 0.).compare(&a, &b).unwrap();
    assert_eq!(report.outputs["/count"], Outcome::EquivalentUnderPolicy);
    assert_eq!(report.outcome, Outcome::Inconclusive);
    b.context.remove("workflow_digest");
    assert_eq!(
        policy(0., 0.).compare(&a, &b).unwrap().missing_context,
        vec!["workflow_digest"]
    );
}
#[test]
fn json_order_ignored_metadata_array_order_and_bounds_are_explicit() {
    let mut s = schema();
    s["properties"]["outputs"]["properties"]["count"] = json!({"type":"object"});
    let p = profile(s);
    let a = p
        .project_results(
            "a",
            "COMPLETE",
            &json!({"count":{"x":2,"array":[1,2],"time":1},"report":"https://example.org"}),
        )
        .unwrap();
    let b = p
        .project_results(
            "b",
            "COMPLETE",
            &json!({"count":{"time":2,"array":[2,1],"x":2},"report":"https://example.org"}),
        )
        .unwrap();
    let mut policy = policy(0., 0.);
    policy.required_context.clear();
    assert_eq!(policy.compare(&a, &b).unwrap().outcome, Outcome::Different);
    if let Comparator::JsonMetrics {
        ignored_pointers,
        unordered_arrays,
        ..
    } = policy.outputs.get_mut("/count").unwrap()
    {
        ignored_pointers.push("/time".into());
        *unordered_arrays = true;
    }
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
    if let Comparator::JsonMetrics { max_nodes, .. } = policy.outputs.get_mut("/count").unwrap() {
        *max_nodes = 1;
    }
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::Inconclusive
    );
}
#[tokio::test]
async fn explicit_http_artifact_access_preserves_partial_records_and_no_credentials() {
    use ga4gh_sdk::{
        clients::drs::{models::access_method::Type, uri::AccessPlan},
        utils::operation::Operation,
    };
    use std::time::Duration;
    let mock = mockito::mock("GET", "/report")
        .match_header("authorization", "Bearer object-only")
        .with_status(403)
        .expect(1)
        .create();
    let plan = AccessPlan {
        object_id: "report".into(),
        method: Type::Https,
        region: None,
        url: url::Url::parse(&format!("{}/report", mockito::server_url())).unwrap(),
        headers: ga4gh_sdk::reqwest::header::HeaderMap::from_iter([(
            ga4gh_sdk::reqwest::header::AUTHORIZATION,
            "Bearer object-only".parse().unwrap(),
        )]),
    };
    let p = profile(schema());
    let mut r = p
        .project_results(
            "run",
            "COMPLETE",
            &json!({"count":2,"report":"drs://example.org/report"}),
        )
        .unwrap();
    r.verify_access_plan(
        "/report",
        &plan,
        None,
        &Operation::new(Duration::from_secs(2)),
        1024,
    )
    .await
    .unwrap();
    assert_eq!(
        r.artifacts["/report"].access_status,
        AccessStatus::Inaccessible
    );
    assert_eq!(
        r.outputs["count"].contract_status,
        ContractStatus::Conformant
    );
    assert!(!serde_json::to_string(&r).unwrap().contains("object-only"));
    mock.assert();
}

#[test]
fn all_pinned_profile_reference_fixtures_are_checked() {
    for bytes in [
        include_bytes!("../../tests/fixtures/io-profile/example_tool.schema.json").as_slice(),
        include_bytes!("../../tests/fixtures/io-profile/valid.schema.json").as_slice(),
    ] {
        assert!(Profile::from_slice(bytes, Pins::reference()).is_ok());
    }
    assert!(Profile::from_slice(
        include_bytes!("../../tests/fixtures/io-profile/invalid.schema.json"),
        Pins::reference()
    )
    .is_err());
}
#[test]
fn large_integer_differences_and_decimal_boundaries_keep_precision() {
    let (a, b) = records(json!(9007199254740992u64), json!(9007199254740993u64));
    assert_eq!(
        policy(0.1, 0.).compare(&a, &b).unwrap().outcome,
        Outcome::Different
    );
    assert_eq!(
        policy(1., 0.).compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
    let (a, b) = records(json!(0.3), json!(0.2));
    assert_eq!(
        policy(0.1, 0.).compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
    let (a, b) = records(json!(1), json!(1.0));
    assert_eq!(
        policy(0., 0.).compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
}
#[test]
fn digest_comparison_requires_consistent_expected_evidence() {
    let p = profile(schema());
    let mut a = p
        .project_results(
            "a",
            "COMPLETE",
            &json!({"count":1,"report":"drs://example.org/report"}),
        )
        .unwrap();
    let mut b = a.clone();
    let mut policy = policy(0., 0.);
    policy.required_context.clear();
    policy.outputs = BTreeMap::from([("/report".into(), Comparator::Sha256 { version: 1 })]);
    a.verify_bytes("/report", b"same", None).unwrap();
    b.verify_bytes("/report", b"same", None).unwrap();
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::Inconclusive
    );
    let expected = ExpectedChecksum {
        algorithm: "sha-256".into(),
        digest: ga4gh_sdk::workflow::digest(b"same"),
        evidence_source: "manifest@v1".into(),
    };
    a.verify_bytes("/report", b"same", Some(expected.clone()))
        .unwrap();
    b.verify_bytes("/report", b"same", Some(expected)).unwrap();
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
    b.artifacts.get_mut("/report").unwrap().computed_sha256 = None;
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::Inconclusive
    );
}

#[tokio::test]
async fn selected_drs_verification_uses_owning_resolver_and_only_object_headers() {
    use ga4gh_sdk::{
        clients::drs::uri::{AccessPreferences, DrsResolver},
        utils::{configuration::Configuration, operation::Operation, transport::Transport},
    };
    let store = mockito::mock("GET", "/selected-store")
        .match_header("authorization", "Bearer object-only")
        .with_body("report")
        .expect(1)
        .create();
    let metadata=mockito::mock("GET","/selected-drs/objects/report").match_header("authorization","Bearer resolver-only").with_body(json!({"id":"report","self_uri":"drs://drs.example/report","size":6,"created_time":"2026-01-01T00:00:00Z","checksums":[],"access_methods":[{"type":"https","access_url":{"url":format!("{}/selected-store",mockito::server_url()),"headers":["Authorization: Bearer object-only"]}}]}).to_string()).expect(1).create();
    let mut config = Configuration::new(
        url::Url::parse(&format!("{}/selected-drs", mockito::server_url())).unwrap(),
    );
    config.bearer_access_token = Some("resolver-only".into());
    let resolver = DrsResolver::default()
        .with_service(
            "drs.example",
            Transport::with_client(
                &config,
                ga4gh_sdk::reqwest::Client::builder()
                    .default_headers(ga4gh_sdk::reqwest::header::HeaderMap::from_iter([(
                        ga4gh_sdk::reqwest::header::AUTHORIZATION,
                        "Bearer resolver-only".parse().unwrap(),
                    )]))
                    .build()
                    .unwrap(),
            ),
        )
        .unwrap();
    let p = profile(schema());
    let mut r = p
        .project_results(
            "run",
            "COMPLETE",
            &json!({"count":1,"report":"drs://drs.example/report"}),
        )
        .unwrap();
    r.verify_drs(
        "/report",
        &resolver,
        &AccessPreferences::default(),
        None,
        &Operation::new(std::time::Duration::from_secs(3)),
        2,
        1024,
    )
    .await
    .unwrap();
    assert_eq!(
        r.artifacts["/report"].access_status,
        AccessStatus::Available
    );
    assert_eq!(
        r.artifacts["/report"].integrity_status,
        IntegrityStatus::Unverified
    );
    let durable = serde_json::to_string(&r).unwrap();
    assert!(!durable.contains("resolver-only"));
    assert!(!durable.contains("object-only"));
    metadata.assert();
    store.assert();
}

#[test]
fn literal_schema_shaped_values_are_not_required_vocabularies() {
    let literal = json!({"$vocabulary":{"https://data.example/literal":true}});
    let mut s = schema();
    s["properties"]["inputs"] = json!({"const":literal});
    let p = profile(s);
    assert!(p.validate_inputs(&literal).is_empty());
}
#[test]
fn output_keys_keep_json_pointer_and_uri_fragment_escaping() {
    let key = "slash/key#percent%é";
    let mut s = schema();
    s["properties"]["outputs"] =
        json!({"type":"object","properties":{key:{"type":"integer"}},"required":[key]});
    let p = profile(s);
    let r = p
        .project_results("one", "COMPLETE", &json!({key:4}))
        .unwrap();
    assert_eq!(r.outputs[key].contract_status, ContractStatus::Conformant);
}
#[test]
fn unordered_numeric_objects_ignore_json_number_representation() {
    let mut s = schema();
    s["properties"]["outputs"]["properties"]["count"] = json!({"type":"array"});
    let p = profile(s);
    let a = p
        .project_results(
            "a",
            "COMPLETE",
            &json!({"count":[{"n":1,"x":1},{"n":1.0,"x":2}],"report":"https://example.org"}),
        )
        .unwrap();
    let b = p
        .project_results(
            "b",
            "COMPLETE",
            &json!({"count":[{"n":1.0,"x":1},{"n":1,"x":2}],"report":"https://example.org"}),
        )
        .unwrap();
    let mut policy = policy(0., 0.);
    policy.required_context.clear();
    if let Comparator::JsonMetrics {
        unordered_arrays, ..
    } = policy.outputs.get_mut("/count").unwrap()
    {
        *unordered_arrays = true;
    }
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::EquivalentUnderPolicy
    );
}

#[test]
fn typed_result_retains_root_context_for_referenced_annotations() {
    let mut s = schema();
    s["$defs"]["html-report"] = json!({"type":"string","format":"uri","contentMediaType":"text/html","x-ga4gh-ui-widget":"html-viewer","x-ga4gh-ontology":"http://edamontology.org/format_2331"});
    s["properties"]["outputs"]["properties"]["report"] = json!({"$ref":"#/$defs/html-report"});
    let p = profile(s.clone());
    let record = p
        .project_results(
            "one",
            "COMPLETE",
            &json!({"count":1,"report":"drs://example.org/report"}),
        )
        .unwrap();
    assert_eq!(
        record.outputs["report"].contract_status,
        ContractStatus::Conformant
    );
    assert_eq!(record.contract_document, s);
    assert_eq!(
        record.outputs["report"].annotations["$ref"],
        "#/$defs/html-report"
    );
}
