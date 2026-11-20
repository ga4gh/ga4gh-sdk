//! `TesState`: the state of a TES task, a `str` enum defined in Python, and the parsing of
//! state and view names.

use crate::utils::convert::define_class;
use ga4gh_sdk::clients::tes::models::TesState;
use ga4gh_sdk::clients::tes::TesView;
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;
use serde_json::Value;

/// Parses a TES view name, in any case; raises `ValueError` for others.
pub fn parse_view(view: &str) -> PyResult<TesView> {
    view.parse::<TesView>()
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Python source of the `TesState` enum. Its members must match the variants of `TesState`.
const TES_STATE_SOURCE: &str = r#"
import enum


class TesState(str, enum.Enum):
    """The state of a TES task. Members are strings equal to the state's name.

    `str()`, f-strings and `json.dumps()` give the state's name; `TesState("RUNNING")`
    looks a state up by name.
    """

    __module__ = "ga4gh_sdk"

    UNKNOWN = "UNKNOWN"
    QUEUED = "QUEUED"
    INITIALIZING = "INITIALIZING"
    RUNNING = "RUNNING"
    PAUSED = "PAUSED"
    COMPLETE = "COMPLETE"
    EXECUTOR_ERROR = "EXECUTOR_ERROR"
    SYSTEM_ERROR = "SYSTEM_ERROR"
    CANCELED = "CANCELED"
    PREEMPTED = "PREEMPTED"
    CANCELING = "CANCELING"

    def __str__(self):
        """Gives the state's name."""
        return self.value

    def __format__(self, format_spec):
        """Formats the state's name, as `str.__format__` does.

        Args:
            format_spec: The format specification, such as `">10"`.

        Returns:
            The formatted name.
        """
        return format(self.value, format_spec)
"#;

static TES_STATE: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// The `TesState` Python enum (a `str` enum, created once per interpreter).
pub fn tes_state_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    TES_STATE
        .get_or_try_init(py, || define_class(py, TES_STATE_SOURCE, "TesState"))
        .map(|cls| cls.bind(py))
}

/// Converts a TES state to the matching `TesState` member.
pub fn state_to_python(py: Python<'_>, state: TesState) -> PyResult<Py<PyAny>> {
    Ok(tes_state_type(py)?
        .getattr(state.to_string().as_str())?
        .unbind())
}

/// Parses a TES state name, in any case.
pub(super) fn parse_state(state: &str) -> PyResult<TesState> {
    let name = state.to_uppercase();
    // `TesState` reads states that it doesn't know as `Unknown`; only exact
    // names are valid here.
    serde_json::from_value::<TesState>(Value::String(name.clone()))
        .ok()
        .filter(|parsed| parsed.to_string() == name)
        .ok_or_else(|| PyValueError::new_err(format!("Invalid TES state: {state}")))
}

/// Reads a state name from a TES response; names that TES 1.1 doesn't define read as
/// `Unknown`.
pub(super) fn read_state(state: &str) -> TesState {
    serde_json::from_value(Value::String(state.to_owned())).unwrap_or_default()
}
