//! Downstream proTES consumer. No proprietary endpoints are added to standard TES.
use ga4gh_sdk::{
    clients::tes::models::TesTask,
    utils::transport::{urlencode, Transport},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, error::Error};
pub type CompanionError = Box<dyn Error>;
pub const API_REVISION: &str = "55c4d1591cd3be883ad0b4fd2da01d49976414cb";
#[derive(Debug, Serialize, Deserialize)]
pub struct Pipeline {
    pub source: Option<String>,
    pub preset: Option<String>,
    pub resolved_hash: Option<String>,
    pub resolved: Option<Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Hook {
    pub step: Option<String>,
    pub phase: Option<String>,
    pub plugin: Option<Value>,
    pub outcome: Option<String>,
    pub changes: Option<Changes>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Changes {
    #[serde(default)]
    pub paths: Vec<String>,
    /// Gateway HMAC evidence, not original mutation values or an SDK attestation.
    pub before: Option<String>,
    pub after: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Attempt {
    pub number: Option<u64>,
    pub backend: Option<String>,
    pub outcome: Option<String>,
    pub reason: Option<String>,
    pub remote_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Provenance {
    pub pipeline: Option<Pipeline>,
    pub plugins: Option<Value>,
    pub builds: Option<Value>,
    pub executions: Option<Vec<Hook>>,
    pub route: Option<Value>,
    pub attempts: Option<Vec<Attempt>>,
    pub final_reason: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Explanation {
    pub api_revision: String,
    pub record: Provenance,
    pub original_record: Value,
    pub replay_reconstruction: String,
    pub caveats: Vec<String>,
}
pub fn explain(raw: Value, has_original_intent: bool) -> Result<Explanation, CompanionError> {
    Ok(Explanation {
        api_revision: API_REVISION.into(),
        record: serde_json::from_value(raw.clone())?,
        original_record: raw,
        replay_reconstruction: if has_original_intent {
            "supplied_original_request_required_for_plan"
        } else {
            "unavailable_without_original_client_intent"
        }
        .into(),
        caveats: vec![
            "Changed paths do not provide old/new mutation values".into(),
            "Gateway provenance is operational evidence, not cryptographic attestation".into(),
            "outcome_unknown must be reconciled; inspection never submits a task".into(),
        ],
    })
}
pub async fn fetch(transport: &Transport, id: &str) -> Result<Explanation, CompanionError> {
    let raw = transport
        .get(&format!("tasks/{}/provenance", urlencode(id)), None)
        .await?;
    explain(serde_json::from_str(&raw)?, false)
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub struct Choices {
    pub pipeline: Option<String>,
    pub backends: Option<BackendSelection>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendMode {
    Order,
    Allow,
    Forbid,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct BackendSelection {
    #[serde(rename = "type")]
    pub mode: BackendMode,
    pub backends: Vec<String>,
}
#[derive(Debug, Serialize)]
pub struct RerunPlan {
    format_version: u32,
    source: String,
    request: Value,
    removed_fields: Vec<String>,
    removed_gateway_tags: Vec<String>,
    submission: String,
}
/// Original intent is a caller assertion, never a fetched FULL task fallback.
pub fn prepare_rerun(original: &Value, choices: Choices) -> Result<RerunPlan, CompanionError> {
    let object = original
        .as_object()
        .ok_or("original request must be an object")?;
    let writable = [
        "name",
        "description",
        "inputs",
        "outputs",
        "resources",
        "executors",
        "volumes",
        "tags",
    ];
    let mut request = serde_json::Map::new();
    let mut removed_fields = vec![];
    for (key, value) in object {
        if writable.contains(&key.as_str()) {
            request.insert(key.clone(), value.clone());
        } else {
            removed_fields.push(key.clone());
        }
    }
    let mut tags = match request.remove("tags") {
        Some(Value::Object(tags)) => tags,
        Some(Value::Null) | None => serde_json::Map::new(),
        _ => return Err("tags must be an object".into()),
    };
    let removed_gateway_tags = tags
        .keys()
        .filter(|key| key.starts_with("gateway."))
        .cloned()
        .collect::<Vec<_>>();
    for key in &removed_gateway_tags {
        tags.remove(key);
    }
    if let Some(pipeline) = choices.pipeline {
        if pipeline.trim().is_empty() {
            return Err("explicit pipeline must be nonempty".into());
        }
        tags.insert("gateway.pipeline".into(), json!(pipeline));
    }
    if let Some(backends) = choices.backends {
        if backends.backends.is_empty() {
            return Err("explicit backend selection must be nonempty".into());
        }
        for backend in &backends.backends {
            let url = url::Url::parse(backend)?;
            if !["https", "http"].contains(&url.scheme())
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("backend choice must be a credential-free HTTP(S) URL".into());
            }
        }
        tags.insert(
            "gateway.backends".into(),
            json!(serde_json::to_string(&backends)?),
        );
    }
    if !tags.is_empty() {
        request.insert("tags".into(), Value::Object(tags));
    }
    let request = Value::Object(request);
    let _: TesTask = serde_json::from_value(request.clone())?;
    if !request
        .get("executors")
        .and_then(Value::as_array)
        .is_some_and(|v| !v.is_empty())
    {
        return Err("original request requires at least one executor".into());
    }
    Ok(RerunPlan {
        format_version: 1,
        source: "caller_supplied_original_client_request".into(),
        request,
        removed_fields,
        removed_gateway_tags,
        submission: "not_submitted; review this new request before explicit create".into(),
    })
}

/// Configure the shared transport with a caller's current OAuth bearer token.
/// Main SDK #31 does not yet apply Configuration authentication fields itself.
/// Library callers can always inject their own authenticated Transport into fetch.
pub fn configured_transport(
    config: &ga4gh_sdk::utils::configuration::Configuration,
) -> Result<Transport, CompanionError> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
    if config.basic_auth.is_some() || config.api_key.is_some() {
        return Err("CLI supports OAuth bearer authentication; inject another authenticated Transport through the library".into());
    }
    let mut headers = HeaderMap::new();
    if let Some(token) = config
        .oauth_access_token
        .as_ref()
        .or(config.bearer_access_token.as_ref())
    {
        let mut value = HeaderValue::from_str(&format!("Bearer {token}"))?;
        value.set_sensitive(true);
        headers.insert(AUTHORIZATION, value);
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers)
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()?;
    Ok(Transport {
        config: config.clone(),
        client,
    })
}
/// Compose an authenticated standard TES client without a second anonymous discovery.
pub async fn tes_client(
    config: &ga4gh_sdk::utils::configuration::Configuration,
) -> Result<ga4gh_sdk::clients::tes::TES, CompanionError> {
    let transport = configured_transport(config)?;
    let service = serde_json::from_str(&transport.get("service-info", None).await?)?;
    Ok(ga4gh_sdk::clients::tes::TES {
        config: config.clone(),
        transport,
        service: Ok(service),
    })
}

impl RerunPlan {
    pub fn request(&self) -> &Value {
        &self.request
    }
    pub fn removed_gateway_tags(&self) -> &[String] {
        &self.removed_gateway_tags
    }
}
/// Submit the exact immutable prepared JSON. Generated models may omit downstream
/// or newer TES fields, so no deserialize/reserialize step is allowed here.
pub async fn create_plan(
    client: &ga4gh_sdk::clients::tes::TES,
    plan: &RerunPlan,
) -> Result<ga4gh_sdk::clients::tes::Task, CompanionError> {
    if !client
        .service
        .as_ref()
        .is_ok_and(|s| s.r#type.artifact.eq_ignore_ascii_case("tes"))
    {
        return Err("expected an authenticated TES service".into());
    }
    let response=client.transport.post("tasks",Some(plan.request.clone())).await.map_err(|error| {
        if error.downcast_ref::<ga4gh_sdk::utils::transport::HttpStatusError>().is_some_and(|e|e.status.is_client_error()) {error}
        else {format!("submission outcome may be unknown; reconcile before another execution: {error}").into()}
    })?;
    let acknowledgement:Value=serde_json::from_str(&response).map_err(|_|"submission may have succeeded; malformed acknowledgement; reconcile before another execution")?;
    let id=acknowledgement.get("id").and_then(Value::as_str).filter(|s|!s.is_empty()).ok_or("submission may have succeeded without a usable task ID; reconcile before another execution")?;
    Ok(ga4gh_sdk::clients::tes::Task::new(
        id.into(),
        client.transport.clone(),
    ))
}
