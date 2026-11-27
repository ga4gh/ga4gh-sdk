//! `WesState`: the state of a WES run, a `str` enum defined in Python.

use crate::utils::convert::define_class;
use ga4gh_sdk::clients::wes::models::State;
use pyo3::prelude::*;
use pyo3::sync::PyOnceLock;
use pyo3::types::PyType;

/// Python source of the `WesState` enum. Its members must match the variants of `State`.
const WES_STATE_SOURCE: &str = r#"
import enum


class WesState(str, enum.Enum):
    """The state of a WES run. Members are strings equal to the state's name.

    `str()`, f-strings and `json.dumps()` give the state's name; `WesState("RUNNING")`
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
    CANCELING = "CANCELING"
    PREEMPTED = "PREEMPTED"

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

static WES_STATE: PyOnceLock<Py<PyType>> = PyOnceLock::new();

/// The `WesState` Python enum (a `str` enum, created once per interpreter).
pub fn wes_state_type(py: Python<'_>) -> PyResult<&Bound<'_, PyType>> {
    WES_STATE
        .get_or_try_init(py, || define_class(py, WES_STATE_SOURCE, "WesState"))
        .map(|cls| cls.bind(py))
}

/// Converts a WES state to the matching `WesState` member.
pub fn state_to_python(py: Python<'_>, state: State) -> PyResult<Py<PyAny>> {
    Ok(wes_state_type(py)?
        .getattr(state.to_string().as_str())?
        .unbind())
}
