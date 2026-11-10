//! Cancelable observation of remote jobs. Dropping observation never cancels work.
use super::{tes::Task, wes::Run};
use crate::utils::{
    configuration::Configuration,
    operation::{Operation, OperationError, ReplaySafety, RetryPolicy},
    transport::{urlencode, RequestOptions, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{error::Error, future::Future, time::Duration};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum ServiceKind {
    Tes,
    Wes,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RemoteHandle {
    pub service: ServiceKind,
    pub api_root: String,
    pub id: String,
}
fn api_root(config: &Configuration) -> String {
    let mut root = config.base_path.clone();
    let _ = root.set_username("");
    let _ = root.set_password(None);
    root.set_query(None);
    root.set_fragment(None);
    let path = format!("{}/", root.path().trim_end_matches('/'));
    root.set_path(&path);
    root.into()
}
impl RemoteHandle {
    pub fn task(&self, config: &Configuration) -> Result<Task, Box<dyn Error>> {
        self.validate(config, ServiceKind::Tes)?;
        Ok(Task::new(self.id.clone(), Transport::new(config)))
    }
    pub fn run(&self, config: &Configuration) -> Result<Run, Box<dyn Error>> {
        self.validate(config, ServiceKind::Wes)?;
        Ok(Run::new(self.id.clone(), Transport::new(config)))
    }
    fn validate(&self, config: &Configuration, kind: ServiceKind) -> Result<(), Box<dyn Error>> {
        if self.service != kind || self.api_root != api_root(config) || self.id.is_empty() {
            return Err("remote handle does not match service/API root or has an empty ID".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug)]
pub struct WaitOptions {
    pub operation: Operation,
    pub poll_interval: Duration,
    pub max_interval: Duration,
    pub retry: RetryPolicy,
    pub fetch_failure_logs: bool,
}
impl WaitOptions {
    pub fn new(budget: Duration) -> Self {
        Self {
            operation: Operation::new(budget),
            poll_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(10),
            retry: RetryPolicy::default(),
            fetch_failure_logs: false,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum WaitOutcome {
    Complete,
    ExecutorError,
    SystemError,
    Preempted,
    Canceled,
    Deadline,
    Interrupted,
    PollingFailed,
}
#[derive(Debug)]
pub struct WaitResult {
    pub handle: RemoteHandle,
    pub outcome: WaitOutcome,
    pub last_state: Option<String>,
    pub last_response: Option<Value>,
    pub details: Option<Value>,
    pub error: Option<String>,
}
fn terminal(kind: ServiceKind, state: &str) -> Option<WaitOutcome> {
    match (kind, state) {
        (_, "COMPLETE") => Some(WaitOutcome::Complete),
        (_, "EXECUTOR_ERROR") => Some(WaitOutcome::ExecutorError),
        (_, "SYSTEM_ERROR") => Some(WaitOutcome::SystemError),
        (_, "CANCELED") => Some(WaitOutcome::Canceled),
        // Terminal unsuccessful in the pinned TES 1.1 and WES 1.1 contracts.
        (ServiceKind::Tes, "PREEMPTED") | (ServiceKind::Wes, "PREEMPTED") => {
            Some(WaitOutcome::Preempted)
        }
        _ => None,
    }
}
/// Shared state machine; unknown states stay observable and never become success.
pub async fn observe<F, Fut>(handle: RemoteHandle, options: &WaitOptions, mut poll: F) -> WaitResult
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Value, Box<dyn Error>>>,
{
    let mut result = WaitResult {
        handle,
        outcome: WaitOutcome::PollingFailed,
        last_state: None,
        last_response: None,
        details: None,
        error: None,
    };
    if options.poll_interval.is_zero() || options.max_interval < options.poll_interval {
        result.error = Some("wait intervals must be positive and ordered".into());
        return result;
    }
    let mut delay = options.poll_interval;
    loop {
        let error = match options.operation.run(poll()).await {
            Ok(Ok(response)) => {
                let state = response
                    .get("state")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if let Some(state) = state {
                    result.last_state = Some(state.clone());
                    result.last_response = Some(response);
                    if let Some(outcome) = terminal(result.handle.service, &state) {
                        result.outcome = outcome;
                        return result;
                    }
                    None
                } else {
                    Some("poll response has no string state".to_string())
                }
            }
            Ok(Err(error)) => {
                result.outcome = match error.downcast_ref::<OperationError>() {
                    Some(OperationError::Deadline) => WaitOutcome::Deadline,
                    Some(OperationError::Cancelled) => WaitOutcome::Interrupted,
                    None => WaitOutcome::PollingFailed,
                };
                Some(error.to_string())
            }
            Err(error) => {
                result.outcome = match error {
                    OperationError::Deadline => WaitOutcome::Deadline,
                    OperationError::Cancelled => WaitOutcome::Interrupted,
                };
                Some(error.to_string())
            }
        };
        if let Some(error) = error {
            result.error = Some(error);
            return result;
        }
        if let Err(error) = options.operation.sleep(delay).await {
            result.outcome = match error {
                OperationError::Deadline => WaitOutcome::Deadline,
                OperationError::Cancelled => WaitOutcome::Interrupted,
            };
            result.error = Some(error.to_string());
            return result;
        }
        delay = delay.saturating_mul(2).min(options.max_interval);
    }
}
async fn poll(
    transport: &Transport,
    path: &str,
    params: Option<Value>,
    options: &WaitOptions,
) -> Result<Value, Box<dyn Error>> {
    let request = RequestOptions {
        operation: options.operation.clone(),
        retry: options.retry.clone(),
        safety: ReplaySafety::ReadOnly,
        limits: Default::default(),
    };
    Ok(serde_json::from_str(
        &transport
            .request_with_policy(reqwest::Method::GET, path, None, params, &request)
            .await?,
    )?)
}
async fn details(
    transport: &Transport,
    path: &str,
    params: Option<Value>,
    options: &WaitOptions,
    result: &mut WaitResult,
) {
    if options.fetch_failure_logs
        && matches!(
            result.outcome,
            WaitOutcome::ExecutorError | WaitOutcome::SystemError | WaitOutcome::Preempted
        )
    {
        match options.operation.run(transport.get(path, params)).await {
            Ok(Ok(body)) => match serde_json::from_str(&body) {
                Ok(value) => result.details = Some(value),
                Err(e) => result.error = Some(format!("failure detail parse: {e}")),
            },
            Ok(Err(error)) => result.error = Some(format!("failure detail lookup: {error}")),
            Err(error) => result.error = Some(format!("failure detail lookup: {error}")),
        }
    }
}
impl Task {
    pub fn remote_handle(&self) -> RemoteHandle {
        RemoteHandle {
            service: ServiceKind::Tes,
            api_root: api_root(&self.transport.config),
            id: self.id.clone(),
        }
    }
    pub async fn wait(&self, options: &WaitOptions) -> WaitResult {
        let path = format!("tasks/{}", urlencode(&self.id));
        let mut result = observe(self.remote_handle(), options, || {
            poll(
                &self.transport,
                &path,
                Some(serde_json::json!({"view":"MINIMAL"})),
                options,
            )
        })
        .await;
        details(
            &self.transport,
            &path,
            Some(serde_json::json!({"view":"FULL"})),
            options,
            &mut result,
        )
        .await;
        result
    }
    pub async fn cancel_and_wait(
        &self,
        options: &WaitOptions,
    ) -> Result<WaitResult, Box<dyn Error>> {
        options.operation.run(self.cancel()).await??;
        Ok(self.wait(options).await)
    }
}
impl Run {
    pub fn remote_handle(&self) -> RemoteHandle {
        RemoteHandle {
            service: ServiceKind::Wes,
            api_root: api_root(&self.transport.config),
            id: self.id.clone(),
        }
    }
    pub async fn wait(&self, options: &WaitOptions) -> WaitResult {
        let path = format!("runs/{}/status", urlencode(&self.id));
        let mut result = observe(self.remote_handle(), options, || {
            poll(&self.transport, &path, None, options)
        })
        .await;
        details(
            &self.transport,
            &format!("runs/{}", urlencode(&self.id)),
            None,
            options,
            &mut result,
        )
        .await;
        result
    }
    pub async fn cancel_and_wait(
        &self,
        options: &WaitOptions,
    ) -> Result<WaitResult, Box<dyn Error>> {
        options.operation.run(self.cancel()).await??;
        Ok(self.wait(options).await)
    }
}
