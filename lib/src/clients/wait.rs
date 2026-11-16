//! Cancelable observation of remote jobs. Dropping observation never cancels work.
//!
//! [`Task::wait`] and [`Run::wait`] poll a TES task or a WES run until it
//! ends, its [`WaitOptions::operation`] runs out of time or is cancelled, or a
//! poll fails. They return a [`WaitResult`] in every case, never an error. A
//! [`RemoteHandle`] identifies the task or run, so that another process can
//! wait for it again.
//!
//! ```no_run
//! use ga4gh_sdk::clients::tes::Task;
//! use ga4gh_sdk::clients::wait::{WaitOptions, WaitOutcome};
//! use ga4gh_sdk::utils::{configuration::Configuration, transport::Transport};
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Configuration::new(url::Url::parse("https://example.org/ga4gh/tes/v1")?);
//! let task = Task::new("task-1".to_string(), Transport::new(&config));
//! let result = task.wait(&WaitOptions::new(Duration::from_secs(600))).await;
//! if result.outcome != WaitOutcome::Complete {
//!     eprintln!("{:?}: {:?}", result.outcome, result.error);
//! }
//! # Ok(())
//! # }
//! ```
use super::{tes::Task, wes::Run};
use crate::utils::{
    configuration::Configuration,
    operation::{Operation, OperationError, ReplaySafety, RetryPolicy},
    transport::{urlencode, RequestOptions, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{error::Error, future::Future, time::Duration};

/// The API that runs a remote job.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum ServiceKind {
    /// A TES task.
    Tes,
    /// A WES run.
    Wes,
}
/// Identifies a TES task or WES run, so that it can be saved and waited for
/// again, for example by another process.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct RemoteHandle {
    /// The API that runs the job.
    pub service: ServiceKind,
    /// The service's API root, without credentials, query or fragment and
    /// ending in `/`.
    pub api_root: String,
    /// The task or run ID.
    pub id: String,
}
fn api_root(config: &Configuration) -> String {
    let mut root = config.base_path.clone();
    let _ = root.set_username("");
    let _ = root.set_password(None);
    root.set_query(None);
    root.set_fragment(None);
    if !root.path().ends_with('/') {
        root.set_path(&format!("{}/", root.path()));
    }
    root.into()
}
impl RemoteHandle {
    /// Builds the TES task of the handle, with `config` for its requests.
    ///
    /// # Errors
    ///
    /// Fails if the handle isn't for TES, its API root differs from that of
    /// `config`, or its ID is empty.
    pub fn task(&self, config: &Configuration) -> Result<Task, Box<dyn Error>> {
        self.validate(config, ServiceKind::Tes)?;
        Ok(Task::new(self.id.clone(), Transport::new(config)))
    }
    /// Builds the WES run of the handle, with `config` for its requests.
    ///
    /// # Errors
    ///
    /// Fails if the handle isn't for WES, its API root differs from that of
    /// `config`, or its ID is empty.
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
/// How long and how often a wait polls.
#[derive(Clone, Debug)]
pub struct WaitOptions {
    /// The deadline and cancellation of the whole wait, its polls included.
    pub operation: Operation,
    /// The delay after the first poll (default 1 s); it doubles after each
    /// poll. Must be positive.
    pub poll_interval: Duration,
    /// The longest delay between polls (default 10 s); at least
    /// `poll_interval`.
    pub max_interval: Duration,
    /// How often each poll request is tried (default: once).
    pub retry: RetryPolicy,
    /// Whether to fetch the full task or run log when the job ends in
    /// [`WaitOutcome::ExecutorError`], [`WaitOutcome::SystemError`] or
    /// [`WaitOutcome::Preempted`] (default `false`); see
    /// [`WaitResult::details`].
    pub fetch_failure_logs: bool,
}
impl WaitOptions {
    /// Options for a wait that ends `budget` from now, with the defaults of
    /// the fields.
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
/// How a wait ended: in a final state of the job, or before it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum WaitOutcome {
    /// The job ended in `COMPLETE`.
    Complete,
    /// The job ended in `EXECUTOR_ERROR`.
    ExecutorError,
    /// The job ended in `SYSTEM_ERROR`.
    SystemError,
    /// The job ended in `PREEMPTED`.
    Preempted,
    /// The job ended in `CANCELED`.
    Canceled,
    /// The wait's deadline passed; the job may still be running.
    Deadline,
    /// The wait's operation was cancelled; the job may still be running.
    Interrupted,
    /// A poll failed, its response had no state, or the options were
    /// invalid; [`WaitResult::error`] says why. The job may still be running.
    PollingFailed,
}
/// What a wait observed. Only the outcomes of final states mean that the job
/// ended; unknown states never count as success.
#[derive(Debug)]
pub struct WaitResult {
    /// The task or run that was waited for.
    pub handle: RemoteHandle,
    /// How the wait ended.
    pub outcome: WaitOutcome,
    /// The last state that the service reported.
    pub last_state: Option<String>,
    /// The last poll response with a state: the minimal view of the task
    /// (TES) or the run status (WES).
    pub last_response: Option<Value>,
    /// With [`WaitOptions::fetch_failure_logs`], the full task (TES) or the
    /// run log (WES) of a job that failed or was preempted.
    pub details: Option<Value>,
    /// Why the wait ended before a final state, or why the details couldn't
    /// be fetched.
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
///
/// Calls `poll` until its response has a final `state` of `handle`'s API,
/// with delays from `options` between calls. States that aren't final, known
/// or not, keep the wait going. A failed poll, a response without a string
/// `state`, the deadline or the cancellation of `options.operation` end it.
/// [`Task::wait`] and [`Run::wait`] use it; call it to wait with a poll of
/// your own.
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
    /// The task's handle, for waiting for it again later.
    pub fn remote_handle(&self) -> RemoteHandle {
        RemoteHandle {
            service: ServiceKind::Tes,
            api_root: api_root(&self.transport.config),
            id: self.id.clone(),
        }
    }
    /// Polls the task's minimal view until it ends or the wait stops; see
    /// [`WaitOutcome`]. Dropping the future stops polling but never cancels
    /// the task.
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
    /// Cancels the task, then waits for it like [`Self::wait`], within the
    /// same `options.operation`.
    ///
    /// # Errors
    ///
    /// Fails if the cancel request fails, or the operation's deadline passes
    /// or it is cancelled before the request is done.
    pub async fn cancel_and_wait(
        &self,
        options: &WaitOptions,
    ) -> Result<WaitResult, Box<dyn Error>> {
        options.operation.run(self.cancel()).await??;
        Ok(self.wait(options).await)
    }
}
impl Run {
    /// The run's handle, for waiting for it again later.
    pub fn remote_handle(&self) -> RemoteHandle {
        RemoteHandle {
            service: ServiceKind::Wes,
            api_root: api_root(&self.transport.config),
            id: self.id.clone(),
        }
    }
    /// Polls the run's status until it ends or the wait stops; see
    /// [`WaitOutcome`]. Dropping the future stops polling but never cancels
    /// the run.
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
    /// Cancels the run, then waits for it like [`Self::wait`], within the
    /// same `options.operation`.
    ///
    /// # Errors
    ///
    /// Fails if the cancel request fails, or the operation's deadline passes
    /// or it is cancelled before the request is done.
    pub async fn cancel_and_wait(
        &self,
        options: &WaitOptions,
    ) -> Result<WaitResult, Box<dyn Error>> {
        options.operation.run(self.cancel()).await??;
        Ok(self.wait(options).await)
    }
}
