pub mod attachments;
pub mod models;

use crate::clients::serviceinfo::models::Service;
use crate::clients::wes::models::RunId;
use crate::clients::wes::models::RunListResponse;
use crate::clients::wes::models::RunListResponseRunsInner;
use crate::clients::wes::models::RunLog;
use crate::clients::wes::models::RunRequest;
use crate::clients::wes::models::State;
use crate::clients::wes::models::TaskListResponse;
use crate::clients::wes::models::TaskLog;
use crate::utils::configuration::Configuration;
/// Existing import path for the shared path-segment encoder.
pub use crate::utils::transport::urlencode;
use crate::utils::transport::{HttpStatusError, Transport};
use log::error;
use serde_json::from_str;
use serde_json::json;

/// A file included in a WES RunWorkflow multipart request.
#[derive(Debug, Clone)]
pub struct WorkflowAttachment {
    pub filename: String,
    pub content: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Run {
    /// The unique ID of the run.
    pub id: String,
    /// The transport layer for sending HTTP requests.
    pub transport: Transport,
}

impl Run {
    /// Creates a new `Run` instance.
    pub fn new(id: String, transport: Transport) -> Self {
        Run { id, transport }
    }

    /// Fetches the current status of the run.
    pub async fn status(&self) -> Result<State, Box<dyn std::error::Error>> {
        let run_id = &self.id;
        let url = format!("runs/{}/status", urlencode(run_id));
        let response = self.transport.get(&url, None).await;
        match response {
            Ok(resp_str) => {
                let status: serde_json::Value = from_str(&resp_str)?;
                // The spec says GET /runs/{run_id}/status returns RunStatus which has run_id and state.
                let state_str = status
                    .get("state")
                    .and_then(|s| s.as_str())
                    .ok_or("Missing state field")?;
                let state: State = serde_json::from_value(json!(state_str))?;
                Ok(state)
            }
            Err(e) => {
                let err_msg = format!("HTTP request failed: {}", e);
                error!("{}", err_msg);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    err_msg,
                )))
            }
        }
    }

    /// Cancels the run.
    pub async fn cancel(&self) -> Result<RunId, Box<dyn std::error::Error>> {
        let id = &self.id;
        let id = urlencode(id);
        let url = format!("runs/{}/cancel", id);
        let response = self.transport.post(&url, None).await;
        match response {
            Ok(resp_str) => {
                crate::utils::acknowledgement::execution_id(
                    &resp_str,
                    "run_id",
                    "WES cancellation",
                    Some(&self.id),
                )?;
                let run_id: RunId = from_str(&resp_str)?;
                Ok(run_id)
            }
            Err(e) => Err(format!("HTTP request failed: {}", e).into()),
        }
    }

    /// Fetches the details of the run (including logs and outputs).
    pub async fn log(&self) -> Result<RunLog, Box<dyn std::error::Error>> {
        let id = &self.id;
        let url = format!("runs/{}", urlencode(id));
        let response = self.transport.get(&url, None).await;
        match response {
            Ok(resp_str) => {
                let log: RunLog = from_str(&resp_str)?;
                Ok(log)
            }
            Err(e) => Err(e),
        }
    }

    /// Lists the tasks of the run, one page at a time (WES 1.1 `ListTasks`).
    pub async fn list_tasks(
        &self,
        page_token: Option<String>,
        page_size: Option<i64>,
    ) -> Result<TaskListResponse, Box<dyn std::error::Error>> {
        let path = format!("runs/{}/tasks", urlencode(&self.id));
        let resp_str = self
            .transport
            .get(&path, paging_params(page_token, page_size))
            .await?;
        Ok(from_str(&resp_str)?)
    }

    /// Fetches one task of the run (WES 1.1 `GetTask`).
    pub async fn get_task(&self, task_id: &str) -> Result<TaskLog, Box<dyn std::error::Error>> {
        // An empty ID would address the task list instead.
        if task_id.is_empty() {
            return Err("task ID must not be empty".into());
        }
        let path = format!("runs/{}/tasks/{}", urlencode(&self.id), urlencode(task_id));
        let resp_str = self.transport.get(&path, None).await?;
        Ok(from_str(&resp_str)?)
    }
}

// Builds the query parameters of the WES list endpoints.
fn paging_params(page_token: Option<String>, page_size: Option<i64>) -> Option<serde_json::Value> {
    let mut params = serde_json::Map::new();
    if let Some(token) = page_token {
        params.insert("page_token".to_string(), json!(token));
    }
    if let Some(size) = page_size {
        params.insert("page_size".to_string(), json!(size));
    }
    if params.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(params))
    }
}

/// The main struct for interacting with a WES service.
#[derive(Debug)]
pub struct WES {
    #[allow(dead_code)]
    pub config: Configuration,
    pub service: Result<Service, Box<dyn std::error::Error>>,
    pub transport: Transport,
}

impl WES {
    /// Creates a new `WES` instance.
    pub async fn new(config: &Configuration) -> Result<Self, Box<dyn std::error::Error>> {
        let transport = Transport::new(config);
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            transport.get("service-info", None),
        )
        .await?;
        let service: Result<Service, Box<dyn std::error::Error>> =
            response.and_then(|body| from_str(&body).map_err(Into::into));

        let instance = WES {
            config: config.clone(),
            transport,
            service,
        };

        instance.check()?;
        Ok(instance)
    }

    fn check(&self) -> Result<(), String> {
        let resp = &self.service;
        match resp.as_ref() {
            Ok(service) if service.r#type.artifact == "wes" => Ok(()),
            Ok(_) => Err("The endpoint is not an instance of WES".into()),
            Err(error) => {
                let mut message = format!("Error accessing WES service: {error}");
                if error
                    .downcast_ref::<HttpStatusError>()
                    .is_some_and(|error| error.status == reqwest::StatusCode::NOT_FOUND)
                {
                    message.push_str(". Check that base_path is the WES API root");
                }
                Err(message)
            }
        }
    }

    /// Creates a new workflow run.
    pub async fn run_workflow(
        &self,
        request: RunRequest,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        self.run_workflow_with_attachments(request, Vec::new())
            .await
    }

    /// Creates a workflow run with optional uploaded workflow files.
    pub async fn run_workflow_with_attachments(
        &self,
        request: RunRequest,
        attachments: Vec<WorkflowAttachment>,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        let attachments = attachments
            .into_iter()
            .map(|file| attachments::StreamingAttachment {
                filename: file.filename,
                source: attachments::AttachmentSource::Bytes(file.content),
            })
            .collect();
        self.run_workflow_streaming(
            request,
            attachments,
            &crate::utils::operation::Operation::new(std::time::Duration::from_secs(60)),
        )
        .await
    }

    /// List runs.
    pub async fn list_runs(
        &self,
        next_page_token: Option<String>,
        page_size: Option<i64>,
    ) -> Result<RunListResponse, Box<dyn std::error::Error>> {
        let response = self
            .transport
            .get("runs", paging_params(next_page_token, page_size))
            .await;

        match response {
            Ok(resp_str) => {
                // Starter Kit WES 0.2.0 returns a bare array, while WES 1.0.1
                // and 1.1.0 specify an object with `runs` and `next_page_token`.
                #[derive(serde::Deserialize)]
                #[serde(untagged)]
                enum RunListBody {
                    Standard(RunListResponse),
                    StarterKit(Vec<RunListResponseRunsInner>),
                }
                let list = match from_str::<RunListBody>(&resp_str)? {
                    RunListBody::Standard(list) => list,
                    RunListBody::StarterKit(runs) => RunListResponse {
                        runs: Some(runs),
                        next_page_token: None,
                    },
                };
                Ok(list)
            }
            Err(e) => {
                error!("HTTP request failed: {:?}", e);
                Err(Box::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("HTTP request failed: {:?}", e),
                )))
            }
        }
    }

    /// Get run full details (wrapper around Run::log for convenience from client)
    pub async fn get_run(&self, id: &str) -> Result<RunLog, Box<dyn std::error::Error>> {
        let run = Run::new(id.to_string(), self.transport.clone());
        run.log().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::serviceinfo::models::ServiceType;
    use mockito::server_url;
    use mockito::{mock, Matcher};
    use url::Url;

    #[test]
    fn test_debug_output() {
        let config = crate::utils::configuration::config_with_markers();
        let transport = Transport::new(&config);
        let run = Run::new("123".to_string(), transport.clone());
        let wes = WES {
            config,
            service: Ok(Service::default()),
            transport,
        };
        for output in [format!("{:?}", run), format!("{:?}", wes)] {
            assert!(!output.contains("SENTINEL"), "{output}");
        }
    }

    fn valid_request() -> RunRequest {
        RunRequest {
            workflow_params: Some(json!({})),
            ..RunRequest::new(
                "CWL".to_string(),
                "v1.2".to_string(),
                "https://example.org/workflow.cwl".to_string(),
            )
        }
    }

    #[test]
    fn test_endpoint_preserves_encoded_base_path() {
        let base = Url::parse("https://example.org/a%20b/wes/v1").unwrap();
        let config = Configuration::new(base);
        assert_eq!(
            Transport::new(&config).endpoint("runs").unwrap().as_str(),
            "https://example.org/a%20b/wes/v1/runs"
        );
    }

    #[tokio::test]
    async fn test_wes_uses_configured_api_prefix() {
        for suffix in ["", "/"] {
            let _service_info = mock("GET", "/ga4gh/wes/v1/service-info")
                .with_status(200)
                .with_body(
                    serde_json::json!({
                        "id": "org.example.wes",
                        "name": "Example WES",
                        "type": { "group": "org.ga4gh", "artifact": "wes", "version": "1.1.0" },
                        "organization": { "name": "Example", "url": "https://example.org" },
                        "version": "1.0.0"
                    })
                    .to_string(),
                )
                .create();
            let _create = mock("POST", "/ga4gh/wes/v1/runs")
                .match_header(
                    "content-type",
                    Matcher::Regex("^multipart/form-data; boundary=".to_string()),
                )
                .with_status(200)
                .with_body(r#"{"run_id":"abc"}"#)
                .create();
            let _status = mock("GET", "/ga4gh/wes/v1/runs/abc/status")
                .with_status(200)
                .with_body(r#"{"run_id":"abc","state":"COMPLETE"}"#)
                .create();

            let base = Url::parse(&format!("{}/ga4gh/wes/v1{suffix}", server_url())).unwrap();
            let wes = WES::new(&Configuration::new(base)).await.unwrap();
            let run = wes.run_workflow(valid_request()).await.unwrap();
            assert_eq!(run.status().await.unwrap(), State::Complete);
        }
    }

    #[tokio::test]
    async fn test_service_info_error_keeps_status_and_404_hint() {
        for (status, path) in [(404, "/missing/wes/v1"), (503, "/unavailable/wes/v1")] {
            let _service = mock("GET", format!("{path}/service-info").as_str())
                .with_status(status)
                .with_body("service failure")
                .create();
            let base = Url::parse(&format!("{}{path}", server_url())).unwrap();
            let error = WES::new(&Configuration::new(base))
                .await
                .unwrap_err()
                .to_string();
            assert!(error.contains(&status.to_string()), "{error}");
            assert!(error.contains("service failure"), "{error}");
            assert_eq!(error.contains("base_path"), status == 404, "{error}");
        }
    }

    #[tokio::test]
    async fn test_wes_create() {
        let _m = mock("POST", "/runs")
            .match_header(
                "content-type",
                Matcher::Regex("^multipart/form-data; boundary=".to_string()),
            )
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex("name=\"workflow_type\"[\\s\\S]*CWL".to_string()),
                Matcher::Regex("name=\"workflow_type_version\"[\\s\\S]*v1.2".to_string()),
                Matcher::Regex("name=\"workflow_params\"[\\s\\S]*\\{\\}".to_string()),
                Matcher::Regex("name=\"workflow_url\"[\\s\\S]*workflow.cwl".to_string()),
            ]))
            .with_status(200)
            .with_body(r#"{"run_id": "123"}"#)
            .create();

        let mock_url = Url::parse(&server_url()).expect("Invalid URL");
        let config = Configuration::new(mock_url);
        let transport = Transport::new(&config);

        let wes = WES {
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "wes".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
            transport,
        };

        let result = wes.run_workflow(valid_request()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, String::from("123"));
    }

    #[tokio::test]
    async fn test_wes_rejects_missing_run_id() {
        let _m = mock("POST", "/runs")
            .with_status(200)
            .with_body("{}")
            .create();
        let config = Configuration::new(Url::parse(&server_url()).unwrap());
        let wes = WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "wes".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
        };
        assert!(wes.run_workflow(valid_request()).await.is_err());
    }

    #[tokio::test]
    async fn test_wes_uploads_workflow_attachment() {
        let _m = mock("POST", "/runs")
            .match_body(Matcher::Regex(
                "name=\"workflow_attachment\"; filename=\"workflow.cwl\"[\\s\\S]*cwlVersion"
                    .to_string(),
            ))
            .with_status(200)
            .with_body(r#"{"run_id":"uploaded"}"#)
            .create();
        let config = Configuration::new(Url::parse(&server_url()).unwrap());
        let wes = WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "wes".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
        };
        let request = RunRequest {
            workflow_url: "workflow.cwl".to_string(),
            ..valid_request()
        };
        let attachments = vec![WorkflowAttachment {
            filename: "workflow.cwl".to_string(),
            content: b"cwlVersion: v1.2".to_vec(),
        }];
        assert_eq!(
            wes.run_workflow_with_attachments(request, attachments)
                .await
                .unwrap()
                .id,
            "uploaded"
        );
    }

    #[tokio::test]
    async fn test_run_status() {
        let _m = mock("GET", "/runs/123/status")
            .with_status(200)
            .with_body(r#"{"run_id": "123", "state": "COMPLETE"}"#)
            .create();

        let mock_url = Url::parse(&server_url()).expect("Invalid URL");
        let transport = Transport::new(&Configuration::new(mock_url));
        let run = Run::new("123".to_string(), transport);

        let result = run.status().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), State::Complete);
    }

    #[tokio::test]
    async fn test_run_id_is_encoded_as_path_segment() {
        let _status = mock("GET", "/ga4gh/wes/v1/runs/a%2Fb%20%3F%23/status")
            .with_status(200)
            .with_body(r#"{"run_id":"a/b ?#","state":"COMPLETE"}"#)
            .create();
        let _log = mock("GET", "/ga4gh/wes/v1/runs/a%2Fb%20%3F%23")
            .with_status(200)
            .with_body(r#"{"run_id":"a/b ?#","state":"COMPLETE"}"#)
            .create();
        let _cancel = mock("POST", "/ga4gh/wes/v1/runs/a%2Fb%20%3F%23/cancel")
            .with_status(200)
            .with_body(r#"{"run_id":"a/b ?#"}"#)
            .create();
        let base = Url::parse(&format!("{}/ga4gh/wes/v1/", server_url())).unwrap();
        let run = Run::new(
            "a/b ?#".to_string(),
            Transport::new(&Configuration::new(base)),
        );
        assert_eq!(run.status().await.unwrap(), State::Complete);
        assert_eq!(run.log().await.unwrap().run_id.as_deref(), Some("a/b ?#"));
        assert_eq!(
            run.cancel().await.unwrap().run_id.as_deref(),
            Some("a/b ?#")
        );
    }

    #[tokio::test]
    async fn test_run_cancel() {
        let _m = mock("POST", "/runs/123/cancel")
            .with_status(200)
            .with_body(r#"{"run_id": "123"}"#)
            .create();

        let mock_url = Url::parse(&server_url()).expect("Invalid URL");
        let transport = Transport::new(&Configuration::new(mock_url));
        let run = Run::new("123".to_string(), transport);

        let result = run.cancel().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().run_id, Some("123".to_string()));
    }

    #[tokio::test]
    async fn test_starter_kit_unimplemented_cancel_is_explicit() {
        let _m = mock("POST", "/runs/123/cancel")
            .with_status(200)
            .with_body("null")
            .create();
        let config = Configuration::new(Url::parse(&server_url()).unwrap());
        let run = Run::new("123".to_string(), Transport::new(&config));
        let err = run.cancel().await.unwrap_err();
        assert!(err.to_string().contains("cancellation cannot be confirmed"));
    }

    #[tokio::test]
    async fn test_wes_list_runs() {
        let _m = mock("GET", "/runs")
            .with_status(200)
            .with_body(r#"{"runs": [], "next_page_token": ""}"#)
            .create();

        let mock_url = Url::parse(&server_url()).expect("Invalid URL");
        let config = Configuration::new(mock_url);
        let transport = Transport::new(&config);
        let wes = WES {
            config,
            service: Ok(Service::default()),
            transport,
        };

        let result = wes.list_runs(None, None).await;
        assert!(result.is_ok());
        let runs = result.unwrap().runs;
        assert!(runs.is_some());
        assert!(runs.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_starter_kit_list_runs_array() {
        let _m = mock("GET", "/runs")
            .with_status(200)
            .with_body(r#"[{"run_id":"123","state":"COMPLETE"}]"#)
            .create();
        let config = Configuration::new(Url::parse(&server_url()).unwrap());
        let wes = WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service::default()),
        };
        let list = wes.list_runs(None, None).await.unwrap();
        let runs = list.runs.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, "123");
        assert_eq!(runs[0].state, Some(State::Complete));
    }

    #[tokio::test]
    async fn test_wes_encodes_page_token() {
        let _m = mock("GET", "/ga4gh/wes/v1/runs")
            .match_query(Matcher::AllOf(vec![
                Matcher::Regex("(^|&)page_token=a%2Bb%26c(&|$)".to_string()),
                Matcher::Regex("(^|&)page_size=5(&|$)".to_string()),
            ]))
            .with_status(200)
            .with_body(r#"{"runs":[]}"#)
            .create();
        let config =
            Configuration::new(Url::parse(&format!("{}/ga4gh/wes/v1/", server_url())).unwrap());
        let wes = WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service::default()),
        };
        assert!(wes
            .list_runs(Some("a+b&c".to_string()), Some(5))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn test_wes_get_run() {
        let _m = mock("GET", "/runs/123")
            .with_status(200)
            .with_body(r#"{"run_id": "123", "state": "COMPLETE"}"#)
            .create();

        let mock_url = Url::parse(&server_url()).expect("Invalid URL");
        let config = Configuration::new(mock_url);
        let transport = Transport::new(&config);
        let wes = WES {
            config,
            service: Ok(Service::default()),
            transport,
        };

        let result = wes.get_run("123").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().run_id, Some("123".to_string()));
    }

    fn wes_for(base: &str) -> WES {
        let config = Configuration::new(Url::parse(base).unwrap());
        WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "wes".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
        }
    }

    #[tokio::test]
    async fn test_wes_sends_tags_and_engine_parameters_as_json() {
        let _m = mock("POST", "/runs")
            .match_body(Matcher::AllOf(vec![
                Matcher::Regex(r#"name="tags"\r\n\r\n\{"project":"biohackathon"\}"#.to_string()),
                Matcher::Regex(
                    r#"name="workflow_engine_parameters"\r\n\r\n\{"--cpus":"2"\}"#.to_string(),
                ),
                Matcher::Regex(r#"name="workflow_engine"\r\n\r\ncwltool"#.to_string()),
            ]))
            .with_status(200)
            .with_body(r#"{"run_id":"tagged"}"#)
            .create();
        let request = RunRequest {
            tags: Some([("project".to_string(), "biohackathon".to_string())].into()),
            workflow_engine_parameters: Some([("--cpus".to_string(), "2".to_string())].into()),
            workflow_engine: Some("cwltool".to_string()),
            ..valid_request()
        };
        assert_eq!(
            wes_for(&server_url())
                .run_workflow(request)
                .await
                .unwrap()
                .id,
            "tagged"
        );
    }

    #[tokio::test]
    async fn test_wes_rejects_empty_required_fields() {
        let wes = wes_for(&server_url());
        for (field, request) in [
            (
                "workflow_type",
                RunRequest {
                    workflow_type: String::new(),
                    ..valid_request()
                },
            ),
            (
                "workflow_type_version",
                RunRequest {
                    workflow_type_version: String::new(),
                    ..valid_request()
                },
            ),
            (
                "workflow_url",
                RunRequest {
                    workflow_url: String::new(),
                    ..valid_request()
                },
            ),
            (
                "workflow_params",
                RunRequest {
                    workflow_params: None,
                    ..valid_request()
                },
            ),
        ] {
            let err = wes.run_workflow(request).await.unwrap_err();
            assert_eq!(err.to_string(), format!("{} is required", field));
        }
    }

    #[tokio::test]
    async fn test_wes_list_runs_reads_status_and_summary_items() {
        let _m = mock("GET", "/ga4gh/wes/v1/runs")
            .with_status(200)
            .with_body(r#"{"runs":[{"run_id":"1","state":"PREEMPTED"},{"run_id":"2","state":"COMPLETE","tags":{"a":"b"}}],"next_page_token":"next"}"#)
            .create();
        let wes = wes_for(&format!("{}/ga4gh/wes/v1", server_url()));
        let list = wes.list_runs(None, None).await.unwrap();
        let runs = list.runs.unwrap();
        assert_eq!(runs[0].state, Some(State::Preempted));
        assert_eq!(runs[1].tags.as_ref().unwrap()["a"], "b");
        assert_eq!(list.next_page_token.as_deref(), Some("next"));
    }

    #[tokio::test]
    async fn test_run_list_tasks() {
        let _m = mock("GET", "/ga4gh/wes/v1/runs/a%2Fb/tasks")
            .match_query(Matcher::AllOf(vec![
                Matcher::Regex("(^|&)page_token=a%2Bb%26c(&|$)".to_string()),
                Matcher::Regex("(^|&)page_size=2(&|$)".to_string()),
            ]))
            .with_status(200)
            .with_body(r#"{"task_logs":[{"id":"t1","name":"align","exit_code":0}],"next_page_token":"next"}"#)
            .create();
        let base = Url::parse(&format!("{}/ga4gh/wes/v1", server_url())).unwrap();
        let run = Run::new("a/b".to_string(), Transport::new(&Configuration::new(base)));
        let list = run
            .list_tasks(Some("a+b&c".to_string()), Some(2))
            .await
            .unwrap();
        let tasks = list.task_logs.unwrap();
        assert_eq!(tasks[0].id, "t1");
        assert_eq!(tasks[0].exit_code, Some(0));
        assert_eq!(list.next_page_token.as_deref(), Some("next"));
    }

    #[tokio::test]
    async fn test_run_list_tasks_without_paging() {
        let _m = mock("GET", "/runs/123/tasks")
            .match_query(Matcher::Missing)
            .with_status(200)
            .with_body(r#"{"task_logs":[]}"#)
            .create();
        let run = Run::new(
            "123".to_string(),
            Transport::new(&Configuration::new(Url::parse(&server_url()).unwrap())),
        );
        assert!(run
            .list_tasks(None, None)
            .await
            .unwrap()
            .task_logs
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn test_run_list_tasks_rejects_invalid_body() {
        let _m = mock("GET", "/runs/bad/tasks")
            .with_status(200)
            .with_body(r#"{"task_logs":[{"name":"no id"}]}"#)
            .create();
        let run = Run::new(
            "bad".to_string(),
            Transport::new(&Configuration::new(Url::parse(&server_url()).unwrap())),
        );
        assert!(run.list_tasks(None, None).await.is_err());
    }

    #[tokio::test]
    async fn test_run_task_methods_reject_invalid_ids() {
        let run = Run::new(
            "123".to_string(),
            Transport::new(&Configuration::new(Url::parse(&server_url()).unwrap())),
        );
        let err = run.get_task("").await.unwrap_err();
        assert_eq!(err.to_string(), "task ID must not be empty");
        // Transport rejects dot segments before sending a request.
        let is_endpoint_error = |err: Box<dyn std::error::Error>| {
            err.downcast_ref::<crate::utils::transport::EndpointError>()
                .is_some()
        };
        for id in [".", ".."] {
            assert!(is_endpoint_error(run.get_task(id).await.unwrap_err()));
            let run = Run::new(id.to_string(), run.transport.clone());
            assert!(is_endpoint_error(
                run.list_tasks(None, None).await.unwrap_err()
            ));
            assert!(is_endpoint_error(run.get_task("t1").await.unwrap_err()));
        }
    }

    #[tokio::test]
    async fn test_run_get_task() {
        let _m = mock("GET", "/ga4gh/wes/v1/runs/123/tasks/t%2F1")
            .with_status(200)
            .with_body(
                r#"{"id":"t/1","name":"align","tes_uri":"https://tes.example.org/v1/tasks/x"}"#,
            )
            .create();
        let base = Url::parse(&format!("{}/ga4gh/wes/v1/", server_url())).unwrap();
        let run = Run::new("123".to_string(), Transport::new(&Configuration::new(base)));
        let task = run.get_task("t/1").await.unwrap();
        assert_eq!(task.id, "t/1");
        assert_eq!(
            task.tes_uri.as_deref(),
            Some("https://tes.example.org/v1/tasks/x")
        );
    }

    #[tokio::test]
    async fn test_run_get_task_reports_http_errors() {
        let _m = mock("GET", "/runs/123/tasks/missing")
            .with_status(404)
            .with_body(r#"{"msg":"not found","status_code":404}"#)
            .create();
        let run = Run::new(
            "123".to_string(),
            Transport::new(&Configuration::new(Url::parse(&server_url()).unwrap())),
        );
        let err = run.get_task("missing").await.unwrap_err();
        assert!(err.to_string().contains("404"), "{err}");
    }
}
