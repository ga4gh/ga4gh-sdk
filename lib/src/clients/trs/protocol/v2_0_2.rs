use super::super::models::{Tool, ToolVersion};
use serde::de::Error as _;
use serde_json::Value;

// The only changed model field in the pinned 2.0.2 schema is an optional
// ToolVersion.description string. Check it before decoding with the shared
// generated model, which preserves the original value in `extra`.
fn check_description(value: &Value, path: &str) -> serde_json::Result<()> {
    if matches!(value.get("description"), Some(value) if !value.is_null() && !value.is_string()) {
        return Err(serde_json::Error::custom(format!(
            "TRS 2.0.2 {path}.description must be a string or null"
        )));
    }
    Ok(())
}

pub(super) fn tool(value: Value) -> serde_json::Result<Tool> {
    if let Some(versions) = value.get("versions").and_then(Value::as_array) {
        for (index, version) in versions.iter().enumerate() {
            check_description(version, &format!("Tool.versions[{index}]"))?;
        }
    }
    serde_json::from_value(value)
}

pub(super) fn version(value: Value) -> serde_json::Result<ToolVersion> {
    check_description(&value, "ToolVersion")?;
    serde_json::from_value(value)
}
