pub mod models;

use crate::clients::serviceinfo::models::Service;
use crate::clients::wes::models::WesRunId;
use crate::clients::wes::models::WesRunListResponse;
use crate::clients::wes::models::WesRunLog;
use crate::clients::wes::models::WesRunRequest;
use crate::clients::wes::models::WesRunStatus;
use crate::clients::wes::models::WesState;
use crate::utils::configuration::Configuration;
/// Existing import path for the shared path-segment encoder.
pub use crate::utils::transport::urlencode;
use crate::utils::transport::{HttpStatusError, Transport};
use log::error;
use reqwest::multipart::{Form, Part};
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
    pub async fn status(&self) -> Result<WesState, Box<dyn std::error::Error>> {
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
                let run_id: WesRunId = from_str(&resp_str)?;
                Ok(run_id)
            }
            Err(e) => Err(format!("HTTP request failed: {}", e).into()),
        }
    }

    /// Fetches the details of the run (including logs and outputs).
    pub async fn log(&self) -> Result<WesRunLog, Box<dyn std::error::Error>> {
        let id = &self.id;
        let url = format!("runs/{}", urlencode(id));
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
        request: WesRunRequest,
    ) -> Result<Run, Box<dyn std::error::Error>> {
        self.run_workflow_with_attachments(request, Vec::new())
            .await
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

        let workflow_type = request
            .workflow_type
            .filter(|s| !s.is_empty())
            .ok_or("workflow_type is required")?;
        let workflow_type_version = request
            .workflow_type_version
            .filter(|s| !s.is_empty())
            .ok_or("workflow_type_version is required")?;
        let workflow_params = request
            .workflow_params
            .ok_or("workflow_params is required")?;
        let workflow_url = request
            .workflow_url
            .filter(|s| !s.is_empty())
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
                || attachment
                    .filename
                    .split('/')
                    .any(|segment| segment == "..")
                || attachment
                    .filename
                    .split('\\')
                    .any(|segment| segment == "..")
            {
                return Err(
                    "workflow attachment filename must not be empty or contain '..' segments"
                        .into(),
                );
            }
            form = form.part(
                "workflow_attachment",
                Part::bytes(attachment.content).file_name(attachment.filename),
            );
        }

        let response = self.transport.post_multipart("runs", form).await;

        match response {
            Ok(response_body) => {
                let run_id = crate::utils::acknowledgement::execution_id(
                    &response_body,
                    "run_id",
                    "WES creation",
                    None,
                )?;

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
        let mut params = serde_json::Map::new();
        if let Some(token) = next_page_token {
            params.insert("page_token".to_string(), json!(token));
        }
        if let Some(size) = page_size {
            params.insert("page_size".to_string(), json!(size));
        }
        let params = if params.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(params))
        };
        let response = self.transport.get("runs", params).await;

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
            assert_eq!(run.status().await.unwrap(), WesState::Complete);
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
        let request = WesRunRequest {
            workflow_params: Some(json!({})),
            workflow_type: Some("CWL".to_string()),
            workflow_type_version: Some("v1.2".to_string()),
            workflow_url: Some("workflow.cwl".to_string()),
            ..Default::default()
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
        let run = Run::new(
            "a/b ?#".to_string(),
            Transport::new(&Configuration::new(base)),
        );
        assert_eq!(run.status().await.unwrap(), WesState::Complete);
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
        assert_eq!(runs[0].state, Some(WesState::Complete));
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
}
