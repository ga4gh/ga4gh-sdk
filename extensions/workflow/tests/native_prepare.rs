use ga4gh_sdk::{
    utils::{extension::ExtensionInstance, extension_manager::ExtensionManager},
    workflow::{digest, Pins},
};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::{env, fs, path::PathBuf};

const CAPABILITY: &str = "ga4gh.workflow";

fn invoke(instance: &ExtensionInstance, operation: &str, request: &Value) -> Value {
    let reply = instance
        .invoke_bounded(
            CAPABILITY,
            operation,
            &serde_json::to_vec(request).unwrap(),
            64 << 20,
        )
        .unwrap();
    let value = serde_json::from_slice(&reply).unwrap();
    let schema: Value = match operation {
        "prepare" => {
            serde_json::from_str(include_str!("../api/v1/prepare-reply.schema.json")).unwrap()
        }
        "release" => {
            serde_json::from_str(include_str!("../api/v1/release-reply.schema.json")).unwrap()
        }
        _ => {
            let mut schema: Value =
                serde_json::from_str(include_str!("../api/v1/operations.schema.json")).unwrap();
            schema["$ref"] = json!(format!("#/$defs/{operation}_request"));
            assert!(jsonschema::validator_for(&schema)
                .unwrap()
                .is_valid(request));
            schema["$ref"] = json!(format!("#/$defs/{operation}_reply"));
            schema
        }
    };
    assert!(
        jsonschema::validator_for(&schema).unwrap().is_valid(&value),
        "{value}"
    );
    value
}

#[tokio::test]
async fn preparation_is_validated_and_owned_by_the_instance() {
    let directory = tempfile::tempdir().unwrap();
    let artifact =
        PathBuf::from(env::var_os("GA4GH_WORKFLOW_LIB_DIR").expect("run make workflow-extension"))
            .join(format!(
                "{}ga4gh_workflow_extension{}",
                env::consts::DLL_PREFIX,
                env::consts::DLL_SUFFIX
            ));
    let mut manifest: Value =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/descriptor.json"))).unwrap();
    let name = "ga4gh-workflow-extension";
    manifest["manifest_version"] = json!(1);
    manifest["digest"] = json!(Sha512::digest(fs::read(&artifact).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>());
    manifest["path"] = json!(artifact);
    let file = directory.path().join("manifest.json");
    fs::write(&file, manifest.to_string()).unwrap();
    let mut manager = ExtensionManager::open(directory.path().join("registry.json")).unwrap();
    manager.add_extension(file).await.unwrap();
    manager.enable_extension(name).unwrap();
    let resolve = |revision| {
        manager.resolve(
            name,
            None,
            None,
            &[(CAPABILITY.to_owned(), vec![revision])].into(),
        )
    };
    assert!(resolve(2).is_err());
    let instance = || unsafe {
        manager
            .create_instance(&resolve(1).unwrap(), b"{}")
            .unwrap()
    };
    let owner = instance();
    {
        let mut input = vec![b' '; ga4gh_cli_protocol::MAX_MESSAGE];
        input[..2].copy_from_slice(b"{}");
        let at_limit = owner.invoke(CAPABILITY, "prepare", &input).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&at_limit).unwrap()["error"]["code"],
            "invalid_request"
        );
        input.push(b' ');
        assert!(owner.invoke(CAPABILITY, "prepare", &input).is_err());
    }
    let other = instance();
    let text = include_str!("../../../examples/sequence-qc/profile.json");
    let mut request = json!({"pins":Pins::reference(),"identity":{"workflow":"public-qc@1"},"profile_utf8":text,
        "request":serde_json::from_str::<Value>(include_str!("../../../examples/sequence-qc/request.json")).unwrap()});
    let first = invoke(&owner, "prepare", &request);
    let mut changed = request.clone();
    let crlf = text.replace("\r\n", "\n").replace('\n', "\r\n");
    changed["profile_utf8"] = json!(crlf);
    let alternate = invoke(&instance(), "prepare", &changed);
    assert_eq!(
        alternate["prepared"]["record"]["schema_digest"],
        digest(crlf.as_bytes())
    );
    assert_ne!(
        alternate["prepared"]["record"]["fingerprint"],
        first["prepared"]["record"]["fingerprint"]
    );
    let record = &first["prepared"]["record"];
    assert_eq!(record["schema_digest"], digest(text.as_bytes()));
    let key = json!({"state_ref":first["prepared"]["state_ref"].as_str().unwrap()});
    assert_eq!(
        invoke(&owner, "prepare", &request)["error"]["code"],
        "state_in_use"
    );
    let second = invoke(&other, "prepare", &request);
    assert_eq!(second["prepared"]["record"], *record);
    assert_ne!(second["prepared"]["state_ref"], key["state_ref"]);
    let submitter = instance();
    let prepared = invoke(&submitter, "prepare", &request);
    let state = &prepared["prepared"]["state_ref"];
    let begin = json!({"state_ref":state,"attachments":[]});
    let finish = json!({"state_ref":state,"outcome":{"submitted":{"run_id":"accepted"}}});
    assert_eq!(
        invoke(&submitter, "finish_submit", &finish)["error"]["code"],
        "invalid_state"
    );
    assert_eq!(
        invoke(&submitter, "begin_submit", &begin)["ok"]["request"],
        request["request"]
    );
    assert_eq!(
        invoke(&submitter, "release", &json!({"state_ref":state}))["error"]["code"],
        "invalid_state"
    );
    assert_eq!(
        invoke(&submitter, "prepare", &request)["error"]["code"],
        "state_in_use"
    );
    assert_eq!(
        invoke(&submitter, "begin_submit", &begin)["error"]["code"],
        "invalid_state"
    );
    assert_eq!(
        invoke(&submitter, "finish_submit", &finish)["ok"]["invocation"],
        *record
    );
    assert_eq!(
        invoke(&submitter, "finish_submit", &finish)["error"]["code"],
        "invalid_state"
    );
    let result = invoke(
        &submitter,
        "results",
        &json!({"profile_utf8":text,"pins":Pins::reference(),"run_reference":"run-1","state":"COMPLETE","outputs":{}}),
    );
    let verified = invoke(
        &submitter,
        "verify",
        &json!({"record":result["ok"],"pointer":"/report","base64":""}),
    );
    let policy: Value =
        serde_json::from_str(include_str!("../../../examples/sequence-qc/policy.json")).unwrap();
    assert_eq!(
        invoke(
            &submitter,
            "compare",
            &json!({"left":verified["ok"],"right":verified["ok"],"policy":policy})
        )["ok"]["outcome"],
        "inconclusive"
    );
    assert_eq!(
        invoke(&other, "release", &key)["error"]["code"],
        "invalid_state"
    );
    let retained = other.method(CAPABILITY, "release").unwrap();
    drop(other);
    let release_input =
        serde_json::to_vec(&json!({"state_ref": second["prepared"]["state_ref"]})).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&retained.call(&release_input).unwrap()).unwrap(),
        json!({"released":true})
    );
    assert_eq!(invoke(&owner, "release", &key), json!({"released":true}));
    assert_eq!(
        invoke(&owner, "release", &key)["error"]["code"],
        "invalid_state"
    );
    assert_eq!(
        invoke(&owner, "prepare", record)["error"]["code"],
        "invalid_request"
    );
    let fresh = invoke(&owner, "prepare", &request);
    assert_eq!(fresh["prepared"]["record"], *record);
    assert_ne!(fresh["prepared"]["state_ref"], key["state_ref"]);
    assert_eq!(
        invoke(&owner, "release", &key)["error"]["code"],
        "invalid_state"
    );
    assert_eq!(
        invoke(
            &owner,
            "release",
            &json!({"state_ref": fresh["prepared"]["state_ref"]})
        ),
        json!({"released":true})
    );
    request["request"]["workflow_params"]["sequences"] = json!(["PRIVATE_INVALID"]);
    let failed = invoke(&instance(), "prepare", &request);
    assert_eq!(failed["error"]["code"], "validation_failed");
    assert!(failed["error"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| item["instance_location"] == "/sequences/0"));
    assert!(!failed.to_string().contains("PRIVATE_INVALID"));
}
