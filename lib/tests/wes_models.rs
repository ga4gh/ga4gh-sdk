//! Serde round trips of the WES 1.1.0 models.
//!
//! The bodies follow the schemas of the WES 1.1.0 OpenAPI spec
//! (`ga4gh/workflow-execution-service-schemas`, tag `1.1.0`). A model
//! passes if reading a body and writing it back gives the same JSON.

use ga4gh_sdk::clients::wes::models::{
    ErrorResponse, RunId, RunListResponse, RunLog, RunRequest, RunStatus, RunSummary, ServiceInfo,
    State, TaskListResponse, TaskLog,
};
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

fn log_body() -> Value {
    json!({
        "name": "main",
        "cmd": ["nextflow", "run", "main.nf"],
        "start_time": "2026-11-09T10:00:00Z",
        "end_time": "2026-11-09T10:05:00Z",
        "stdout": "https://wes.example.org/logs/stdout",
        "stderr": "https://wes.example.org/logs/stderr",
        "exit_code": 0,
        "system_logs": ["pulled image"]
    })
}

fn task_log_body() -> Value {
    json!({
        "id": "task-1",
        "name": "align",
        "cmd": ["bwa", "mem"],
        "start_time": "2026-11-09T10:01:00Z",
        "end_time": "2026-11-09T10:02:00Z",
        "stdout": "https://wes.example.org/tasks/1/stdout",
        "stderr": "https://wes.example.org/tasks/1/stderr",
        "exit_code": 0,
        "system_logs": ["node-7"],
        "tes_uri": "https://tes.example.org/ga4gh/tes/v1/tasks/abc"
    })
}

fn run_request_body() -> Value {
    json!({
        "workflow_params": { "input": "s3://bucket/in.txt" },
        "workflow_type": "CWL",
        "workflow_type_version": "v1.2",
        "tags": { "project": "biohackathon" },
        "workflow_engine_parameters": { "--cpus": "2" },
        "workflow_engine": "cwltool",
        "workflow_engine_version": "3.1",
        "workflow_url": "https://example.org/workflow.cwl"
    })
}

#[test]
fn state_round_trips_all_values() {
    let values = [
        ("UNKNOWN", State::Unknown),
        ("QUEUED", State::Queued),
        ("INITIALIZING", State::Initializing),
        ("RUNNING", State::Running),
        ("PAUSED", State::Paused),
        ("COMPLETE", State::Complete),
        ("EXECUTOR_ERROR", State::ExecutorError),
        ("SYSTEM_ERROR", State::SystemError),
        ("CANCELED", State::Canceled),
        ("CANCELING", State::Canceling),
        ("PREEMPTED", State::Preempted),
    ];
    for (text, state) in values {
        assert_eq!(round_trip::<State>(json!(text)), state);
        assert_eq!(state.to_string(), text);
    }
    assert_eq!(State::default(), State::Unknown);
}

#[test]
fn run_id_round_trips() {
    let run_id = round_trip::<RunId>(json!({ "run_id": "run-1" }));
    assert_eq!(run_id.run_id.as_deref(), Some("run-1"));
}

#[test]
fn run_status_round_trips() {
    let status = round_trip::<RunStatus>(json!({ "run_id": "run-1", "state": "PREEMPTED" }));
    assert_eq!(status.run_id, "run-1");
    assert_eq!(status.state, Some(State::Preempted));
}

#[test]
fn run_status_requires_run_id() {
    assert!(serde_json::from_value::<RunStatus>(json!({ "state": "RUNNING" })).is_err());
}

#[test]
fn run_summary_round_trips() {
    let summary = round_trip::<RunSummary>(json!({
        "run_id": "run-1",
        "state": "COMPLETE",
        "start_time": "2026-11-09T10:00:00Z",
        "end_time": "2026-11-09T10:05:00Z",
        "tags": { "project": "biohackathon" }
    }));
    assert_eq!(summary.tags["project"], "biohackathon");
}

#[test]
fn run_request_round_trips_with_string_maps() {
    let request = round_trip::<RunRequest>(run_request_body());
    assert_eq!(request.tags.unwrap()["project"], "biohackathon");
    assert_eq!(request.workflow_engine_parameters.unwrap()["--cpus"], "2");
}

#[test]
fn run_request_rejects_non_string_tags() {
    let mut body = run_request_body();
    body["tags"] = json!({ "count": 1 });
    assert!(serde_json::from_value::<RunRequest>(body).is_err());
}

#[test]
fn run_list_response_round_trips_status_and_summary_items() {
    let list = round_trip::<RunListResponse>(json!({
        "runs": [
            { "run_id": "run-1", "state": "RUNNING" },
            {
                "run_id": "run-2",
                "state": "COMPLETE",
                "start_time": "2026-11-09T10:00:00Z",
                "end_time": "2026-11-09T10:05:00Z",
                "tags": { "project": "biohackathon" }
            }
        ],
        "next_page_token": "page-2"
    }));
    let runs = list.runs.unwrap();
    assert_eq!(runs[0].tags, None);
    assert_eq!(runs[1].tags.as_ref().unwrap()["project"], "biohackathon");
    assert_eq!(list.next_page_token.as_deref(), Some("page-2"));
}

#[test]
fn run_log_round_trips_with_task_logs_url() {
    let log = round_trip::<RunLog>(json!({
        "run_id": "run-1",
        "request": run_request_body(),
        "state": "COMPLETE",
        "run_log": log_body(),
        "task_logs_url": "https://wes.example.org/ga4gh/wes/v1/runs/run-1/tasks",
        "outputs": { "result.txt": "s3://bucket/result.txt" }
    }));
    assert_eq!(log.request.unwrap().workflow_type, "CWL");
    assert_eq!(log.run_log.unwrap().exit_code, Some(0));
    assert!(log.task_logs.is_none());
}

#[test]
fn run_log_round_trips_deprecated_log_and_task_log_items() {
    let log = round_trip::<RunLog>(json!({
        "run_id": "run-1",
        "state": "RUNNING",
        "task_logs": [log_body(), task_log_body()]
    }));
    let task_logs = log.task_logs.unwrap();
    assert_eq!(task_logs[0].id, None);
    assert_eq!(task_logs[0].name.as_deref(), Some("main"));
    assert_eq!(task_logs[1].id.as_deref(), Some("task-1"));
    assert_eq!(
        task_logs[1].tes_uri.as_deref(),
        Some("https://tes.example.org/ga4gh/tes/v1/tasks/abc")
    );
}

#[test]
fn run_log_reads_partial_request_as_none() {
    let mut request = run_request_body();
    request["workflow_type_version"] = Value::Null;
    let log: RunLog = serde_json::from_value(json!({
        "run_id": "run-1",
        "request": request,
        "state": "COMPLETE",
        "outputs": {}
    }))
    .unwrap();
    assert!(log.request.is_none());
    assert_eq!(log.state, Some(State::Complete));
}

#[test]
fn run_log_reads_null_task_logs() {
    let log: RunLog =
        serde_json::from_value(json!({ "run_id": "run-1", "task_logs": null })).unwrap();
    assert!(log.task_logs.is_none());
}

#[test]
fn task_log_round_trips() {
    let task = round_trip::<TaskLog>(task_log_body());
    assert_eq!(task.id, "task-1");
    assert_eq!(task.name, "align");
}

#[test]
fn task_log_requires_id_and_name() {
    assert!(serde_json::from_value::<TaskLog>(json!({ "name": "align" })).is_err());
    assert!(serde_json::from_value::<TaskLog>(json!({ "id": "task-1" })).is_err());
}

#[test]
fn task_list_response_round_trips() {
    let list = round_trip::<TaskListResponse>(json!({
        "task_logs": [task_log_body()],
        "next_page_token": ""
    }));
    assert_eq!(list.task_logs.unwrap()[0].id, "task-1");
    assert_eq!(list.next_page_token.as_deref(), Some(""));
}

#[test]
fn service_info_round_trips() {
    let info = round_trip::<ServiceInfo>(json!({
        "id": "org.example.wes",
        "name": "Example WES",
        "type": { "group": "org.ga4gh", "artifact": "wes", "version": "1.1.0" },
        "description": "WES for tests",
        "organization": { "name": "Example", "url": "https://example.org" },
        "contactUrl": "mailto:wes@example.org",
        "documentationUrl": "https://example.org/docs",
        "createdAt": "2026-01-01T00:00:00Z",
        "updatedAt": "2026-11-01T00:00:00Z",
        "environment": "test",
        "version": "1.0.0",
        "workflow_type_versions": {
            "CWL": { "workflow_type_version": ["v1.0", "v1.2"] }
        },
        "supported_wes_versions": ["1.1.0"],
        "supported_filesystem_protocols": ["https", "s3"],
        "workflow_engine_versions": {
            "cwltool": { "workflow_engine_version": ["3.1"] }
        },
        "default_workflow_engine_parameters": [
            { "name": "--cpus", "type": "int", "default_value": "1" }
        ],
        "system_state_counts": { "RUNNING": 2, "COMPLETE": 10 },
        "auth_instructions_url": "https://example.org/auth",
        "tags": { "deployment": "test" }
    }));
    assert_eq!(info.r#type.artifact, "wes");
    assert_eq!(
        info.workflow_engine_versions["cwltool"]
            .workflow_engine_version
            .as_ref()
            .unwrap()[0],
        "3.1"
    );
    assert_eq!(info.system_state_counts["COMPLETE"], 10);
}

#[test]
fn error_response_round_trips() {
    let error = round_trip::<ErrorResponse>(json!({ "msg": "not found", "status_code": 404 }));
    assert_eq!(error.status_code, Some(404));
}
