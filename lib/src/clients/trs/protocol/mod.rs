//! Decoders selected before reading a response; errors never choose another decoder.
mod v2_0_1;
mod v2_0_2;

use super::{
    models::{Tool, ToolVersion},
    TrsVersion,
};
use serde_json::Value;

impl TrsVersion {
    pub(super) fn decode_tool(self, value: Value) -> serde_json::Result<Tool> {
        match self {
            Self::V2_0_1 => v2_0_1::tool(value),
            Self::V2_0_2 => v2_0_2::tool(value),
        }
    }
    pub(super) fn decode_version(self, value: Value) -> serde_json::Result<ToolVersion> {
        match self {
            Self::V2_0_1 => v2_0_1::version(value),
            Self::V2_0_2 => v2_0_2::version(value),
        }
    }
    pub(super) fn decode_tools(self, value: Value) -> serde_json::Result<Vec<Tool>> {
        let values: Vec<Value> = serde_json::from_value(value)?;
        values
            .into_iter()
            .map(|value| self.decode_tool(value))
            .collect()
    }
    pub(super) fn decode_versions(self, value: Value) -> serde_json::Result<Vec<ToolVersion>> {
        let values: Vec<Value> = serde_json::from_value(value)?;
        values
            .into_iter()
            .map(|value| self.decode_version(value))
            .collect()
    }
}
