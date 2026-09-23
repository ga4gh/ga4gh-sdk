pub mod models;

use crate::clients::serviceinfo::models::Service;
use crate::clients::wes::models::WesRunId;
use crate::clients::wes::models::WesRunListResponse;
use crate::clients::wes::models::WesRunLog;
use crate::clients::wes::models::WesRunRequest;
use crate::clients::wes::models::WesRunStatus;
use crate::clients::wes::models::WesState;
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
    pub async fn status(&self) -> Result<WesState, Box<dyn std::error::Error>> {
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
                let state: WesState = serde_json::from_value(json!(state_str))?;
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
    pub async fn cancel(&self) -> Result<WesRunId, Box<dyn std::error::Error>> {
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
                let run_id: WesRunId = from_str(&resp_str)?;
                Ok(run_id)
            }
            Err(e) => Err(format!("HTTP request failed: {}", e).into()),
        }
    }

    /// Fetches the details of the run (including logs and outputs).
    pub async fn log(&self) -> Result<WesRunLog, Box<dyn std::error::Error>> {
        let id = &self.id;
        let url = endpoint(&self.transport.config, &format!("runs/{}", urlencode(id)))?;
        let response = self.transport.get(&url, None).await;
        match response {
            Ok(resp_str) => {
                let log: WesRunLog = from_str(&resp_str)?;
                Ok(log)
            }
            Err(e) => Err(e),
        }
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
        request: WesRunRequest,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        self.run_workflow_with_attachments(request, Vec::new()).await
    }

    /// Creates a workflow run with optional uploaded workflow files.
    pub async fn run_workflow_with_attachments(
        &self,
        request: WesRunRequest,
        attachments: Vec<WorkflowAttachment>,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        self.check().map_err(|e| {
            error!("Service check failed: {}", e);
            e
        })?;

        let workflow_type = request.workflow_type.filter(|s| !s.is_empty())
            .ok_or("workflow_type is required")?;
        let workflow_type_version = request.workflow_type_version.filter(|s| !s.is_empty())
            .ok_or("workflow_type_version is required")?;
        let workflow_params = request.workflow_params.ok_or("workflow_params is required")?;
        let workflow_url = request.workflow_url.filter(|s| !s.is_empty())
            .ok_or("workflow_url is required")?;

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
                let v: WesRunId = serde_json::from_str(&response_body)?;
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
    ) -> Result<WesRunListResponse, Box<dyn std::error::Error>> {
        let mut query = url::form_urlencoded::Serializer::new(String::new());
        if let Some(token) = next_page_token {
            query.append_pair("page_token", &token);
        }
        if let Some(size) = page_size {
            query.append_pair("page_size", &size.to_string());
        }
        let query = query.finish();
        let path = if query.is_empty() {
            "runs".to_string()
        } else {
            format!("runs?{}", query)
        };
        let url = endpoint(&self.transport.config, &path)?;

        let response = self.transport.get(&url, None).await;

        match response {
            Ok(resp_str) => {
                // Starter Kit WES 0.2.0 returns a bare array, while WES 1.0.1
                // specifies an object with `runs` and `next_page_token`.
                #[derive(serde::Deserialize)]
                #[serde(untagged)]
                enum RunListBody {
                    Standard(WesRunListResponse),
                    StarterKit(Vec<WesRunStatus>),
                }
                let list = match from_str::<RunListBody>(&resp_str)? {
                    RunListBody::Standard(list) => list,
                    RunListBody::StarterKit(runs) => WesRunListResponse {
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
    pub async fn get_run(&self, id: &str) -> Result<WesRunLog, Box<dyn std::error::Error>> {
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

    fn valid_request() -> WesRunRequest {
        WesRunRequest {
            workflow_params: Some(json!({})),
            workflow_type: Some("CWL".to_string()),
            workflow_type_version: Some("v1.2".to_string()),
            workflow_url: Some("https://example.org/workflow.cwl".to_string()),
            ..Default::default()
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
        assert_eq!(run.status().await.unwrap(), WesState::Complete);
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
        let request = WesRunRequest {
            workflow_params: Some(json!({})),
            workflow_type: Some("CWL".to_string()),
            workflow_type_version: Some("v1.2".to_string()),
            workflow_url: Some("workflow.cwl".to_string()),
            ..Default::default()
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
        assert_eq!(result.unwrap(), WesState::Complete);
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
        assert_eq!(run.status().await.unwrap(), WesState::Complete);
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
        assert_eq!(runs[0].state, Some(WesState::Complete));
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
}
