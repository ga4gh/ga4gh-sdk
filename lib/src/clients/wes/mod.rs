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
use crate::utils::transport::Transport;
use log::error;
use reqwest::multipart::{Form, Part};
use serde_json::from_str;
use serde_json::json;

/// URL-encodes a string.
pub fn urlencode<T: AsRef<str>>(s: T) -> String {
    s.as_ref().as_bytes().iter().map(|byte| {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            (*byte as char).to_string()
        } else {
            format!("%{byte:02X}")
        }
    }).collect()
}

// Configuration.base_path is the WES API root, with or without a trailing slash.
fn endpoint(config: &Configuration, path: &str) -> Result<String, Box<dyn std::error::Error>> {
    let mut base = config.base_path.clone();
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    Ok(base.join(path)?.to_string())
}

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
        let url = endpoint(&self.transport.config, &format!("runs/{}/status", urlencode(run_id)))?;
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
        let url = endpoint(&self.transport.config, &format!("runs/{}/cancel", id))?;
        let response = self.transport.post(&url, None).await;
        match response {
            Ok(resp_str) => {
                if resp_str.trim().is_empty() || resp_str.trim() == "null" {
                    return Err(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "WES server returned no cancellation result; cancellation cannot be confirmed",
                    )));
                }
                let run_id: RunId = from_str(&resp_str)?;
                Ok(run_id)
            }
            Err(e) => Err(format!("HTTP request failed: {}", e).into()),
        }
    }

    /// Fetches the details of the run (including logs and outputs).
    pub async fn log(&self) -> Result<RunLog, Box<dyn std::error::Error>> {
        let id = &self.id;
        let url = endpoint(&self.transport.config, &format!("runs/{}", urlencode(id)))?;
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
        let path = paged_path(
            &format!("runs/{}/tasks", path_segment("run", &self.id)?),
            page_token,
            page_size,
        );
        let url = endpoint(&self.transport.config, &path)?;
        let resp_str = self.transport.get(&url, None).await?;
        Ok(from_str(&resp_str)?)
    }

    /// Fetches one task of the run (WES 1.1 `GetTask`).
    pub async fn get_task(&self, task_id: &str) -> Result<TaskLog, Box<dyn std::error::Error>> {
        let path = format!(
            "runs/{}/tasks/{}",
            path_segment("run", &self.id)?,
            path_segment("task", task_id)?
        );
        let url = endpoint(&self.transport.config, &path)?;
        let resp_str = self.transport.get(&url, None).await?;
        Ok(from_str(&resp_str)?)
    }
}

// Encodes an ID as one path segment. Empty, "." and ".." IDs are rejected:
// URL resolution would turn them into another endpoint.
fn path_segment(kind: &str, id: &str) -> Result<String, String> {
    if id.is_empty() || id == "." || id == ".." {
        return Err(format!("invalid {} ID: {:?}", kind, id));
    }
    Ok(urlencode(id))
}

// Appends the WES paging parameters to a path.
fn paged_path(path: &str, page_token: Option<String>, page_size: Option<i64>) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    if let Some(token) = page_token {
        query.append_pair("page_token", &token);
    }
    if let Some(size) = page_size {
        query.append_pair("page_size", &size.to_string());
    }
    let query = query.finish();
    if query.is_empty() {
        path.to_string()
    } else {
        format!("{}?{}", path, query)
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
        let service_info_url = endpoint(config, "service-info")?;
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            transport.get(&service_info_url, None),
        ).await??;
        let service: Service = from_str(&response)?;

        let instance = WES {
            config: config.clone(),
            transport,
            service: Ok(service),
        };

        instance.check()?;
        Ok(instance)
    }

    fn check(&self) -> Result<(), String> {
        let resp = &self.service;
        match resp.as_ref() {
            Ok(service) if service.r#type.artifact == "wes" => Ok(()),
            Ok(_) => Err("The endpoint is not an instance of WES".into()),
            Err(_) => Err("Error accessing the service".into()),
        }
    }

    /// Creates a new workflow run.
    pub async fn run_workflow(
        &self,
        request: RunRequest,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        self.run_workflow_with_attachments(request, Vec::new()).await
    }

    /// Creates a workflow run with optional uploaded workflow files.
    pub async fn run_workflow_with_attachments(
        &self,
        request: RunRequest,
        attachments: Vec<WorkflowAttachment>,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        self.check().map_err(|e| {
            error!("Service check failed: {}", e);
            e
        })?;

        let required = |value: String, name: &str| -> Result<String, String> {
            if value.is_empty() {
                Err(format!("{} is required", name))
            } else {
                Ok(value)
            }
        };
        let workflow_type = required(request.workflow_type, "workflow_type")?;
        let workflow_type_version =
            required(request.workflow_type_version, "workflow_type_version")?;
        let workflow_params = request
            .workflow_params
            .ok_or("workflow_params is required")?;
        let workflow_url = required(request.workflow_url, "workflow_url")?;

        let mut form = Form::new()
            .text("workflow_type", workflow_type)
            .text("workflow_type_version", workflow_type_version)
            .text("workflow_params", serde_json::to_string(&workflow_params)?)
            .text("workflow_url", workflow_url);
        if let Some(value) = request.tags {
            form = form.text("tags", serde_json::to_string(&value)?);
        }
        if let Some(value) = request.workflow_engine_parameters {
            form = form.text("workflow_engine_parameters", serde_json::to_string(&value)?);
        }
        if let Some(value) = request.workflow_engine {
            form = form.text("workflow_engine", value);
        }
        if let Some(value) = request.workflow_engine_version {
            form = form.text("workflow_engine_version", value);
        }
        for attachment in attachments {
            if attachment.filename.is_empty()
                || attachment.filename.split('/').any(|segment| segment == "..")
                || attachment.filename.split('\\').any(|segment| segment == "..") {
                return Err("workflow attachment filename must not be empty or contain '..' segments".into());
            }
            form = form.part("workflow_attachment", Part::bytes(attachment.content).file_name(attachment.filename));
        }

        let url = endpoint(&self.transport.config, "runs")?;
        let response = self.transport.post_multipart(&url, form).await;

        match response {
            Ok(response_body) => {
                let v: RunId = serde_json::from_str(&response_body)?;
                let run_id = v.run_id.filter(|id| !id.is_empty())
                    .ok_or("WES response is missing run_id")?;

                let run = Run {
                    id: run_id,
                    transport: self.transport.clone(),
                };
                Ok(run)
            }
            Err(e) => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to create run: {}", e),
            ))),
        }
    }

    /// List runs.
    pub async fn list_runs(
        &self,
        next_page_token: Option<String>,
        page_size: Option<i64>,
    ) -> Result<RunListResponse, Box<dyn std::error::Error>> {
        let path = paged_path("runs", next_page_token, page_size);
        let url = endpoint(&self.transport.config, &path)?;

        let response = self.transport.get(&url, None).await;

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
    use mockito::{mock, Matcher};
    use mockito::server_url;
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
            endpoint(&config, "runs").unwrap(),
            "https://example.org/a%20b/wes/v1/runs"
        );
    }

    #[tokio::test]
    async fn test_wes_uses_configured_api_prefix() {
        let _service_info = mock("GET", "/ga4gh/wes/v1/service-info")
            .with_status(200)
            .with_body(serde_json::json!({
                "id": "org.example.wes",
                "name": "Example WES",
                "type": { "group": "org.ga4gh", "artifact": "wes", "version": "1.1.0" },
                "organization": { "name": "Example", "url": "https://example.org" },
                "version": "1.0.0"
            }).to_string())
            .create();
        let _create = mock("POST", "/ga4gh/wes/v1/runs")
            .match_header("content-type", Matcher::Regex("^multipart/form-data; boundary=".to_string()))
            .with_status(200)
            .with_body(r#"{"run_id":"abc"}"#)
            .create();
        let _status = mock("GET", "/ga4gh/wes/v1/runs/abc/status")
            .with_status(200)
            .with_body(r#"{"run_id":"abc","state":"COMPLETE"}"#)
            .create();

        let base = Url::parse(&format!("{}/ga4gh/wes/v1", server_url())).unwrap();
        let wes = WES::new(&Configuration::new(base)).await.unwrap();
        let run = wes.run_workflow(valid_request()).await.unwrap();
        assert_eq!(run.status().await.unwrap(), State::Complete);
    }

    #[tokio::test]
    async fn test_wes_create() {
        let _m = mock("POST", "/runs")
            .match_header("content-type", Matcher::Regex("^multipart/form-data; boundary=".to_string()))
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
        let _m = mock("POST", "/runs").with_status(200).with_body("{}").create();
        let config = Configuration::new(Url::parse(&server_url()).unwrap());
        let wes = WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service { r#type: Box::new(ServiceType { artifact: "wes".to_string(), ..Default::default() }), ..Service::default() }),
        };
        assert!(wes.run_workflow(valid_request()).await.is_err());
    }

    #[tokio::test]
    async fn test_wes_uploads_workflow_attachment() {
        let _m = mock("POST", "/runs")
            .match_body(Matcher::Regex("name=\"workflow_attachment\"; filename=\"workflow.cwl\"[\\s\\S]*cwlVersion".to_string()))
            .with_status(200)
            .with_body(r#"{"run_id":"uploaded"}"#)
            .create();
        let config = Configuration::new(Url::parse(&server_url()).unwrap());
        let wes = WES {
            transport: Transport::new(&config),
            config,
            service: Ok(Service { r#type: Box::new(ServiceType { artifact: "wes".to_string(), ..Default::default() }), ..Service::default() }),
        };
        let request = RunRequest {
            workflow_url: "workflow.cwl".to_string(),
            ..valid_request()
        };
        let attachments = vec![WorkflowAttachment { filename: "workflow.cwl".to_string(), content: b"cwlVersion: v1.2".to_vec() }];
        assert_eq!(wes.run_workflow_with_attachments(request, attachments).await.unwrap().id, "uploaded");
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
        let run = Run::new("a/b ?#".to_string(), Transport::new(&Configuration::new(base)));
        assert_eq!(run.status().await.unwrap(), State::Complete);
        assert_eq!(run.log().await.unwrap().run_id.as_deref(), Some("a/b ?#"));
        assert_eq!(run.cancel().await.unwrap().run_id.as_deref(), Some("a/b ?#"));
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
            .match_query("page_token=a%2Bb%26c&page_size=5")
            .with_status(200)
            .with_body(r#"{"runs":[]}"#)
            .create();
        let config = Configuration::new(Url::parse(&format!("{}/ga4gh/wes/v1/", server_url())).unwrap());
        let wes = WES { transport: Transport::new(&config), config, service: Ok(Service::default()) };
        assert!(wes.list_runs(Some("a+b&c".to_string()), Some(5)).await.is_ok());
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
            .match_query("page_token=a%2Bb%26c&page_size=2")
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
    async fn test_run_task_methods_reject_dot_segments() {
        let run = Run::new(
            "123".to_string(),
            Transport::new(&Configuration::new(Url::parse(&server_url()).unwrap())),
        );
        for task_id in ["", ".", ".."] {
            let err = run.get_task(task_id).await.unwrap_err();
            assert_eq!(err.to_string(), format!("invalid task ID: {task_id:?}"));
        }
        for run_id in [".", ".."] {
            let run = Run::new(run_id.to_string(), run.transport.clone());
            let err = run.list_tasks(None, None).await.unwrap_err();
            assert_eq!(err.to_string(), format!("invalid run ID: {run_id:?}"));
            let err = run.get_task("t1").await.unwrap_err();
            assert_eq!(err.to_string(), format!("invalid run ID: {run_id:?}"));
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
