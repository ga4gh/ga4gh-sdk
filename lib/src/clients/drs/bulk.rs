//! DRS 1.4 bulk lookup, one explicit service/credential context per call.
//! Duplicate inputs collapse. The MVP accepts one access ID per object per call;
//! object-only grouped errors can then be attributed without fabricating an ID.
//!
//! Object lookups also read one documented non-standard reply shape: Gen3
//! (indexd) names the resolved array `resolved_drs_objects` instead of
//! `resolved_drs_object`. [`BulkReport::dialects`] records the shapes seen in
//! a call. Requests are always standard, and no other dialect is inferred.
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
    fmt,
    sync::atomic::{AtomicUsize, Ordering},
};

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct BulkKey {
    pub object_id: String,
    pub access_id: Option<String>,
}
impl fmt::Debug for BulkKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BulkKey")
            .field("object_id", &self.object_id)
            .field("access_id", &self.access_id.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BulkPending {
    pub api_root: String,
    pub endpoint: String,
    pub retry_after_secs: u64,
}
/// Multiple returned values are retained rather than silently selecting a duplicate.
/// Debug output shows counts because resolved access URLs and error text may
/// contain credentials. Inspect the public fields only in an appropriate context.
pub struct ObjectOutcome<T> {
    pub resolved: Vec<T>,
    pub failure_codes: Vec<u16>,
    pub pending: Option<BulkPending>,
    pub errors: Vec<String>,
}
impl<T> fmt::Debug for ObjectOutcome<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ObjectOutcome")
            .field("resolved_count", &self.resolved.len())
            .field("failure_codes", &self.failure_codes)
            .field("pending", &self.pending)
            .field("error_count", &self.errors.len())
            .finish()
    }
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
/// Reply shape of a bulk response body, as recorded in [`BulkReport::dialects`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[non_exhaustive]
pub enum BulkDialect {
    /// Field names of the published DRS 1.4 and 1.5 schemas.
    Standard,
    /// Gen3 (indexd) object lookup: resolved objects in `resolved_drs_objects`.
    /// Its objects can also carry `created_time: null`, which the typed
    /// [`DrsObject`] cannot represent; read them with
    /// [`DRS::get_bulk_objects_raw`].
    Gen3,
}
/// Result of one bulk call. Fields may be added; build it with `Default`.
#[derive(Debug)]
#[non_exhaustive]
pub struct BulkReport<T> {
    /// One outcome per distinct requested key.
    pub outcomes: BTreeMap<BulkKey, ObjectOutcome<T>>,
    /// Reply problems that cannot be attributed to one requested key.
    pub diagnostics: Vec<String>,
    /// Reply shapes seen across all response bodies of this call, merged over
    /// chunks and 413 splits. Empty when no body had a resolved array, e.g.
    /// after HTTP failures only, or when a reply had unsupported fields.
    pub dialects: BTreeSet<BulkDialect>,
}
impl<T> Default for BulkReport<T> {
    fn default() -> Self {
        Self {
            outcomes: BTreeMap::new(),
            diagnostics: Vec::new(),
            dialects: BTreeSet::new(),
        }
    }
}
impl<T> BulkReport<T> {
    fn merge(&mut self, other: Self) {
        self.outcomes.extend(other.outcomes);
        self.diagnostics.extend(other.diagnostics);
        self.dialects.extend(other.dialects);
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
fn require_access_ids(keys: &[BulkKey]) -> Result<(), Box<dyn Error>> {
    if keys.iter().any(|k| k.access_id.is_none()) {
        return Err("bulk access lookup requires an access ID for each object".into());
    }
    Ok(())
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
    const GEN3_FIELD: &str = "resolved_drs_objects";
    let field = if access {
        "resolved_drs_object_access_urls"
    } else {
        "resolved_drs_object"
    };
    // A null field counts as absent, so it cannot hide the other array.
    let present = |name: &str| body.get(name).filter(|value| !value.is_null());
    let gen3 = if access { None } else { present(GEN3_FIELD) };
    let resolved = match (present(field), gen3) {
        (Some(values), gen3) => {
            if gen3.is_some() {
                report.diagnostics.push(format!(
                    "response contains both {field} and {GEN3_FIELD}; {GEN3_FIELD} was ignored"
                ));
            }
            Some((values, BulkDialect::Standard))
        }
        (None, Some(values)) => Some((values, BulkDialect::Gen3)),
        (None, None) => None,
    };
    // Starter Kit 1.3.0experimental reports failures in this map; it is not
    // interpreted, and the reply is not attributed to any dialect.
    let unsupported = !access && body.get("unresolved_drs_object").is_some();
    if unsupported {
        report.diagnostics.push(
            "response contains unresolved_drs_object, which is not a DRS field; it was ignored"
                .into(),
        );
    }
    if let Some((values, dialect)) = resolved {
        if let Some(values) = values.as_array() {
            if !unsupported {
                report.dialects.insert(dialect);
            }
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
                        Err(error) => {
                            let mut message = format!("invalid resolved entry: {error}");
                            if !access && value.get("created_time").is_some_and(Value::is_null) {
                                message.push_str(
                                    "; created_time is null, but DRS requires a date-time \
                                     string (get_bulk_objects_raw returns the entry unchanged)",
                                );
                            }
                            outcome.errors.push(message)
                        }
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
fn object_keys(ids: &[String]) -> Vec<BulkKey> {
    ids.iter()
        .map(|id| BulkKey {
            object_id: id.clone(),
            access_id: None,
        })
        .collect()
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
                    report.merge(correlate(&keys, body, access));
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
            report.merge(chunk);
        }
        Ok(report)
    }
    /// Looks up DRS objects in bounded `POST /objects` requests.
    ///
    /// Each resolved entry must match the DRS schema. An entry that does not,
    /// such as a Gen3 object with `created_time: null`, becomes an error in its
    /// outcome; use [`DRS::get_bulk_objects_raw`] to read it unchanged.
    pub async fn get_bulk_objects(
        &self,
        ids: &[String],
        passports: &[String],
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<DrsObject>, Box<dyn Error>> {
        self.bulk(object_keys(ids), passports, false, policy, operation)
            .await
    }
    /// Like [`DRS::get_bulk_objects`], but returns each resolved entry as
    /// JSON, correlated by its `id`, without schema validation. Fields the SDK
    /// model omits or cannot represent, e.g. `created_time: null`, are kept.
    /// Identity checks, diagnostics and dialect recording are the same.
    pub async fn get_bulk_objects_raw(
        &self,
        ids: &[String],
        passports: &[String],
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<Value>, Box<dyn Error>> {
        self.bulk(object_keys(ids), passports, false, policy, operation)
            .await
    }
    /// Returned access URLs and headers may be credentials. The report's Debug
    /// output omits them; logging a raw generated `BulkAccessUrl` does not.
    pub async fn get_bulk_access_urls(
        &self,
        keys: &[BulkKey],
        passports: &[String],
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<BulkAccessUrl>, Box<dyn Error>> {
        require_access_ids(keys)?;
        self.bulk(keys.to_vec(), passports, true, policy, operation)
            .await
    }
    /// Like [`DRS::get_bulk_access_urls`], but keeps each resolved entry's JSON, including
    /// fields that the SDK models don't know. Resolved entries are not checked against
    /// the `BulkAccessUrl` model; they may hold credentials, as there.
    pub async fn get_bulk_access_urls_raw(
        &self,
        keys: &[BulkKey],
        passports: &[String],
        policy: &BulkPolicy,
        operation: &Operation,
    ) -> Result<BulkReport<Value>, Box<dyn Error>> {
        require_access_ids(keys)?;
        self.bulk(keys.to_vec(), passports, true, policy, operation)
            .await
    }
}
