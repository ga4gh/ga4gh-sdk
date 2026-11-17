//! A task model that extends the TES specification's `TesTask`.

use ga4gh_sdk::clients::tes::models::TesTask;
use ga4gh_sdk::clients::tes::{Task, TesView, TES};
use serde::{Deserialize, Serialize};

/// A TES task with the service's additional `priority` field.
///
/// The SDK's [`TesTask`] is embedded with `#[serde(flatten)]`, so the JSON
/// stays a single flat object: the specification's fields come from `task`,
/// `priority` is added next to them, and any other unknown fields end up in
/// `task.extra`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PrioritizedTask {
    /// The fields that the TES specification defines.
    #[serde(flatten)]
    pub task: TesTask,
    /// Scheduling priority defined by the example service.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}

impl PrioritizedTask {
    /// Wraps a TES task without setting a typed priority.
    pub fn new(task: TesTask) -> Self {
        Self {
            task,
            priority: None,
        }
    }

    /// Sets the priority, replacing a `priority` in `task.extra`, so that
    /// the JSON holds the key only once.
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.task.extra.remove("priority");
        self.priority = Some(priority);
        self
    }

    /// Submits the task with the SDK's TES client, including `priority`.
    ///
    /// # Errors
    ///
    /// Fails as [`TES::create_as`] does.
    pub async fn submit(&self, tes: &TES) -> ga4gh_sdk::Result<Task> {
        tes.create_as(self).await
    }

    /// Fetches a task, including `priority`, with the SDK's TES client.
    ///
    /// # Errors
    ///
    /// Fails as [`TES::get_as`] does, for example if the response isn't a
    /// valid task.
    pub async fn fetch(tes: &TES, view: TesView, id: &str) -> ga4gh_sdk::Result<Self> {
        tes.get_as(view, id).await
    }
}
