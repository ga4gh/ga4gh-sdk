//! Models keep fields and enum values that their spec doesn't define.
//!
//! Each API gets a round trip: a body with extra fields (top level and
//! nested) and enum values outside the spec is read and written back, and the
//! JSON must not change. States that the TES and WES specs don't define read
//! as `UNKNOWN`, which both specs define as the safe default.

use ga4gh_sdk::clients::{drs, registry, serviceinfo, tes, trs, wes};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

/// Reads `body` as `T`, writes it back and checks that nothing changed.
fn round_trip<T: DeserializeOwned + Serialize>(body: Value) -> T {
    let model: T = serde_json::from_value(body.clone()).expect("body parses");
    assert_eq!(
        serde_json::to_value(&model).expect("model serializes"),
        body
    );
    model
}

fn service_info_body(artifact: &str) -> Value {
    json!({
        "id": "org.example.service",
        "name": "Example",
        "type": {
            "group": "org.ga4gh",
            "artifact": artifact,
            "version": "1.0.0",
            "x-type-note": "nested"
        },
        "organization": { "name": "Example", "url": "https://example.org", "x-org": 1 },
        "version": "1.0.0",
        "x-vendor": { "feature": true }
    })
}

#[test]
fn tes_task_keeps_extra_fields_and_unknown_file_types() {
    let body = json!({
        "id": "task-1",
        "state": "COMPLETE",
        "executors": [{ "image": "alpine", "command": ["true"], "x-gpu": "a100" }],
        "inputs": [{ "path": "/in", "url": "s3://bucket/in", "type": "SYMLINK" }],
        "tags": { "project": "demo" },
        "x-backend": { "queue": "long" }
    });
    let task: tes::models::TesTask = round_trip(body);
    assert_eq!(task.extra["x-backend"], json!({ "queue": "long" }));
    let executors = task.executors.as_ref().unwrap();
    assert_eq!(executors[0].extra["x-gpu"], "a100");
    let inputs = task.inputs.as_ref().unwrap();
    assert_eq!(
        inputs[0].r#type,
        Some(tes::models::TesFileType::Unrecognized("SYMLINK".into()))
    );
    // Tags stay a plain map of strings.
    let tags: &std::collections::HashMap<String, String> = task.tags.as_ref().unwrap();
    assert_eq!(tags["project"], "demo");
}

#[test]
fn tes_unknown_states_read_as_unknown() {
    let task: tes::models::TesTask =
        serde_json::from_value(json!({ "id": "task-1", "state": "SUSPENDED" })).unwrap();
    assert_eq!(task.state, Some(tes::models::TesState::Unknown));
    // Unlike other enums, the state doesn't keep the server's value.
    assert_eq!(serde_json::to_value(&task).unwrap()["state"], "UNKNOWN");
    let list: tes::models::TesListTasksResponse = serde_json::from_value(json!({
        "tasks": [{ "id": "a", "state": "RUNNING" }, { "id": "b", "state": "SUSPENDED" }]
    }))
    .unwrap();
    let states: Vec<_> = list.tasks.iter().map(|task| task.state).collect();
    assert_eq!(
        states,
        [
            Some(tes::models::TesState::Running),
            Some(tes::models::TesState::Unknown)
        ]
    );
}

#[test]
fn tes_task_new_keeps_its_executors() {
    let executor = tes::models::TesExecutor::new("alpine".into(), vec!["true".into()]);
    let task = tes::models::TesTask::new(vec![executor.clone()]);
    assert_eq!(task.executors, Some(vec![executor]));
    assert!(task.extra.is_empty());
}

#[test]
fn tes_service_info_keeps_extra_fields_and_unknown_artifacts() {
    let mut body = service_info_body("tes-plus");
    body["storage"] = json!(["s3://bucket"]);
    let info: tes::models::TesServiceInfo = round_trip(body);
    assert_eq!(
        info.r#type.artifact,
        tes::models::tes_service_type::Artifact::Unrecognized("tes-plus".into())
    );
    assert_eq!(info.extra["x-vendor"], json!({ "feature": true }));
}

#[test]
fn wes_run_log_keeps_extra_fields_and_reads_unknown_states_as_unknown() {
    let body = json!({
        "run_id": "run-1",
        "state": "COMPLETE",
        "run_log": { "name": "main", "x-host": "node-1" },
        "outputs": {},
        "x-cost": 1.5
    });
    let log: wes::models::RunLog = round_trip(body);
    assert_eq!(log.extra["x-cost"], 1.5);
    assert_eq!(log.run_log.unwrap().extra["x-host"], "node-1");

    let status: wes::models::RunStatus =
        serde_json::from_value(json!({ "run_id": "run-1", "state": "ARCHIVED" })).unwrap();
    assert_eq!(status.state, Some(wes::models::State::Unknown));
}

#[test]
fn wes_run_request_keeps_extra_fields() {
    let body = json!({
        "workflow_params": {},
        "workflow_type": "CWL",
        "workflow_type_version": "v1.2",
        "workflow_url": "main.cwl",
        "x-priority": "high"
    });
    let request: wes::models::RunRequest = round_trip(body);
    assert_eq!(request.extra["x-priority"], "high");
}

#[test]
fn drs_object_keeps_extra_fields_and_unknown_access_types() {
    let body = json!({
        "id": "obj-1",
        "self_uri": "drs://example.org/obj-1",
        "size": 10,
        "created_time": "2026-11-10T00:00:00Z",
        "checksums": [{ "checksum": "abc", "type": "sha-256", "x-algo-note": "n" }],
        "access_methods": [{
            "type": "azure",
            "access_url": { "url": "https://example.org/obj-1", "x-expires": 60 },
            "authorizations": { "supported_types": ["BearerAuth", "MagicAuth"] }
        }],
        "x-retention": "30d"
    });
    let object: drs::models::DrsObject = round_trip(body);
    assert_eq!(object.extra["x-retention"], "30d");
    let method = &object.access_methods.as_ref().unwrap()[0];
    assert_eq!(
        method.r#type,
        drs::models::access_method::Type::Unrecognized("azure".into())
    );
    assert_eq!(method.access_url.as_ref().unwrap().extra["x-expires"], 60);
    let supported = method
        .authorizations
        .as_ref()
        .unwrap()
        .supported_types
        .clone()
        .unwrap();
    assert_eq!(
        supported[1],
        drs::models::access_method_authorizations::SupportedTypes::Unrecognized("MagicAuth".into())
    );
}

#[test]
fn trs_tool_keeps_extra_fields_and_unknown_descriptor_types() {
    let body = json!({
        "url": "https://trs.example.org/tools/1",
        "id": "1",
        "organization": "Example",
        "toolclass": { "id": "1", "name": "Workflow", "x-class": true },
        "versions": [{
            "url": "https://trs.example.org/tools/1/versions/1",
            "id": "1",
            "descriptor_type": ["CWL", "JUPYTER"],
            "x-doi": "10.1234/x"
        }],
        "x-stars": 5
    });
    let tool: trs::models::Tool = round_trip(body);
    assert_eq!(tool.extra["x-stars"], 5);
    assert_eq!(tool.toolclass.extra["x-class"], true);
    assert_eq!(
        tool.versions[0].descriptor_type,
        Some(vec![
            trs::models::DescriptorType::Cwl,
            trs::models::DescriptorType::Unrecognized("JUPYTER".into())
        ])
    );
}

#[test]
fn trs_file_wrapper_reads_known_values_of_both_image_type_enums() {
    use trs::models::{DescriptorType, FileWrapper, FileWrapperImageType, ImageType};
    for (value, expected) in [
        ("Docker", FileWrapperImageType::ImageType(ImageType::Docker)),
        (
            "CWL",
            FileWrapperImageType::DescriptorType(DescriptorType::Cwl),
        ),
        (
            "Podman",
            FileWrapperImageType::ImageType(ImageType::Unrecognized("Podman".into())),
        ),
        // Unknown to both enums, even if it looks like a descriptor type.
        (
            "PLAIN_JUPYTER",
            FileWrapperImageType::ImageType(ImageType::Unrecognized("PLAIN_JUPYTER".into())),
        ),
    ] {
        let file: FileWrapper =
            round_trip(json!({ "content": "x", "image_type": value, "x-size": 1 }));
        assert_eq!(*file.image_type.unwrap(), expected, "{value}");
        assert_eq!(file.extra["x-size"], 1);
    }
    assert!(serde_json::from_value::<FileWrapper>(json!({ "image_type": 1 })).is_err());
}

#[test]
fn trs_tool_file_keeps_unknown_file_types() {
    use trs::models::tool_file::FileType;
    let file: trs::models::ToolFile = round_trip(json!({ "path": "a", "file_type": "OTHER" }));
    assert_eq!(file.file_type, Some(FileType::Other));
    let file: trs::models::ToolFile = round_trip(json!({ "path": "a", "file_type": "LOCKFILE" }));
    assert_eq!(
        file.file_type,
        Some(FileType::Unrecognized("LOCKFILE".into()))
    );
}

#[test]
fn service_info_keeps_extra_fields() {
    let info: serviceinfo::models::Service = round_trip(service_info_body("beacon"));
    assert_eq!(info.extra["x-vendor"], json!({ "feature": true }));
    assert_eq!(info.r#type.extra["x-type-note"], "nested");
    assert_eq!(info.organization.extra["x-org"], 1);
}

#[test]
fn registry_service_keeps_extra_fields() {
    let mut body = service_info_body("wes");
    body["url"] = json!("https://example.org/ga4gh/wes/v1");
    let service: registry::models::ExternalService = round_trip(body);
    assert_eq!(service.extra["x-vendor"], json!({ "feature": true }));
}

#[test]
fn unrecognized_enum_values_display_as_received() {
    assert_eq!(
        trs::models::DescriptorType::Unrecognized("JUPYTER".into()).to_string(),
        "JUPYTER"
    );
    assert_eq!(trs::models::DescriptorType::Cwl.to_string(), "CWL");
    // Enums of a single property have `Display` too.
    assert_eq!(drs::models::access_method::Type::S3.to_string(), "s3");
    assert_eq!(
        drs::models::access_method::Type::Unrecognized("azure".into()).to_string(),
        "azure"
    );
}
