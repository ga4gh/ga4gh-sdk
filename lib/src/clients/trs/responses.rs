//! Response-preserving TRS helpers. Pass `&TRS` to retain its selected profile.
//! Passing `&Transport` uses the unverified 2.0.1 baseline without tag filtering.
//! `next_page` is a server-provided URL, never a guessed numeric offset.
use super::models::{DescriptorType, Tool};
use super::{TrsVersion, TRS};
use crate::compatibility::CapabilitySupport;
use crate::utils::transport::{response_json, response_text, urlencode, BodyLimits, Transport};
use reqwest::{header::HeaderMap, Method, Response};
use serde::de::DeserializeOwned;
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

/// A typed pagination context. Construct it from a client to preserve its policy,
/// or from a transport for the legacy, unverified 2.0.1 read contract.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    transport: &'a Transport,
    profile: TrsVersion,
    tags: CapabilitySupport,
}

impl<'a> From<&'a TRS> for Context<'a> {
    fn from(client: &'a TRS) -> Self {
        Self {
            transport: client.transport(),
            profile: client.compatibility().selection().profile(),
            tags: client.compatibility().tag_filter(),
        }
    }
}

impl<'a> From<&'a Transport> for Context<'a> {
    fn from(transport: &'a Transport) -> Self {
        Self {
            transport,
            profile: TrsVersion::V2_0_1,
            tags: CapabilitySupport::Unknown,
        }
    }
}

pub(super) async fn json<T: DeserializeOwned>(
    transport: &Transport,
    endpoint: &str,
    params: Option<Value>,
    limits: BodyLimits,
) -> Result<(HeaderMap, T), Box<dyn Error>> {
    let mut request = transport
        .request_builder(Method::GET, endpoint)?
        .header(reqwest::header::ACCEPT, "application/json");
    if let Some(params) = params {
        let pairs = crate::utils::query::query_pairs(&params).map_err(std::io::Error::from)?;
        if !pairs.is_empty() {
            request = request.query(&pairs);
        }
    }
    let response = request.send().await?;
    let headers = response.headers().clone();
    Ok((headers, response_json(response, limits).await?))
}

async fn page(
    context: Context<'_>,
    endpoint: &str,
    params: Option<Value>,
    limits: BodyLimits,
) -> Result<ToolPage, Box<dyn Error>> {
    if let Some(params) = &params {
        // Check what will be sent: null and empty query arrays are omitted by
        // the shared encoder, so they do not request filtering.
        let pairs = crate::utils::query::query_pairs(params).map_err(std::io::Error::from)?;
        if pairs.iter().any(|(key, _)| key == "tags") {
            context.tags.require("TRS", "tags")?;
        }
    }
    let (headers, body) = json(context.transport, endpoint, params, limits).await?;
    Ok(ToolPage {
        tools: context.profile.decode_tools(body)?,
        headers,
    })
}

/// Fetch one page using the client's selected decoder and capability policy.
/// Use [`super::ToolFilter::query`] to validate typed filters first.
pub async fn list_tools_page<'a>(
    context: impl Into<Context<'a>>,
    params: Option<Value>,
    limits: BodyLimits,
) -> Result<ToolPage, Box<dyn Error>> {
    page(context.into(), "tools", params, limits).await
}

/// Follow only a tools URL at the same API root. Credentials are never sent to
/// a foreign pagination URL. The original query (including opaque tokens) stays.
pub async fn next_tools_page<'a>(
    context: impl Into<Context<'a>>,
    next_page: &str,
    limits: BodyLimits,
) -> Result<ToolPage, Box<dyn Error>> {
    let context = context.into();
    let mut tools = context.transport.endpoint("tools")?;
    // Resolve links against a credential-free base. Relative links must not
    // inherit URL userinfo and then look like credential-bearing foreign links.
    let _ = tools.set_username("");
    let _ = tools.set_password(None);
    let next = tools.join(next_page)?;
    if next.origin() != tools.origin()
        || next.path() != tools.path()
        || !next.username().is_empty()
        || next.password().is_some()
        || next.fragment().is_some()
    {
        return Err("TRS next_page must identify tools at the same API root".into());
    }
    // An empty echoed `tags=` does not request tag filtering. Preserve the
    // server's opaque link while guarding a nonempty filter before the request.
    if next
        .query_pairs()
        .any(|(key, value)| key == "tags" && !value.is_empty())
    {
        context.tags.require("TRS", "tags")?;
    }
    let endpoint = match next.query() {
        Some(query) => format!("tools?{query}"),
        None => "tools".into(),
    };
    page(context, &endpoint, None, limits).await
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
    // `Unrecognized` descriptor types hold any string; send them as one segment.
    let path = format!(
        "tools/{}/versions/{}/{}/files",
        urlencode(tool),
        urlencode(version),
        super::segment("descriptor type", &descriptor.to_string())?
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
