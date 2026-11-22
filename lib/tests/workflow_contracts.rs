#![cfg(feature = "workflow-contracts")]
use ga4gh_sdk::clients::{
    serviceinfo::models::{Service, ServiceType},
    wes::{models::RunRequest, WorkflowAttachment, WES},
};
use ga4gh_sdk::utils::{configuration::Configuration, transport::Transport};
use ga4gh_sdk::workflow::{results::ResultRecord, InvocationIdentity, Pins, Profile};
use serde_json::{json, Value};
fn schema() -> Value {
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "type":"object", "$defs":{"choice":{"type":"string","enum":["fast","full"]}}, "properties":{"inputs":{"type":"object","properties":{"mode":{"$ref":"#/$defs/choice"},"nested":{"type":"object","properties":{"n":{"type":"integer"}},"required":["n"]}},"required":["mode","nested"]},"outputs":{"type":"object","properties":{"count":{"type":"integer"},"report":{"type":"string","format":"uri","contentMediaType":"text/html","x-ga4gh-ui-widget":"html-viewer"}},"required":["count","report"]}}})
}
fn profile(value: Value) -> Profile {
    Profile::from_slice(&serde_json::to_vec(&value).unwrap(), Pins::reference()).unwrap()
}
/// Returns a WES client for the API root `base` that skips service-info
/// discovery.
fn wes(base: &str) -> WES {
    WES::without_discovery(
        Transport::new(&Configuration::new(url::Url::parse(base).unwrap())),
        Service {
            r#type: Box::new(ServiceType {
                artifact: "wes".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
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
fn fingerprint_ignores_map_iteration_order() {
    // Each fresh `HashMap` gets its own random hasher, so the maps below
    // iterate in different orders; the fingerprint must not depend on it.
    let p = profile(schema());
    let prepare = |reverse: bool| {
        let mut keys: Vec<String> = (0..32).map(|i| format!("key-{i:02}")).collect();
        if reverse {
            keys.reverse();
        }
        let map = || {
            keys.iter()
                .map(|k| (k.clone(), format!("value of {k}")))
                .collect::<std::collections::HashMap<_, _>>()
        };
        let req = RunRequest {
            workflow_url: "workflow.cwl".into(),
            workflow_type: "CWL".into(),
            workflow_type_version: "v1.2".into(),
            workflow_params: Some(json!({"mode":"fast","nested":{"n":4}})),
            tags: Some(map()),
            workflow_engine_parameters: Some(map()),
            ..Default::default()
        };
        p.prepare(
            req,
            InvocationIdentity {
                workflow: "qc@sha256:abc".into(),
                descriptor_digest: None,
            },
        )
        .unwrap()
        .fingerprint()
        .to_owned()
    };
    let expected = prepare(false);
    for reverse in [false, true].into_iter().cycle().take(16) {
        assert_eq!(prepare(reverse), expected);
    }
}
#[test]
fn prepare_rejects_extra_fields_that_shadow_request_fields() {
    let p = profile(schema());
    let prepare = |extra: Value| {
        let req = RunRequest {
            workflow_url: "workflow.cwl".into(),
            workflow_type: "CWL".into(),
            workflow_type_version: "v1.2".into(),
            workflow_params: Some(json!({"mode":"fast","nested":{"n":4}})),
            extra: extra.as_object().unwrap().clone(),
            ..Default::default()
        };
        p.prepare(
            req,
            InvocationIdentity {
                workflow: "qc@sha256:abc".into(),
                descriptor_digest: None,
            },
        )
    };
    // The fingerprint would hash the extra value instead of the validated one
    assert!(prepare(json!({"workflow_params":{"mode":"slow"}})).is_err());
    assert!(prepare(json!({"workflow_url":"other.cwl"})).is_err());
    // Submitting rejects these too, even if the request leaves them unset
    assert!(prepare(json!({"tags":{"a":"b"}})).is_err());
    assert!(prepare(json!({"workflow_attachment":"x"})).is_err());
    assert!(prepare(json!({"vendor_option":1})).is_ok());
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
#[test]
fn contract_document_and_embedded_urls_are_redacted_like_outputs() {
    let mut s = schema();
    s["properties"]["outputs"]["properties"]["note"] = json!({
        "type": "string",
        "description": "Mirror: https://user:password@example.org/data?token=secret#cap."
    });
    s["properties"]["outputs"]["properties"]["report"]["examples"] =
        json!(["https://user:password@example.org/report?token=secret"]);
    let p = profile(s);
    let r = p
        .project_results(
            "one",
            "COMPLETE",
            &json!({
                "count": 1,
                "report": "https://example.org/report",
                "note": "Read https://user:password@example.org/a?token=secret#cap, then https://example.org/b."
            }),
        )
        .unwrap();
    let durable = serde_json::to_string(&r).unwrap();
    for secret in ["user", "password", "token", "secret", "cap"] {
        assert!(!durable.contains(secret), "{secret} in {durable}");
    }
    // Embedded URLs lose only their access details; the text stays
    assert_eq!(
        r.value_at("/note"),
        Some(&json!(
            "Read https://example.org/a, then https://example.org/b."
        ))
    );
    assert!(r.outputs["note"].access_details_redacted);
    assert!(!r.outputs["report"].access_details_redacted);
    assert_eq!(
        r.contract_document["properties"]["outputs"]["properties"]["note"]["description"],
        "Mirror: https://example.org/data."
    );
    assert_eq!(
        r.outputs["note"].annotations["description"],
        "Mirror: https://example.org/data."
    );
    // Text without access details stays as it is
    assert_eq!(
        r.contract_document["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    // The digest still identifies the profile as given
    assert_eq!(r.schema_digest, p.schema_digest());
}

#[test]
fn embedded_query_delimiters_are_redacted_in_stored_results() {
    for (kind, url) in [
        ("angle", "https://example.org/report?token=<secret>"),
        ("quote", "https://example.org/report?token=\"secret\""),
        (
            "backtick",
            "https://example.org/report?token=\u{60}secret\u{60}",
        ),
        ("fragment", "https://example.org/report#token=<secret>"),
    ] {
        assert!(url::Url::parse(url).is_ok(), "{kind}");
        let mut s = schema();
        s["properties"]["outputs"]["properties"]["report"]["description"] =
            json!(format!("Mirror: {url} then continue"));
        let p = profile(s);
        let r = p
            .project_results(
                "one",
                "COMPLETE",
                &json!({"count": 1, "report": "https://example.org/report"}),
            )
            .unwrap();
        let stored = serde_json::to_string(&r).unwrap();
        assert!(!stored.contains("secret"), "{kind}: {stored}");
        assert!(
            !r.outputs["report"].annotations["description"]
                .as_str()
                .unwrap()
                .contains("secret"),
            "{kind}"
        );
    }

    // JSON escapes can hide the URL scheme in a string that itself holds JSON.
    let mut s = schema();
    s["properties"]["outputs"]["properties"]["report"]["description"] =
        json!(r#"{"url":"\u0068ttps://example.org/report?token=\u003Csecret\u003E","n":2}"#);
    let p = profile(s);
    let r = p
        .project_results(
            "one",
            "COMPLETE",
            &json!({"count": 1, "report": "https://example.org/report"}),
        )
        .unwrap();
    assert_eq!(
        r.contract_document["properties"]["outputs"]["properties"]["report"]["description"],
        r#"{"url":"https://example.org/report","n":2}"#
    );
    assert_eq!(
        r.outputs["report"].annotations["description"],
        r#"{"url":"https://example.org/report","n":2}"#
    );
}

/// Returns a JSON metrics comparator with the tolerances `abs` and `rel`
/// that skips `ignored` pointers, sorts arrays if `unordered` and walks at
/// most `max_nodes` nodes.
fn json_metrics(
    abs: f64,
    rel: f64,
    ignored: &[&str],
    unordered: bool,
    max_nodes: usize,
) -> Comparator {
    Comparator::JsonMetrics {
        version: 1,
        absolute_tolerance: abs,
        relative_tolerance: rel,
        ignored_pointers: ignored.iter().map(|p| (*p).to_owned()).collect(),
        unordered_arrays: unordered,
        max_nodes,
    }
}
fn policy(abs: f64, rel: f64) -> ComparisonPolicy {
    ComparisonPolicy {
        id: "qc-v1".into(),
        version: 1,
        outputs: BTreeMap::from([("/count".into(), json_metrics(abs, rel, &[], false, 100))]),
        required_context: vec!["workflow_digest".into(), "schema_digest".into()],
    }
}
/// Projects `a` and `b` as the outputs of two runs under the output
/// contract `outputs` and gives both the same required context.
fn project_pair(outputs: Value, a: Value, b: Value) -> (ResultRecord, ResultRecord) {
    let mut s = schema();
    s["properties"]["outputs"] = outputs;
    let p = profile(s);
    let mut a = p.project_results("a", "COMPLETE", &a).unwrap();
    let mut b = p.project_results("b", "COMPLETE", &b).unwrap();
    a.context.insert("workflow_digest".into(), "abc".into());
    b.context = a.context.clone();
    (a, b)
}
fn records(a: Value, b: Value) -> (ResultRecord, ResultRecord) {
    let mut outputs = schema()["properties"]["outputs"].clone();
    outputs["properties"]["count"] = json!({"type":"number"});
    project_pair(
        outputs,
        json!({"count":a,"report":"https://example.org/qc"}),
        json!({"count":b,"report":"https://example.org/qc"}),
    )
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
#[test]
fn json_metrics_outcome_ignores_key_order_and_nulls_yield_to_differences() {
    let mut s = schema();
    s["properties"]["outputs"]["properties"]["count"] = json!({"type":"object"});
    let p = profile(s);
    let outcome = |a: Value, b: Value| {
        let project = |run: &str, count: Value| {
            let mut r = p
                .project_results(
                    run,
                    "COMPLETE",
                    &json!({"count":count,"report":"https://example.org/qc"}),
                )
                .unwrap();
            r.context.insert("workflow_digest".into(), "abc".into());
            r
        };
        policy(0., 0.)
            .compare(&project("a", a), &project("b", b))
            .unwrap()
            .outcome
    };
    // Check each case with the differing key at both ends ("a" sorts
    // first, "z" last), so the outcome is shown not to depend on position.
    for (first, last) in [("a", "z"), ("z", "a")] {
        // A missing key beats a compared null
        assert_eq!(
            outcome(json!({first:1,last:null}), json!({last:null})),
            Outcome::Different
        );
        // A differing value beats a compared null
        assert_eq!(
            outcome(json!({first:1,last:null}), json!({first:2,last:null})),
            Outcome::Different
        );
        assert_eq!(
            outcome(
                json!({first:[1,{"x":null}],last:[null]}),
                json!({first:[2,{"x":null}],last:[null]})
            ),
            Outcome::Different
        );
        // Without a difference, a compared null makes it inconclusive
        assert_eq!(
            outcome(json!({first:1,last:null}), json!({first:1,last:null})),
            Outcome::Inconclusive
        );
        assert_eq!(
            outcome(json!({first:1,last:null}), json!({first:1,last:3})),
            Outcome::Inconclusive
        );
        assert_eq!(
            outcome(json!({first:1,last:null}), json!({first:2,last:3})),
            Outcome::Different
        );
        assert_eq!(
            outcome(json!({first:1,last:2}), json!({first:1,last:2})),
            Outcome::EquivalentUnderPolicy
        );
    }
    // A key whose JSON pointer is too long is inconclusive, even on one
    // side only
    let long = "k".repeat(2100);
    let long = long.as_str();
    assert_eq!(outcome(json!({long: 1}), json!({})), Outcome::Inconclusive);
    assert_eq!(
        outcome(json!({"a": 1, long: 1}), json!({"a": 2})),
        Outcome::Inconclusive
    );
    // Values over the node bound stay inconclusive, even with a difference
    let wide = |n: i64| {
        Value::Object(
            (0..200)
                .map(|i| (format!("k{i:03}"), json!(i + n)))
                .collect(),
        )
    };
    assert_eq!(outcome(wide(0), wide(1)), Outcome::Inconclusive);
}
#[tokio::test]
async fn explicit_http_artifact_access_preserves_partial_records_and_no_credentials() {
    let mut server = mockito::Server::new_async().await;
    use ga4gh_sdk::{
        clients::drs::{models::access_method::Type, uri::AccessPlan},
        utils::operation::Operation,
    };
    use std::time::Duration;
    let mock = server
        .mock("GET", "/report")
        .match_header("authorization", "Bearer object-only")
        .with_status(403)
        .expect(1)
        .create();
    let plan = AccessPlan {
        object_id: "report".into(),
        method: Type::Https,
        region: None,
        url: url::Url::parse(&format!("{}/report", server.url())).unwrap(),
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
async fn malformed_expected_checksums_fail_before_any_read() {
    use ga4gh_sdk::{
        clients::drs::{
            models::access_method::Type,
            uri::{AccessPlan, AccessPreferences, DrsResolver},
        },
        utils::{configuration::Configuration, operation::Operation, transport::Transport},
    };
    let checksum = |algorithm: &str, digest: &str, source: &str| ExpectedChecksum {
        algorithm: algorithm.into(),
        digest: digest.into(),
        evidence_source: source.into(),
    };
    let valid = "0".repeat(64);
    let malformed = [
        checksum("md5", &valid, "manifest"),
        checksum("sha256", "not-hex", "manifest"),
        checksum("sha256", &valid, " "),
    ];
    // Nothing listens on the discard port, so the read fails
    let plan = AccessPlan {
        object_id: "report".into(),
        method: Type::Https,
        region: None,
        url: url::Url::parse("http://127.0.0.1:9/report").unwrap(),
        headers: Default::default(),
    };
    // Nothing listens on the discard port, so resolution fails too
    let resolver = DrsResolver::default()
        .with_service(
            "drs.example",
            Transport::new(&Configuration::new(
                url::Url::parse("http://127.0.0.1:9/ga4gh/drs/v1").unwrap(),
            )),
        )
        .unwrap();
    let operation = Operation::new(std::time::Duration::from_secs(2));
    let p = profile(schema());
    let mut r = p
        .project_results(
            "run",
            "COMPLETE",
            &json!({"count":1,"report":"drs://drs.example/report"}),
        )
        .unwrap();
    // Fails because of the checksum, not for another reason
    let rejected = |result: Result<(), ga4gh_sdk::workflow::ContractError>, case: &str| {
        let message = result.expect_err(case).to_string();
        assert!(
            message.contains("checksum") || message.contains("SHA-256"),
            "{case}: {message}"
        );
    };
    for expected in malformed {
        let some = || Some(expected.clone());
        let case = |method: &str, pointer: &str| format!("{method} {pointer} {expected:?}");
        for pointer in ["/missing", "/report"] {
            rejected(
                r.verify_bytes(pointer, b"report", some()),
                &case("verify_bytes", pointer),
            );
            rejected(
                r.verify_access_plan(pointer, &plan, some(), &operation, 1024)
                    .await,
                &case("verify_access_plan", pointer),
            );
            rejected(
                r.verify_drs(
                    pointer,
                    &resolver,
                    &AccessPreferences::default(),
                    some(),
                    &operation,
                    1,
                    1024,
                )
                .await,
                &case("verify_drs", pointer),
            );
        }
        assert!(r.artifacts.is_empty());
    }
    // A valid expected checksum is still recorded when the read or the
    // resolution fails
    let valid_checksum = || Some(checksum("SHA-256", &valid, "manifest"));
    r.verify_access_plan("/report", &plan, valid_checksum(), &operation, 1024)
        .await
        .unwrap();
    assert_eq!(
        r.artifacts["/report"].access_status,
        AccessStatus::Inaccessible
    );
    r.artifacts.clear();
    r.verify_drs(
        "/report",
        &resolver,
        &AccessPreferences::default(),
        valid_checksum(),
        &operation,
        1,
        1024,
    )
    .await
    .unwrap();
    assert_eq!(
        r.artifacts["/report"].access_status,
        AccessStatus::Inaccessible
    );
    assert!(r.artifacts["/report"].expected.is_some());
}

#[tokio::test]
async fn selected_drs_verification_uses_owning_resolver_and_only_object_headers() {
    let mut server = mockito::Server::new_async().await;
    use ga4gh_sdk::{
        clients::drs::uri::{AccessPreferences, DrsResolver},
        utils::{configuration::Configuration, operation::Operation, transport::Transport},
    };
    let store = server
        .mock("GET", "/selected-store")
        .match_header("authorization", "Bearer object-only")
        .with_body("report")
        .expect(1)
        .create();
    let metadata=server.mock("GET","/selected-drs/objects/report").match_header("authorization","Bearer resolver-only").with_body(json!({"id":"report","self_uri":"drs://drs.example/report","size":6,"created_time":"2026-01-01T00:00:00Z","checksums":[],"access_methods":[{"type":"https","access_url":{"url":format!("{}/selected-store",server.url()),"headers":["Authorization: Bearer object-only"]}}]}).to_string()).expect(1).create();
    let config =
        Configuration::new(url::Url::parse(&format!("{}/selected-drs", server.url())).unwrap());
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
fn output_keys_with_characters_illegal_in_uri_fragments_are_validated() {
    let keys = [
        // Not allowed in a fragment and left unencoded before #340
        "{",
        "}",
        "|",
        "^",
        "[",
        "]",
        "reads[1]",
        // Not allowed in a fragment and encoded before as well
        "a b",
        "100%",
        "a\"b<c>d",
        "`",
        "back\\slash",
        // Every byte outside ASCII is encoded
        "é",
        // Allowed in a fragment as they are; "/" and "~" are JSON Pointer
        // escapes first
        "x/y~z",
        "what?",
        "a:b@c!$&'()*+,;=-._~",
        // The empty name points at "/properties/outputs/properties/"
        "",
    ];
    let properties: serde_json::Map<String, Value> = keys
        .iter()
        .map(|k| (k.to_string(), json!({"type":"integer"})))
        .collect();
    let mut s = schema();
    s["properties"]["outputs"] = json!({"type":"object","properties":properties});
    let p = profile(s);
    let with = |value: Value| -> Value {
        Value::Object(
            keys.iter()
                .map(|k| (k.to_string(), value.clone()))
                .collect(),
        )
    };
    let good = p
        .project_results("one", "COMPLETE", &with(json!(1)))
        .unwrap();
    let bad = p
        .project_results("two", "COMPLETE", &with(json!("one")))
        .unwrap();
    for key in keys {
        assert_eq!(
            good.outputs[key].contract_status,
            ContractStatus::Conformant,
            "{key}"
        );
        assert_eq!(
            bad.outputs[key].contract_status,
            ContractStatus::Invalid,
            "{key}"
        );
    }
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

/// Reads `value` as a profile with the reference pins and returns the error
/// message; panics if the profile is accepted.
fn profile_error(value: &Value) -> String {
    Profile::from_slice(&serde_json::to_vec(value).unwrap(), Pins::reference())
        .err()
        .unwrap()
        .to_string()
}
#[test]
fn rejects_unpinned_revisions_non_objects_unknown_dialects_and_ids() {
    let bytes = serde_json::to_vec(&schema()).unwrap();
    let mut pins = Pins::reference();
    pins.vocabulary_revision = "main".into();
    assert!(Profile::from_slice(&bytes, pins)
        .err()
        .unwrap()
        .to_string()
        .contains("pinned reference revision"));
    assert!(profile_error(&json!([])).contains("must be an object"));
    let mut s = schema();
    s["$schema"] = json!("http://json-schema.org/draft-07/schema#");
    assert!(profile_error(&s).contains("unsupported schema dialect"));
    for id in ["relative/profile.json", "https://example.org/profile.json#"] {
        let mut s = schema();
        s["$id"] = json!(id);
        assert!(
            profile_error(&s).contains("absolute URI without fragment"),
            "{id}"
        );
    }
}
#[test]
fn checks_vocabularies_in_subschema_arrays_and_accepts_standard_ones() {
    let mut s = schema();
    s["$vocabulary"] = json!({
        "https://json-schema.org/draft/2020-12/vocab/core": true,
        "https://unknown.invalid/optional-vocab": false
    });
    s["allOf"] = json!([{"type":"object"}]);
    let p = profile(s.clone());
    assert_eq!(p.document(), &s);
    s["allOf"] = json!([{"$vocabulary":{"https://unknown.invalid/vocab":true}}]);
    assert!(profile_error(&s).contains("unsupported required vocabulary"));
}
#[test]
fn profile_without_outputs_exposes_pins_and_annotations() {
    let mut s = schema();
    s["properties"].as_object_mut().unwrap().remove("outputs");
    let p = profile(s);
    assert_eq!(p.pins(), &Pins::reference());
    assert_eq!(
        p.annotations("/properties/inputs/properties/mode"),
        Some(&json!({"$ref":"#/$defs/choice"}))
    );
    assert_eq!(p.annotations("/properties/outputs"), None);
    assert!(p
        .validate_outputs(&json!({}))
        .unwrap_err()
        .to_string()
        .contains("no output contract"));
}
#[test]
fn prepare_rejects_blank_identities_and_request_fields() {
    let p = profile(schema());
    let request = RunRequest {
        workflow_url: "workflow.cwl".into(),
        workflow_type: "CWL".into(),
        workflow_type_version: "v1.2".into(),
        workflow_params: Some(json!({"mode":"fast","nested":{"n":4}})),
        ..Default::default()
    };
    let identity = |workflow: &str| InvocationIdentity {
        workflow: workflow.into(),
        descriptor_digest: None,
    };
    assert!(p
        .prepare(request.clone(), identity(" "))
        .unwrap_err()
        .to_string()
        .contains("workflow identity must be pinned"));
    let blank_type = RunRequest {
        workflow_type: " ".into(),
        ..request
    };
    assert!(p
        .prepare(blank_type, identity("qc@sha256:abc"))
        .unwrap_err()
        .to_string()
        .contains("requires workflow_url, workflow_type"));
}
#[tokio::test]
async fn submit_with_attachments_checks_the_primary_descriptor_digest() {
    let mut server = mockito::Server::new_async().await;
    let submission = server
        .mock("POST", "/runs")
        .with_body(r#"{"run_id":"checked-run"}"#)
        .expect(1)
        .create();
    let wes = wes(&server.url());
    let descriptor = b"class: Workflow\n";
    let p = profile(schema());
    let prepare = |descriptor_digest: String| {
        p.prepare(
            RunRequest {
                workflow_url: "workflow.cwl".into(),
                workflow_type: "CWL".into(),
                workflow_type_version: "v1.2".into(),
                workflow_params: Some(json!({"mode":"fast","nested":{"n":4}})),
                ..Default::default()
            },
            InvocationIdentity {
                workflow: "qc@sha256:abc".into(),
                descriptor_digest: Some(descriptor_digest),
            },
        )
        .unwrap()
    };
    let attachment = |filename: &str, content: &[u8]| {
        vec![WorkflowAttachment {
            filename: filename.into(),
            content: content.to_vec(),
        }]
    };
    let expected = format!(
        "sha256:{}",
        ga4gh_sdk::workflow::digest(descriptor).to_ascii_uppercase()
    );
    for (prepared, attachments, message) in [
        (
            prepare(ga4gh_sdk::workflow::digest(descriptor)),
            attachment("workflow.cwl", descriptor),
            "must use sha256:",
        ),
        (
            prepare(expected.clone()),
            attachment("other.cwl", descriptor),
            "primary descriptor attachment missing",
        ),
        (
            prepare(expected.clone()),
            attachment("workflow.cwl", b"class: CommandLineTool\n"),
            "primary descriptor digest mismatch",
        ),
    ] {
        let error = prepared
            .submit_with_attachments(&wes, attachments)
            .await
            .unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
    let run = prepare(expected)
        .submit_with_attachments(&wes, attachment("workflow.cwl", descriptor))
        .await
        .unwrap();
    assert_eq!(run.id, "checked-run");
    submission.assert();
}
/// Projects `a` and `b` as the `count` output of two runs whose contract
/// accepts any JSON value there, with the same required context.
fn any_records(a: Value, b: Value) -> (ResultRecord, ResultRecord) {
    project_pair(
        json!({"type":"object","properties":{"count":{}}}),
        json!({"count":a}),
        json!({"count":b}),
    )
}
/// Compares the `count` outputs `a` and `b` at `pointer` with exact
/// tolerances, skipping `ignored` pointers, sorting arrays if `unordered`
/// and walking at most `max_nodes` nodes.
fn metric_outcome(
    a: Value,
    b: Value,
    pointer: &str,
    ignored: &[&str],
    unordered: bool,
    max_nodes: usize,
) -> Outcome {
    let (a, b) = any_records(a, b);
    let mut policy = policy(0., 0.);
    policy.outputs = BTreeMap::from([(
        pointer.into(),
        json_metrics(0., 0., ignored, unordered, max_nodes),
    )]);
    policy.compare(&a, &b).unwrap().outputs[pointer].clone()
}
#[test]
fn json_metrics_compare_scalars_arrays_and_nested_pointers() {
    let equal = Outcome::EquivalentUnderPolicy;
    let outcome = |a: Value, b: Value| metric_outcome(a, b, "/count", &[], false, 100);
    assert_eq!(outcome(json!("x"), json!("x")), equal);
    assert_eq!(outcome(json!("x"), json!("y")), Outcome::Different);
    assert_eq!(outcome(json!(true), json!(false)), Outcome::Different);
    assert_eq!(outcome(json!([1]), json!([1, 2])), Outcome::Different);
    assert_eq!(outcome(json!(1e300), json!(1e300)), equal);
    assert_eq!(outcome(json!(1e300), json!(2e300)), Outcome::Different);
    let unordered = |a: Value, b: Value| metric_outcome(a, b, "/count", &[], true, 100);
    assert_eq!(
        unordered(json!(["b", true, [1], 2]), json!([2, [1], true, "b"])),
        equal
    );
    assert_eq!(
        unordered(json!(["b", null, 2]), json!([2, null, "b"])),
        Outcome::Inconclusive
    );
    assert_eq!(
        metric_outcome(json!([1, 2]), json!([1, 3]), "/count", &["/1"], false, 100),
        equal
    );
    assert_eq!(
        metric_outcome(
            json!({"x":1,"y":1}),
            json!({"x":1,"y":2}),
            "/count/x",
            &[],
            false,
            100
        ),
        equal
    );
}
#[test]
fn json_metrics_values_beyond_the_bounds_are_inconclusive() {
    let outcome =
        |a: Value, max_nodes: usize| metric_outcome(a.clone(), a, "/count", &[], false, max_nodes);
    let mut deep = json!(1);
    for _ in 0..130 {
        deep = json!([deep]);
    }
    assert_eq!(outcome(deep, 1000), Outcome::Inconclusive);
    assert_eq!(outcome(json!({"a":[1],"b":[1]}), 3), Outcome::Inconclusive);
    assert_eq!(outcome(json!([1, 2, 3]), 2), Outcome::Inconclusive);
    assert_eq!(
        outcome(json!("x".repeat(8 * 1024 * 1024 + 1)), 10),
        Outcome::Inconclusive
    );
    assert_eq!(
        outcome(json!({"k".repeat(2046): [0]}), 10),
        Outcome::Inconclusive
    );
    assert_eq!(
        outcome(json!({"k".repeat(2045): [0]}), 10),
        Outcome::EquivalentUnderPolicy
    );
}
#[test]
fn comparison_rejects_invalid_policies() {
    let (a, b) = any_records(json!(1), json!(1));
    let mut blank = policy(0., 0.);
    blank.id = " ".into();
    let mut relative = policy(0., 0.);
    relative.outputs = BTreeMap::from([("count".into(), Comparator::Sha256 { version: 1 })]);
    let mut digest_version = policy(0., 0.);
    digest_version.outputs = BTreeMap::from([("/count".into(), Comparator::Sha256 { version: 2 })]);
    for (policy, message) in [
        (blank, "requires id, version 1 and named outputs"),
        (relative, "keys must be JSON pointers"),
        (digest_version, "unsupported digest comparator version"),
    ] {
        let error = policy.compare(&a, &b).unwrap_err();
        assert!(error.to_string().contains(message), "{error}");
    }
}
#[test]
fn digest_comparison_detects_different_and_unattested_artifacts() {
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
    let expected = |bytes: &[u8]| ExpectedChecksum {
        algorithm: "sha256".into(),
        digest: ga4gh_sdk::workflow::digest(bytes),
        evidence_source: "manifest@v1".into(),
    };
    a.verify_bytes("/report", b"left", Some(expected(b"left")))
        .unwrap();
    b.verify_bytes("/report", b"right", Some(expected(b"right")))
        .unwrap();
    assert_eq!(policy.compare(&a, &b).unwrap().outcome, Outcome::Different);
    b.artifacts
        .get_mut("/report")
        .unwrap()
        .expected
        .as_mut()
        .unwrap()
        .evidence_source = " ".into();
    assert_eq!(
        policy.compare(&a, &b).unwrap().outcome,
        Outcome::Inconclusive
    );
}
#[test]
fn stored_results_redact_json_text_and_reference_fragments_but_keep_data() {
    let mut s = schema();
    s["$defs"]["external"] =
        json!({"$dynamicRef":"https://example.org/schema.json?sig=SECRET#a=b"});
    s["$defs"]["unanchored"] = json!({"$dynamicRef":"https://example.org/other.json?sig=SECRET"});
    s["properties"]["outputs"] =
        json!({"type":"object","properties":{"count":{"x-ga4gh-hidden":true}}});
    let p = profile(s);
    let record = p
        .project_results(
            "run",
            "COMPLETE",
            &json!({"count":r#"["plain","https://example.org/r?sig=SECRET"]"#}),
        )
        .unwrap();
    assert_eq!(
        record.contract_document["$defs"]["external"]["$dynamicRef"],
        "https://example.org/schema.json"
    );
    assert_eq!(
        record.contract_document["$defs"]["unanchored"]["$dynamicRef"],
        "https://example.org/other.json"
    );
    assert_eq!(
        record.outputs["count"].annotations,
        json!({"x-ga4gh-hidden":true})
    );
    assert_eq!(
        record.outputs["count"].value,
        Some(json!(r#"["plain","https://example.org/r"]"#))
    );
    assert!(record.outputs["count"].access_details_redacted);
}
#[tokio::test]
async fn access_plan_for_a_missing_output_records_it_without_a_request() {
    use ga4gh_sdk::{
        clients::drs::{models::access_method::Type, uri::AccessPlan},
        utils::operation::Operation,
    };
    let plan = AccessPlan {
        object_id: "missing".into(),
        method: Type::Https,
        region: None,
        url: url::Url::parse("http://127.0.0.1:9/never-requested").unwrap(),
        headers: Default::default(),
    };
    let mut record = profile(schema())
        .project_results("run", "COMPLETE", &json!({"count":1}))
        .unwrap();
    record
        .verify_access_plan(
            "/report",
            &plan,
            None,
            &Operation::new(std::time::Duration::from_secs(2)),
            1024,
        )
        .await
        .unwrap();
    assert_eq!(
        record.artifacts["/report"].access_status,
        AccessStatus::Missing
    );
}
