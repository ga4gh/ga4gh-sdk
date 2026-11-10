//! Response-preserving TRS helpers, composable with any configured Transport.
//! `next_page` is a server-provided URL, never a guessed numeric offset.
use super::models::{DescriptorType, Tool};
use crate::utils::transport::{response_text, urlencode, BodyLimits, Transport};
use reqwest::{header::HeaderMap, Method, Response};
use serde_json::Value;
use std::error::Error;

#[derive(Debug)]
pub struct ToolPage {
    pub tools: Vec<Tool>,
    pub headers: HeaderMap,
}

impl ToolPage {
    pub fn next_page(&self) -> Result<Option<&str>, reqwest::header::ToStrError> {
        self.headers
            .get("next_page")
            .map(|h| h.to_str())
            .transpose()
            .map(|value| value.filter(|s| !s.is_empty()))
    }
}

async fn page(
    transport: &Transport,
    endpoint: &str,
    params: Option<Value>,
    limits: BodyLimits,
) -> Result<ToolPage, Box<dyn Error>> {
    let response = transport
        .request_response(Method::GET, endpoint, None, params)
        .await?;
    let headers = response.headers().clone();
    let body = response_text(response, limits).await?;
    Ok(ToolPage {
        tools: serde_json::from_str(&body)?,
        headers,
    })
}

pub async fn list_tools_page(
    transport: &Transport,
    params: Option<Value>,
    limits: BodyLimits,
) -> Result<ToolPage, Box<dyn Error>> {
    page(transport, "tools", params, limits).await
}

/// Follow only a tools URL at the same API root. Credentials are never sent to
/// a foreign pagination URL. The original query (including opaque tokens) stays.
pub async fn next_tools_page(
    transport: &Transport,
    next_page: &str,
    limits: BodyLimits,
) -> Result<ToolPage, Box<dyn Error>> {
    let tools = transport.endpoint("tools")?;
    let next = tools.join(next_page)?;
    if next.origin() != tools.origin()
        || next.path() != tools.path()
        || !next.username().is_empty()
        || next.password().is_some()
        || next.fragment().is_some()
    {
        return Err("TRS next_page must identify tools at the same API root".into());
    }
    let endpoint = match next.query() {
        Some(query) => format!("tools?{query}"),
        None => "tools".into(),
    };
    page(transport, &endpoint, None, limits).await
}

/// Opaque ZIP response; consume `chunk()`/`bytes_stream()` or `read_bounded`.
/// No extraction, eager buffering, or retries. Drop the response to stop reading.
pub async fn files_zip(
    transport: &Transport,
    tool: &str,
    version: &str,
    descriptor: DescriptorType,
) -> Result<Response, Box<dyn Error>> {
    if [tool, version]
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return Err("TRS tool/version IDs must be nonempty opaque segments".into());
    }
    let path = format!(
        "tools/{}/versions/{}/{descriptor}/files",
        urlencode(tool),
        urlencode(version)
    );
    let response = transport
        .request_response(
            Method::GET,
            &path,
            None,
            Some(serde_json::json!({"format":"zip"})),
        )
        .await?;
    if !response.status().is_success() {
        response_text(response, BodyLimits::default()).await?;
        return Err("TRS ZIP request failed".into());
    }
    Ok(response)
}
