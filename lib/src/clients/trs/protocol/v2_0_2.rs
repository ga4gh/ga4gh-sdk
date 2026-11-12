use super::super::{
    models::{Tool, ToolVersion},
    wire::v2_0_2::models as wire,
};
use serde::Deserialize;
use serde_json::Value;

// Validate the selected wire contract, including nested versions, then project
// the original JSON into the existing public model. New properties stay in
// `extra`; no serialization round trip through the wire DTO discards them.
pub(super) fn tool(value: Value) -> serde_json::Result<Tool> {
    wire::Tool::deserialize(&value)?;
    serde_json::from_value(value)
}
pub(super) fn version(value: Value) -> serde_json::Result<ToolVersion> {
    wire::ToolVersion::deserialize(&value)?;
    serde_json::from_value(value)
}
