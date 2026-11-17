//! The TES calls, shared by the sync classes (`TES`, `Task`) and the async ones (`AsyncTES`,
//! `AsyncTask`). The plain functions need the GIL and only prepare a request. The `async`
//! ones send it and convert the result without the GIL; they hold only owned, `Send` data
//! and give errors as text (the SDK's boxed errors are not `Send`), which the callers raise
//! as `RuntimeError`.
//!
//! The calls that list tasks are in `list`, those that wait for a task in `wait`.

use super::helpers::invalid_response_text;
use super::tes_task::PyTesTask;
use ga4gh_sdk::clients::tes::models::TesState;
use ga4gh_sdk::clients::tes::Task;
use ga4gh_sdk::clients::tes::TesView;
use ga4gh_sdk::clients::tes::TES;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::urlencode;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyString;
use serde_json::Value;
use std::path::PathBuf;

/// Connects to a TES service: reads its service info and checks that it is a TES.
pub async fn connect(config: &Configuration) -> Result<TES, String> {
    TES::new(config)
        .await
        .map_err(|error| format!("Failed to create TES instance: {error}"))
}

/// The JSON of a task to create, from the argument of `create()`.
///
/// `task` is a `TesTask`, a `dict` (or other JSON-serialisable object), or the path of a
/// JSON file as `str` or `os.PathLike`. A `str` is always a path. `method` names the
/// caller in errors, for example `"TES.create()"`.
pub fn task_to_create(task: &Bound<'_, PyAny>, method: &str) -> PyResult<Value> {
    if let Ok(text) = task.cast::<PyString>() {
        // Lossy, so that paths with surrogate escapes (non-UTF-8 file names) pass.
        if text.to_string_lossy().trim_start().starts_with('{') {
            return Err(PyValueError::new_err(format!(
                "{method} takes a str as a file path; pass JSON text as \
                 TesTask.from_json(text), or a path that starts with '{{' as a \
                 pathlib.Path"
            )));
        }
    }
    let task = if task.is_instance_of::<PyString>() || task.hasattr("__fspath__")? {
        PyTesTask::from_path(task.py(), &task.extract::<PathBuf>()?)?
    } else {
        PyTesTask::from_python(task)?
    };
    Ok(task.raw().clone())
}

/// Creates a task from its JSON, sent as given.
pub async fn create_task(tes: &TES, task: Value) -> Result<Task, String> {
    tes.create_value(task)
        .await
        .map_err(|error| format!("Failed to create task: {error}"))
}

/// Gets a task in a view parsed by [`parse_view`](super::parse_view).
pub async fn get_task(tes: &TES, view: TesView, id: &str) -> Result<PyTesTask, String> {
    let response = tes
        .get_raw(view, id)
        .await
        .map_err(|error| format!("Failed to get task: {error}"))?;
    let raw = serde_json::from_str(&response).map_err(invalid_response_text)?;
    PyTesTask::parse(raw).map_err(invalid_response_text)
}

/// The task's current state.
pub async fn task_status(task: &Task) -> Result<TesState, String> {
    task.status()
        .await
        .map_err(|e| format!("Failed to get task status: {e}"))
}

/// Asks the server to cancel the task.
pub async fn cancel_task(task: &Task) -> Result<(), String> {
    task.cancel()
        .await
        .map(drop)
        .map_err(|e| format!("Failed to cancel task: {e}"))
}

/// The path of a task's lookup.
pub fn task_path(task: &Task) -> String {
    format!("tasks/{}", urlencode(&task.id))
}
