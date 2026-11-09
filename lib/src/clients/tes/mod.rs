/// This module provides a client for interacting with the TES (Task Execution Service) API.
///
/// The `TES` struct represents a TES client and provides methods for creating tasks, retrieving task status, canceling tasks, and listing tasks.
///
/// # Examples
///
/// Creating a TES client:
///
/// ```rust
/// use ga4gh_sdk::clients::tes::TES;
/// use ga4gh_sdk::utils::configuration::Configuration;
///
/// # async fn test_tes_new() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Configuration::new(url::Url::parse("http://example.com")?);
/// let tes_result = TES::new(&config).await;
/// // Check if the client was created successfully
/// assert!(tes_result.is_ok());
/// # Ok(())
/// # }
/// ```
///
/// Creating a task:
///
/// ```rust
/// use ga4gh_sdk::clients::tes::TES;
/// use ga4gh_sdk::utils::configuration::Configuration;
/// use ga4gh_sdk::clients::tes::models::TesTask;
///
/// # async fn test_tes() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Configuration::new(url::Url::parse("http://example.com")?);
/// let tes = TES::new(&config).await?;
/// let task = TesTask::default();
/// let result = tes.create(task).await?;
/// assert_eq!(result.id, "123");
/// # Ok(())
/// # }
/// ```
///
/// Retrieving task status:
///
/// ```rust
/// use ga4gh_sdk::clients::tes::Task;
/// use ga4gh_sdk::utils::configuration::Configuration;
/// use ga4gh_sdk::clients::tes::models::TesState;
/// use ga4gh_sdk::utils::transport::Transport;
///
/// # async fn test_task_status() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Configuration::new(url::Url::parse("http://example.com")?);
/// let transport = Transport::new(&config);
/// let task = Task::new("123".to_string(), transport);
/// let result = task.status().await?;
/// assert_eq!(result, TesState::Complete);
/// # Ok(())
/// # }
/// ```
///
/// Canceling a task:
///
/// ```rust
/// use ga4gh_sdk::clients::tes::Task;
/// use ga4gh_sdk::utils::configuration::Configuration;
/// use ga4gh_sdk::utils::transport::Transport;
///
/// # async fn test_task_cancel() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Configuration::new(url::Url::parse("http://example.com")?);
/// let transport = Transport::new(&config);
/// let task = Task::new("123".to_string(), transport);
/// let result = task.cancel().await?;
/// assert_eq!(result["status"], "CANCELLED");
/// # Ok(())
/// # }
/// ```
///
/// Listing tasks:
///
/// ```rust
/// use ga4gh_sdk::clients::tes::TES;
/// use ga4gh_sdk::utils::configuration::Configuration;
/// use ga4gh_sdk::clients::tes::models::ListTasksParams;
///
/// # async fn test_tes_list_tasks() -> Result<(), Box<dyn std::error::Error>> {
/// let config = Configuration::new(url::Url::parse("http://example.com")?);
/// let tes = TES::new(&config).await?;
/// let result = tes.list_tasks(None).await?;
/// assert!(result.tasks.is_empty());
/// # Ok(())
/// # }
/// ```
pub mod models;
mod list_tasks_params;
use crate::utils::configuration::Configuration;
use crate::clients::serviceinfo::models::Service;
use crate::clients::serviceinfo::ServiceInfo;
use crate::clients::tes::models::TesListTasksResponse;
use crate::clients::tes::models::TesState;
use crate::clients::tes::models::TesTask;
use crate::utils::transport::Transport;
use crate::clients::tes::models::ListTasksParams;
use serde_json;
use serde_json::from_str;
use serde_json::json;
use serde::Serialize;
use serde_json::Value;
use log::{debug, error};

/// Serializes any serializable item into a JSON `Value`.
///
/// # Arguments
/// - `item`: The item to serialize.
///
/// # Returns
/// - A `serde_json::Value` containing the serialized JSON.
///
/// # Panics
/// - If serialization fails, the function will panic.
fn serialize_to_json<T: Serialize>(item: T) -> Value {
    serde_json::to_value(&item).unwrap()
}

/// URL-encodes a string.
///
/// # Arguments
/// - `s`: A reference to a string to encode.
///
/// # Returns
/// - A `String` containing the URL-encoded value.
pub fn urlencode<T: AsRef<str>>(s: T) -> String {
    ::url::form_urlencoded::byte_serialize(s.as_ref().as_bytes()).collect()
}

#[derive(Debug, Clone)]
pub struct Task {
    /// The unique ID of the task.
    pub id: String,
    /// The transport layer for sending HTTP requests.
    pub transport: Transport,
}

impl Task {
    /// Creates a new `Task` instance.
    ///
    /// # Arguments
    /// - `id`: The task ID.
    /// - `transport`: The `Transport` instance for HTTP communication.
    ///
    /// # Returns
    /// - A new `Task` instance.
    pub fn new(id: String, transport: Transport) -> Self {
        Task { id, transport }
    }

    /// Fetches the current status of the task.
    ///
    /// # Returns
    /// - On success, returns a `TesState` representing the task state.
    /// - On failure, returns an error.
    pub async fn status(&self) -> Result<TesState, Box<dyn std::error::Error>> {
        let task_id = &self.id;
        let view = "FULL";
        let url = format!("/tasks/{}?view={}", task_id, view);
        let response = self.transport.get(&url, None).await;
        match response {
            Ok(resp_str) => {
                let task: TesTask = from_str(&resp_str)?;
                Ok(task.state.unwrap())
            }
            Err(e) => {
                let err_msg = format!("HTTP request failed: {}", e);
                eprintln!("{}", err_msg);
                Err(Box::new(std::io::Error::new(std::io::ErrorKind::Other, err_msg)))
            }
        }
    }

    /// Cancels the task.
    ///
    /// # Returns
    /// - On success, returns a `serde_json::Value` containing the server's response.
    /// - On failure, returns an error.
    pub async fn cancel(&self) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let id = &self.id;
        let id = &urlencode(id);
        let url = format!("/tasks/{}:cancel", id);
        let response = self.transport.post(&url, None).await;
        match response {
	    Ok(resp_str) => {
	        let parsed_json = serde_json::from_str::<serde_json::Value>(&resp_str);
	        match parsed_json {
	            Ok(json) => Ok(json),
	            Err(e) => Err(format!("Failed to parse JSON: {}", e).into()),
	        }
	    }
	    Err(e) => Err(format!("HTTP request failed: {}", e).into()),
	}
    }
}

/// The main struct for interacting with a TES service.
#[derive(Debug)]
pub struct TES {
    #[allow(dead_code)]
    pub config: Configuration, // not used yet
    pub service: Result<Service, Box<dyn std::error::Error>>,
    pub transport: Transport,
}

impl TES {
    /// Creates a new `TES` instance.
    ///
    /// # Arguments
    /// - `config`: A reference to the service configuration.
    ///
    /// # Returns
    /// - A new `TES` instance, or an error if the initialization fails.
    pub async fn new(config: &Configuration) -> Result<Self, Box<dyn std::error::Error>> {
        let transport = Transport::new(config);
        let service_info = ServiceInfo::new(config)?;

        let resp = service_info.get().await;

        let instance = TES {
            config: config.clone(),
            transport,
            service: resp,
        };

        instance.check()?; // Propagate the error if check() fails
        Ok(instance)
    }

    /// Checks if the service is of TES class.
    ///
    /// # Returns
    /// - Ok(()) if the service is valid.
    /// - Err(String) if the service is invalid or an error occurs.
    fn check(&self) -> Result<(), String> {
        let resp = &self.service;
        match resp.as_ref() {
            Ok(service) if service.r#type.artifact == "tes" => Ok(()),
            Ok(_) => Err("The endpoint is not an instance of TES".into()),
            Err(_) => Err("Error accessing the service".into()),
        }
    }

    /// Creates a new TES task.
    ///
    /// # Arguments
    /// - `task`: The `TesTask` to create.
    ///
    /// # Returns
    /// - On success, returns a `Task` containing the created task details.
    /// - On failure, returns an error.
    pub async fn create(
        &self,
        task: TesTask, /*, params: models::TesTask*/
    ) -> Result<Task, Box<dyn std::error::Error>> {
        // First, check if the service is of TES class
        self.check().map_err(|e| {
            log::error!("Service check failed: {}", e);
            e
        })?;
        let response = self
            .transport
            .post("/ga4gh/tes/v1/tasks", Some(json!(task)))
            .await;
        match response {
            Ok(response_body) => {
                let v: serde_json::Value = serde_json::from_str(&response_body)?;

                // Access the `id` field
                let task_id = v
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .trim_matches('"')
                    .to_string();

                let task = Task {
                    id: task_id,
                    transport: self.transport.clone(),
                };
                Ok(task)
            }
            Err(e) => Err(Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("Failed to post task: {}", e),
            ))),
        }
    }

    /// Retrieves the details of a specific TES task.
    ///
    /// # Arguments
    /// - `view`: The level of detail to include in the response (e.g., `FULL`).
    /// - `id`: The task ID.
    ///
    /// # Returns
    /// - On success, returns a `TesTask` containing the task details.
    /// - On failure, returns an error.
    pub async fn get(&self, view: &str, id: &str) -> Result<TesTask, Box<dyn std::error::Error>> {
        let url = format!("/tasks/{}?view={}", id, view);
        let response = self.transport.get(&url, None).await;

        match response {
            Ok(resp_str) => {
                let task: TesTask = from_str(&resp_str)?;
                Ok(task)
            }
            Err(e) => Err(e),
        }
    }

    /// Lists TES tasks based on provided filtering parameters.
    ///
    /// # Arguments
    /// - `params`: Optional filtering parameters for listing tasks.
    ///
    /// # Returns
    /// - On success, returns a `TesListTasksResponse` containing the task list.
    /// - On failure, returns an error.
    pub async fn list_tasks(
        &self,
        params: Option<ListTasksParams>,
    ) -> Result<TesListTasksResponse, Box<dyn std::error::Error>> {
        let params_value = params.map(serialize_to_json);
        // Make the request with or without parameters based on the presence of params
        let response = if let Some(params_value) = params_value {
            self.transport.get("/tasks", Some(params_value)).await
        } else {
            self.transport.get("/tasks", None).await
        };
        
        match response {
            Ok(resp_str) => {
                let task: TesListTasksResponse = from_str(&resp_str)?;
                Ok(task)
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
}
#[cfg(test)]
mod tests {
    use crate::clients::serviceinfo::models::ServiceType;

    use super::*;
    use mockito::mock;
    use mockito::server_url;

    #[test]
    fn test_debug_output() {
        let config = crate::utils::configuration::config_with_markers();
        let transport = Transport::new(&config);
        let task = Task::new("123".to_string(), transport.clone());
        let tes = TES {
            config,
            service: Ok(Service::default()),
            transport,
        };
        for output in [format!("{:?}", task), format!("{:?}", tes)] {
            assert!(!output.contains("SENTINEL"), "{output}");
        }
    }

    #[tokio::test]
    async fn test_tes_create() {
        let _m = mock("POST", "/ga4gh/tes/v1/tasks")
            .with_status(200)
            .with_body(r#"{"id": "123"}"#)
            .create();

        let mock_url = url::Url::parse(&server_url()).expect("Invalid URL");
        let config= Configuration::new(mock_url);
        let transport = Transport::new(&config);
        
        let tes = TES {
            config,
            service: Ok(Service {
                r#type: Box::new(ServiceType {
                    artifact: "tes".to_string(),
                    ..Default::default()
                }),
                ..Service::default()
            }),
            transport,
        };

        let task = TesTask::default();
        let result = tes.create(task).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, String::from("123"));
    }

    #[tokio::test]
    async fn test_task_status() {
        let _m = mock("GET", "/tasks/123?view=FULL")
            .with_status(200)
            .with_body(r#"{"state": "COMPLETE", "executors": []}"#)
            .create();
        let mock_url = url::Url::parse(&server_url()).expect("Invalid URL");
        let transport = Transport::new(&Configuration::new(mock_url));
        let task = Task::new("123".to_string(), transport);

        let result = task.status().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), TesState::Complete);
    }

    #[tokio::test]
    async fn test_task_cancel() {
        let _m = mock("POST", "/tasks/123:cancel")
            .with_status(200)
            .with_body(r#"{"status": "CANCELLED", "executors": []}"#)
            .create();

        let mock_url = url::Url::parse(&server_url()).expect("Invalid URL");
        let transport = Transport::new(&Configuration::new(mock_url));

        let task = Task::new("123".to_string(), transport);

        let result = task.cancel().await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap()["status"], "CANCELLED");
    }

    #[tokio::test]
    async fn test_tes_get() {
        let _m = mock("GET", "/tasks/123?view=FULL")
            .with_status(200)
            .with_body(r#"{"id": "123", "state": "COMPLETE", "executors": []}"#)
            .create();

        let mock_url = url::Url::parse(&server_url()).expect("Invalid URL");
        let config= Configuration::new(mock_url);
        let transport = Transport::new(&config);
        let tes = TES {
            config,
            service: Ok(Service::default()),
            transport,
        };

        let result = tes.get("FULL", "123").await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().id, Some(String::from("123")));
    }

    #[tokio::test]
    async fn test_tes_list_tasks() {
        let _m = mock("GET", "/tasks")
            .with_status(200)
            .with_body(r#"{"tasks": []}"#)
            .create();

        let mock_url = url::Url::parse(&server_url()).expect("Invalid URL");
        let config= Configuration::new(mock_url);
        let transport = Transport::new(&config);
        let tes = TES {
            config,
            service: Ok(Service::default()),
            transport,
        };

        let result = tes.list_tasks(None).await;
        assert!(result.is_ok());
        assert!(result.unwrap().tasks.is_empty());
    }
}