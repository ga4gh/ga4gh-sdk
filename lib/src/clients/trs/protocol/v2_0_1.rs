use super::super::models::{Tool, ToolVersion};
use serde_json::Value;

pub(super) fn tool(value: Value) -> serde_json::Result<Tool> {
    serde_json::from_value(value)
}
pub(super) fn version(value: Value) -> serde_json::Result<ToolVersion> {
    serde_json::from_value(value)
}
