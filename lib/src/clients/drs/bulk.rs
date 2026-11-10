//! DRS 1.4 bulk lookup, one explicit service/credential context per call.
//! Duplicate inputs collapse. The MVP accepts one access ID per object per call;
//! object-only grouped errors can then be attributed without fabricating an ID.
use super::{
    models::{BulkAccessUrl, DrsObject},
    resolution::deferred_delay,
    DRS,
};
use crate::utils::{
    operation::{retry_after, Operation, RetryPolicy},
    transport::{read_bounded, BodyLimits},
};
use futures_util::{stream, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct BulkKey {
    pub object_id: String,
    pub access_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BulkPending {
    pub api_root: String,
    pub endpoint: String,
    pub retry_after_secs: u64,
}
/// Multiple returned values are retained rather than silently selecting a duplicate.
#[derive(Debug)]
pub struct ObjectOutcome<T> {
    pub resolved: Vec<T>,
    pub failure_codes: Vec<u16>,
    pub pending: Option<BulkPending>,
    pub errors: Vec<String>,
}
impl<T> Default for ObjectOutcome<T> {
    fn default() -> Self {
        Self {
            resolved: Vec::new(),
            failure_codes: Vec::new(),
            pending: None,
            errors: Vec::new(),
        }
    }
}
impl<T> ObjectOutcome<T> {
    pub fn is_success(&self) -> bool {
        self.resolved.len() == 1
            && self.failure_codes.is_empty()
            && self.pending.is_none()
            && self.errors.is_empty()
    }
}
#[derive(Debug)]
pub struct BulkReport<T> {
    pub outcomes: BTreeMap<BulkKey, ObjectOutcome<T>>,
    pub diagnostics: Vec<String>,
}
impl<T> Default for BulkReport<T> {
    fn default() -> Self {
        Self {
            outcomes: BTreeMap::new(),
            diagnostics: Vec::new(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct BulkPolicy {
    pub chunk_size: usize,
    pub advertised_limit: Option<usize>,
    pub concurrency: usize,
    pub max_requests: usize,
    pub max_split_depth: usize,
    pub retry: RetryPolicy,
    pub limits: BodyLimits,
}
impl Default for BulkPolicy {
    fn default() -> Self {
        Self {
            chunk_size: 100,
            advertised_limit: None,
            concurrency: 4,
            max_requests: 1000,
            max_split_depth: 8,
            retry: Default::default(),
            limits: Default::default(),
        }
    }
}
impl BulkPolicy {
    pub fn with_service_info(mut self, raw: &Value) -> Result<Self, Box<dyn Error>> {
        if let Some(value) = raw.get("maxBulkRequestLength") {
            let limit = value
                .as_u64()
                .filter(|v| *v > 0)
                .ok_or("invalid maxBulkRequestLength")?;
            self.advertised_limit = Some(usize::try_from(limit)?);
        }
        Ok(self)
    }
    fn batch_size(&self) -> Result<usize, Box<dyn Error>> {
        self.retry.validate()?;
        if self.chunk_size == 0
            || self.advertised_limit == Some(0)
            || self.concurrency == 0
            || self.concurrency > 64
            || self.max_requests == 0
            || self.max_split_depth > 32
        {
            return Err(
                "bulk limits must be positive, concurrency <=64 and split depth <=32".into(),
            );
        }
        Ok(self
            .chunk_size
            .min(self.advertised_limit.unwrap_or(self.chunk_size)))
    }
}
fn request_body(keys: &[BulkKey], passports: &[String], access: bool) -> Value {
    let mut body = if access {
        json!({"bulk_object_access_ids":keys.iter().map(|k|json!({"bulk_object_id":k.object_id,"bulk_access_ids":[k.access_id.as_ref().unwrap()]})).collect::<Vec<_>>()})
    } else {
        json!({"bulk_object_ids":keys.iter().map(|k|&k.object_id).collect::<Vec<_>>()})
    };
    if !passports.is_empty() {
        body["passports"] = json!(passports);
    }
    body
}
fn correlate<T: DeserializeOwned>(keys: &[BulkKey], body: Value, access: bool) -> BulkReport<T> {
    let mut report = BulkReport::default();
    for key in keys {
        report
            .outcomes
            .insert(key.clone(), ObjectOutcome::default());
    }
    if !body.is_object() {
        for result in report.outcomes.values_mut() {
            result.errors.push("bulk response must be an object".into());
        }
        return report;
    }
    let field = if access {
        "resolved_drs_object_access_urls"
    } else {
        "resolved_drs_object"
    };
    if let Some(values) = body.get(field) {
        if let Some(values) = values.as_array() {
            for value in values {
                let object_id = value
                    .get(if access { "drs_object_id" } else { "id" })
                    .and_then(Value::as_str);
                let access_id = if access {
                    value.get("drs_access_id").and_then(Value::as_str)
                } else {
                    None
                };
                if object_id.is_none() || (access && access_id.is_none()) {
                    report
                        .diagnostics
                        .push("resolved entry is missing correlation identity".into());
                    continue;
                }
                let key = BulkKey {
                    object_id: object_id.unwrap().into(),
                    access_id: access_id.map(str::to_owned),
                };
                match report.outcomes.get_mut(&key) {
                    Some(outcome) => match serde_json::from_value::<T>(value.clone()) {
                        Ok(value) => {
                            if !outcome.resolved.is_empty() {
                                outcome.errors.push("duplicate resolved identity".into());
                                report
                                    .diagnostics
                                    .push("duplicate resolved identity".into());
                            }
                            outcome.resolved.push(value);
                        }
                        Err(error) => outcome
                            .errors
                            .push(format!("invalid resolved entry: {error}")),
                    },
                    None => report
                        .diagnostics
                        .push("response contained an unrequested resolved identity".into()),
                }
            }
        } else {
            report
                .diagnostics
                .push("resolved entries must be an array".into());
        }
    }
    if let Some(groups) = body.get("unresolved_drs_objects") {
        if let Some(groups) = groups.as_array() {
            for group in groups {
                let code = group
                    .get("error_code")
                    .and_then(Value::as_u64)
                    .filter(|c| (100..=599).contains(c))
                    .map(|c| c as u16);
                let ids = group.get("object_ids").and_then(Value::as_array);
                if let (Some(code), Some(ids)) = (code, ids) {
                    for id in ids {
                        let matches = report
                            .outcomes
                            .iter_mut()
                            .filter(|(key, _)| Some(key.object_id.as_str()) == id.as_str())
                            .collect::<Vec<_>>();
                        if matches.len() != 1 {
                            report.diagnostics.push(
                                "unresolved entry has missing/unrequested/ambiguous identity"
                                    .into(),
                            );
                            continue;
                        }
                        let (_, outcome) = matches.into_iter().next().unwrap();
                        if !outcome.failure_codes.is_empty() {
                            outcome.errors.push("duplicate unresolved identity".into());
                            report
                                .diagnostics
                                .push("duplicate unresolved identity".into());
                        }
                        outcome.failure_codes.push(code);
                        if !outcome.resolved.is_empty() {
                            outcome
                                .errors
                                .push("identity is both resolved and unresolved".into());
                        }
                    }
                } else {
                    report
                        .diagnostics
                        .push("invalid grouped unresolved entry".into());
                }
            }
        } else {
            report
                .diagnostics
                .push("unresolved entries must be an array".into());
        }
    }
    for outcome in report.outcomes.values_mut() {
        if outcome.resolved.is_empty()
            && outcome.failure_codes.is_empty()
            && outcome.errors.is_empty()
        {
            outcome
                .errors
                .push("response omitted requested identity".into());
        }
    }
    report
}
enum ChunkReply {
    Body(Value),
    Pending(u64),
    Failure(u16),
    Error(String),
}
impl DRS {
    async fn bulk_attempt(
        &self,
        keys: &[BulkKey],
        passports: &[String],
        access: bool,
        policy: &BulkPolicy,
        operation: &Operation,
        count: &AtomicUsize,
    ) -> ChunkReply {
        let path = if access { "objects/access" } else { "objects" };
        for attempt in 1..=policy.retry.max_attempts {
            if count
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    (n < policy.max_requests).then_some(n + 1)
                })
                .is_err()
            {
                return ChunkReply::Error("bulk request bound exhausted".into());
            }
            let response = match operation
                .run(self.transport.request_response(
                    reqwest::Method::POST,
                    path,
                    Some(request_body(keys, passports, access)),
                    None,
                ))
                .await
            {
                Ok(Ok(response)) => response,
                Ok(Err(error))
                    if attempt < policy.retry.max_attempts
                        && error.downcast_ref::<reqwest::Error>().is_some() =>
                {
                    if let Err(error) = operation.sleep(policy.retry.delay(attempt)).await {
                        return ChunkReply::Error(error.to_string());
                    }
                    continue;
                }
                Ok(Err(error)) => return ChunkReply::Error(error.to_string()),
                Err(error) => return ChunkReply::Error(error.to_string()),
            };
            if attempt < policy.retry.max_attempts
                && matches!(response.status().as_u16(), 429 | 503)
            {
                let delay = match response.headers().get(reqwest::header::RETRY_AFTER) {
                    Some(value) => match value
                        .to_str()
                        .ok()
                        .and_then(|v| retry_after(v, std::time::SystemTime::now()).ok())
                    {
                        Some(delay) => delay.max(policy.retry.delay(attempt)),
                        None => return ChunkReply::Error("invalid bulk Retry-After".into()),
                    },
                    None => policy.retry.delay(attempt),
                };
                drop(response);
                if let Err(error) = operation.sleep(delay).await {
                    return ChunkReply::Error(error.to_string());
                }
                continue;
            }
            match deferred_delay(&response) {
                Ok(Some(delay)) => return ChunkReply::Pending(delay),
                Err(error) => return ChunkReply::Error(error.to_string()),
                Ok(None) => (),
            }
            let status = response.status();
            if !status.is_success() {
                return ChunkReply::Failure(status.as_u16());
            }
            let body = match operation
                .run(read_bounded(response, policy.limits.success))
                .await
            {
                Ok(Ok(body)) => body.body,
                Ok(Err(error)) => return ChunkReply::Error(error.to_string()),
                Err(error) => return ChunkReply::Error(error.to_string()),
            };
            return match serde_json::from_slice(&body) {
                Ok(body) => ChunkReply::Body(body),
                Err(error) => ChunkReply::Error(format!("invalid bulk response JSON: {error}")),
            };
        }
        ChunkReply::Error("bulk retry attempts exhausted".into())
    }
    async fn bulk_chunk<T: DeserializeOwned>(
        &self,
        keys: Vec<BulkKey>,
        passports: &[String],
        access: bool,
        policy: &BulkPolicy,
        operation: &Operation,
        count: &AtomicUsize,
    ) -> BulkReport<T> {
        let mut report = BulkReport::default();
        let mut queue = VecDeque::from([(keys, 0)]);
        while let Some((keys, depth)) = queue.pop_front() {
            let reply = self
                .bulk_attempt(&keys, passports, access, policy, operation, count)
                .await;
            if matches!(reply, ChunkReply::Failure(413))
                && keys.len() > 1
                && depth < policy.max_split_depth
            {
                let middle = keys.len() / 2;
                queue.push_back((keys[..middle].to_vec(), depth + 1));
                queue.push_back((keys[middle..].to_vec(), depth + 1));
                continue;
            }
            match reply {
                ChunkReply::Body(body) => {
                    let chunk = correlate(&keys, body, access);
                    report.outcomes.extend(chunk.outcomes);
                    report.diagnostics.extend(chunk.diagnostics);
                }
                reply => {
                    for key in keys {
                        let mut outcome = ObjectOutcome::default();
                        match &reply {
                            ChunkReply::Failure(code) => outcome.failure_codes.push(*code),
                            ChunkReply::Error(error) => outcome.errors.push(error.clone()),
                            ChunkReply::Pending(delay) => {
                                outcome.pending = Some(BulkPending {
                                    api_root: self.resolution_reference(String::new()).api_root,
                                    endpoint: if access { "objects/access" } else { "objects" }
                                        .into(),
                                    retry_after_secs: *delay,
                                })
                            }
                            ChunkReply::Body(_) => unreachable!(),
                        }
                        report.outcomes.insert(key, outcome);
                    }
                }
            }
        }
        report
    }
    async fn bulk<T: DeserializeOwned>(
        &self,
        keys: Vec<BulkKey>,
        passports: &[String],
        access: bool,
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<T>, Box<dyn Error>> {
        if keys.is_empty() {
            return Ok(BulkReport::default());
        }
        let batch = policy.batch_size()?;
        let mut ids = BTreeMap::new();
        let mut unique = BTreeSet::new();
        for key in keys {
            if key.object_id.is_empty() || key.access_id.as_ref().is_some_and(String::is_empty) {
                return Err("bulk IDs must be nonempty".into());
            }
            if let Some(previous) = ids.insert(key.object_id.clone(), key.access_id.clone()) {
                if previous != key.access_id {
                    return Err("bulk MVP requires one access ID per object per call".into());
                }
            }
            unique.insert(key);
        }
        let keys = unique.into_iter().collect::<Vec<_>>();
        let chunks = keys
            .chunks(batch)
            .map(<[BulkKey]>::to_vec)
            .collect::<Vec<_>>();
        let count = AtomicUsize::new(0);
        let results = stream::iter(chunks)
            .map(|chunk| self.bulk_chunk(chunk, passports, access, policy, operation, &count))
            .buffer_unordered(policy.concurrency);
        futures_util::pin_mut!(results);
        let mut report = BulkReport::default();
        while let Some(chunk) = results.next().await {
            report.outcomes.extend(chunk.outcomes);
            report.diagnostics.extend(chunk.diagnostics);
        }
        Ok(report)
    }
    pub async fn get_bulk_objects(
        &self,
        ids: &[String],
        passports: &[String],
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<DrsObject>, Box<dyn Error>> {
        self.bulk(
            ids.iter()
                .map(|id| BulkKey {
                    object_id: id.clone(),
                    access_id: None,
                })
                .collect(),
            passports,
            false,
            policy,
            operation,
        )
        .await
    }
    pub async fn get_bulk_access_urls(
        &self,
        keys: &[BulkKey],
        passports: &[String],
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<BulkAccessUrl>, Box<dyn Error>> {
        if keys.iter().any(|k| k.access_id.is_none()) {
            return Err("bulk access lookup requires an access ID for each object".into());
        }
        self.bulk(keys.to_vec(), passports, true, policy, operation)
            .await
    }
}
