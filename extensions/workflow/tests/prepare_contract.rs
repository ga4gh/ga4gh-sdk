use ga4gh_sdk::workflow::{digest, Pins, Profile};
use serde_json::{json, Value};

fn validator(source: &str) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(source).unwrap();
    jsonschema::validator_for(&schema).unwrap()
}

#[test]
fn prepare_envelopes_preserve_profile_bytes_records_and_diagnostics() {
    let input = validator(include_str!("../api/v1/prepare-request.schema.json"));
    let output = validator(include_str!("../api/v1/prepare-reply.schema.json"));
    let original = include_str!("../../../examples/sequence-qc/profile.json");
    let request: Value =
        serde_json::from_str(include_str!("../../../examples/sequence-qc/request.json")).unwrap();
    let mut fingerprints = Vec::new();
    for text in [original.to_owned(), format!("{original}\r\n")] {
        let message = json!({"profile_utf8":text,"pins":Pins::reference(),
            "request":request,"identity":{"workflow":"public-qc@1"}});
        let decoded: Value =
            serde_json::from_slice(&serde_json::to_vec(&message).unwrap()).unwrap();
        assert!(input.is_valid(&decoded));
        let profile = Profile::from_slice(
            decoded["profile_utf8"].as_str().unwrap().as_bytes(),
            serde_json::from_value(decoded["pins"].clone()).unwrap(),
        )
        .unwrap();
        let plan = profile
            .prepare(
                serde_json::from_value(decoded["request"].clone()).unwrap(),
                serde_json::from_value(decoded["identity"].clone()).unwrap(),
            )
            .unwrap();
        fingerprints.push(plan.fingerprint().to_owned());
        let record = serde_json::to_value(plan).unwrap();
        assert_eq!(record["schema_digest"], digest(text.as_bytes()));
        assert!(output.is_valid(&json!({"prepared":{"state_ref":"live-1","record":record}})));
        assert!(!input.is_valid(&record));
        assert!(!output.is_valid(&json!({"prepared":{"record":record}})));
        let diagnostics = profile.validate_inputs(&json!({"sequences":["INVALID"]}));
        assert!(diagnostics
            .iter()
            .any(|item| item.instance_location == "/sequences/0"));
        let error = json!({"code":"validation_failed","message":"Invalid inputs","diagnostics":diagnostics});
        assert!(output.is_valid(&json!({"error":error})));
        assert!(!output
            .is_valid(&json!({"error":error,"prepared":{"state_ref":"live-1","record":record}})));
    }
    assert_ne!(fingerprints[0], fingerprints[1]);
}
