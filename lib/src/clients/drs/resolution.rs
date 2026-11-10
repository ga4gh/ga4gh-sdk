//! DRS deferred resolution preserves pending state instead of parsing an empty 202.
use super::{
    models::{AccessUrl, DrsObject},
    DRS,
};
use crate::utils::{
    operation::{Operation, OperationError},
    transport::{response_text, BodyLimits},
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{error::Error, fmt, future::Future, time::Duration};

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResolutionReference {
    /// API root without credentials. Supply configuration again when resuming.
    pub api_root: String,
    pub endpoint: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PendingResolution {
    pub reference: ResolutionReference,
    pub retry_after_secs: u64,
}
impl fmt::Display for PendingResolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DRS resolution pending; retry after {} seconds",
            self.retry_after_secs
        )
    }
}
impl Error for PendingResolution {}
#[derive(Debug)]
pub enum Resolution<T> {
    Ready(T),
    Pending(PendingResolution),
}

#[derive(Debug)]
pub enum ResolveCause {
    Operation(OperationError),
    AttemptsExhausted,
    Request(Box<dyn Error>),
}
#[derive(Debug)]
pub struct ResolveError {
    pub reference: ResolutionReference,
    pub last_pending: Option<PendingResolution>,
    pub cause: ResolveCause,
}
impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.cause {
            ResolveCause::Operation(e) => write!(f, "DRS resolution interrupted: {e}"),
            ResolveCause::AttemptsExhausted => {
                f.write_str("DRS resolution attempts exhausted; reference retained")
            }
            ResolveCause::Request(e) => write!(f, "DRS resolution request failed: {e}"),
        }
    }
}
impl Error for ResolveError {}

/// Opt-in bounded state machine. Local cancellation stops observation only.
/// Zero seconds is clamped to one; missing/malformed/overflow/>1h delays fail.
pub async fn resolve_until<T, F, Fut>(
    reference: ResolutionReference,
    operation: &Operation,
    max_attempts: usize,
    mut attempt: F,
) -> Result<T, ResolveError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Resolution<T>, Box<dyn Error>>>,
{
    let mut pending = None;
    let failure = |cause, pending| ResolveError {
        reference: reference.clone(),
        last_pending: pending,
        cause,
    };
    if max_attempts == 0 || max_attempts > 10_000 {
        return Err(failure(
            ResolveCause::Request("resolution attempts must be 1..=10000".into()),
            pending,
        ));
    }
    for index in 0..max_attempts {
        match operation.run(attempt()).await {
            Ok(Ok(Resolution::Ready(value))) => return Ok(value),
            Ok(Ok(Resolution::Pending(value))) => pending = Some(value),
            Ok(Err(error)) => return Err(failure(ResolveCause::Request(error), pending)),
            Err(error) => return Err(failure(ResolveCause::Operation(error), pending)),
        }
        if index + 1 < max_attempts {
            let delay = Duration::from_secs(pending.as_ref().unwrap().retry_after_secs.max(1));
            if let Err(error) = operation.sleep(delay).await {
                return Err(failure(ResolveCause::Operation(error), pending));
            }
        }
    }
    Err(failure(ResolveCause::AttemptsExhausted, pending))
}

impl DRS {
    pub fn resolution_reference(&self, endpoint: String) -> ResolutionReference {
        let mut root = self.config.base_path.clone();
        let _ = root.set_username("");
        let _ = root.set_password(None);
        root.set_query(None);
        root.set_fragment(None);
        ResolutionReference {
            api_root: root.to_string(),
            endpoint,
        }
    }

    /// Single request only; never sleeps or retries. A reference is credential-free.
    pub async fn resolve_reference<T: DeserializeOwned>(
        &self,
        reference: &ResolutionReference,
    ) -> Result<Resolution<T>, Box<dyn Error>> {
        if reference.api_root != self.resolution_reference(String::new()).api_root {
            return Err("DRS resolution reference belongs to another API root".into());
        }
        let response = self
            .transport
            .request_response(reqwest::Method::GET, &reference.endpoint, None, None)
            .await?;
        if response.status() == reqwest::StatusCode::ACCEPTED {
            let value = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .ok_or("DRS 202 missing Retry-After")?
                .to_str()?;
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err("DRS Retry-After must be integral seconds".into());
            }
            let seconds: u64 = value.parse().map_err(|_| "DRS Retry-After overflow")?;
            if seconds > 3600 {
                return Err("DRS Retry-After exceeds the supported one-hour bound".into());
            }
            return Ok(Resolution::Pending(PendingResolution {
                reference: reference.clone(),
                retry_after_secs: seconds.max(1),
            }));
        }
        let body = response_text(response, BodyLimits::default()).await?;
        Ok(Resolution::Ready(serde_json::from_str(&body)?))
    }
    pub async fn get_object_once(
        &self,
        object_id: &str,
    ) -> Result<Resolution<DrsObject>, Box<dyn Error>> {
        self.resolve_reference(
            &self.resolution_reference(format!("objects/{}", super::urlencode(object_id))),
        )
        .await
    }
    pub async fn get_access_url_once(
        &self,
        object_id: &str,
        access_id: &str,
    ) -> Result<Resolution<AccessUrl>, Box<dyn Error>> {
        self.resolve_reference(&self.resolution_reference(format!(
            "objects/{}/access/{}",
            super::urlencode(object_id),
            super::urlencode(access_id)
        )))
        .await
    }
    pub async fn resolve_object(
        &self,
        object_id: &str,
        operation: &Operation,
        max_attempts: usize,
    ) -> Result<DrsObject, ResolveError> {
        let reference =
            self.resolution_reference(format!("objects/{}", super::urlencode(object_id)));
        resolve_until(reference, operation, max_attempts, || {
            self.get_object_once(object_id)
        })
        .await
    }
    pub async fn resolve_access_url(
        &self,
        object_id: &str,
        access_id: &str,
        operation: &Operation,
        max_attempts: usize,
    ) -> Result<AccessUrl, ResolveError> {
        let reference = self.resolution_reference(format!(
            "objects/{}/access/{}",
            super::urlencode(object_id),
            super::urlencode(access_id)
        ));
        resolve_until(reference, operation, max_attempts, || {
            self.get_access_url_once(object_id, access_id)
        })
        .await
    }
}
