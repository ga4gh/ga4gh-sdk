//! A client for an endpoint that the TES specification doesn't define.

use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::{urlencode, Transport};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::error::Error;

/// Resource usage of a task, as returned by `GET /tasks/{id}/usage`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskUsage {
    /// The task's ID.
    pub id: String,
    /// CPU time used by all executors, in seconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_seconds: Option<f64>,
    /// Peak memory of any executor, in bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_bytes: Option<u64>,
    /// Fields that this model doesn't define, kept as received.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// Client for the usage endpoint, beneath the TES API root.
///
/// It sends its requests through the SDK's [`Transport`], so it shares the
/// base path, user agent and HTTP client (timeouts, TLS, connection pool)
/// with the SDK's own clients.
#[derive(Clone, Debug)]
pub struct UsageClient {
    transport: Transport,
}

impl UsageClient {
    /// Creates a client with its own transport for `config`.
    pub fn new(config: &Configuration) -> Self {
        Self::from_transport(Transport::new(config))
    }

    /// Creates a client that shares an existing transport, such as
    /// `ga4gh_sdk::clients::tes::TES::transport`.
    pub fn from_transport(transport: Transport) -> Self {
        Self { transport }
    }

    /// The transport that this client sends its requests with.
    pub fn transport(&self) -> &Transport {
        &self.transport
    }

    /// Fetches the resource usage of the task with ID `id`.
    ///
    /// # Errors
    ///
    /// Fails if the request fails, if the service responds with an error
    /// status (as [`ga4gh_sdk::utils::transport::HttpError`]), or if the
    /// response isn't a valid [`TaskUsage`].
    pub async fn task_usage(&self, id: &str) -> Result<TaskUsage, Box<dyn Error>> {
        let path = format!("tasks/{}/usage", urlencode(id));
        let body = self.transport.get(&path, None).await?;
        Ok(serde_json::from_str(&body)?)
    }
}
