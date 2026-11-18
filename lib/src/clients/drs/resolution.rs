//! Reads DRS answers that a server prepares later, keeping their pending
//! state instead of parsing an empty `202`.
//!
//! A DRS server can answer `202 Accepted` with a `Retry-After` header while
//! it prepares an object or access URL. The `_once` methods of [`DRS`] send
//! one request and give a [`Resolution`]: the value, or a
//! [`PendingResolution`] to retry later, also from another process. The
//! `resolve_` methods retry until the value is ready, within an
//! [`Operation`] and a number of attempts.
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

/// The request of a deferred resolution, without credentials, so that it can
/// be saved and sent again.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct ResolutionReference {
    /// API root without credentials. Supply configuration again when resuming.
    pub api_root: String,
    /// The endpoint below the API root, such as `objects/<id>`.
    pub endpoint: String,
}
/// The server is preparing the answer; ask again after `retry_after_secs`.
#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub struct PendingResolution {
    /// The request to send again.
    pub reference: ResolutionReference,
    /// How many seconds to wait, from the server's `Retry-After` (1 to 3600).
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
/// The answer to one resolution request.
#[derive(Debug)]
pub enum Resolution<T> {
    /// The server sent the value.
    Ready(T),
    /// The server answered `202 Accepted`; the value isn't ready yet.
    Pending(PendingResolution),
}

/// Why a resolution with retries failed.
#[derive(Debug)]
pub enum ResolveCause {
    /// The operation's deadline passed or it was cancelled.
    Operation(OperationError),
    /// Every attempt was answered with `202 Accepted`.
    AttemptsExhausted,
    /// A request failed, its answer was invalid, or `max_attempts` was out
    /// of range.
    Request(crate::Error),
}
/// A resolution with retries failed; it can be resumed from `reference`.
#[derive(Debug)]
pub struct ResolveError {
    /// The request that was being resolved.
    pub reference: ResolutionReference,
    /// The last `202 Accepted` answer, if any.
    pub last_pending: Option<PendingResolution>,
    /// Why the resolution failed.
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
impl Error for ResolveError {
    /// The error that stopped the resolution, if any, so that
    /// [`crate::Error::downcast_ref`] finds it.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.cause {
            ResolveCause::Operation(error) => Some(error),
            ResolveCause::Request(error) => Some(error),
            ResolveCause::AttemptsExhausted => None,
        }
    }
}

/// Calls `attempt` up to `max_attempts` times (1 to 10 000), waiting the
/// server's `Retry-After` between pending answers, until it gives a value.
///
/// Opt-in bounded state machine. Local cancellation stops observation only.
/// Zero seconds is clamped to one; missing/malformed/overflow/>1h delays fail.
///
/// # Errors
///
/// Fails with the [`ResolveCause`] if `max_attempts` is out of range, an
/// attempt fails, the operation ends, or every attempt is pending.
// ResolveError exposes structured context; boxing it would change the public API.
#[allow(clippy::result_large_err)]
pub async fn resolve_until<T, F, Fut>(
    reference: ResolutionReference,
    operation: &Operation,
    max_attempts: usize,
    mut attempt: F,
) -> Result<T, ResolveError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = crate::Result<Resolution<T>>>,
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
    /// The reference of `endpoint` at this client's API root, without
    /// credentials, query or fragment.
    ///
    /// The API root is the base path of [`DRS::configuration`], the
    /// transport's configuration, so a reference always names the service
    /// that received the request.
    pub fn resolution_reference(&self, endpoint: String) -> ResolutionReference {
        let mut root = self.configuration().base_path.clone();
        let _ = root.set_username("");
        let _ = root.set_password(None);
        root.set_query(None);
        root.set_fragment(None);
        if !root.path().ends_with('/') {
            root.set_path(&format!("{}/", root.path()));
        }
        ResolutionReference {
            api_root: root.to_string(),
            endpoint,
        }
    }

    /// Single request only; never sleeps or retries. A reference is credential-free.
    ///
    /// # Errors
    ///
    /// Fails as [`DRS::resolve_reference_raw`] does, or if the body doesn't
    /// match `T`.
    pub async fn resolve_reference<T: DeserializeOwned>(
        &self,
        reference: &ResolutionReference,
    ) -> crate::Result<Resolution<T>> {
        match self.resolve_reference_raw(reference).await? {
            Resolution::Ready(body) => Ok(Resolution::Ready(serde_json::from_str(&body)?)),
            Resolution::Pending(pending) => Ok(Resolution::Pending(pending)),
        }
    }

    /// Like [`DRS::resolve_reference`], but keeps the server's JSON text, including fields
    /// that the SDK models don't know.
    ///
    /// # Errors
    ///
    /// Fails if `reference` belongs to another API root, the request fails,
    /// or a `202 Accepted` answer has no valid `Retry-After` of at most an
    /// hour.
    pub async fn resolve_reference_raw(
        &self,
        reference: &ResolutionReference,
    ) -> crate::Result<Resolution<String>> {
        if reference.api_root != self.resolution_reference(String::new()).api_root {
            return Err("DRS resolution reference belongs to another API root".into());
        }
        let response = self
            .transport()
            .request_response(reqwest::Method::GET, &reference.endpoint, None, None)
            .await?;
        if let Some(seconds) = deferred_delay(&response)? {
            return Ok(Resolution::Pending(PendingResolution {
                reference: reference.clone(),
                retry_after_secs: seconds,
            }));
        }
        let body = response_text(response, BodyLimits::default()).await?;
        Ok(Resolution::Ready(body))
    }
    /// Reads an object with one request, which may be pending.
    ///
    /// # Errors
    ///
    /// Fails as [`DRS::resolve_reference`] does.
    pub async fn get_object_once(&self, object_id: &str) -> crate::Result<Resolution<DrsObject>> {
        self.resolve_reference(
            &self.resolution_reference(format!("objects/{}", super::urlencode(object_id))),
        )
        .await
    }
    /// Reads an access URL of an object with one request, which may be
    /// pending.
    ///
    /// # Errors
    ///
    /// Fails as [`DRS::resolve_reference`] does.
    pub async fn get_access_url_once(
        &self,
        object_id: &str,
        access_id: &str,
    ) -> crate::Result<Resolution<AccessUrl>> {
        self.resolve_reference(&self.resolution_reference(format!(
            "objects/{}/access/{}",
            super::urlencode(object_id),
            super::urlencode(access_id)
        )))
        .await
    }
    /// Reads an object, retrying pending answers as [`resolve_until`] does.
    ///
    /// # Errors
    ///
    /// Fails as [`resolve_until`] does.
    #[allow(clippy::result_large_err)]
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
    /// Reads an access URL of an object, retrying pending answers as
    /// [`resolve_until`] does.
    ///
    /// # Errors
    ///
    /// Fails as [`resolve_until`] does.
    #[allow(clippy::result_large_err)]
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

/// Gives the delay in seconds (at least 1) of a `202 Accepted` response, and
/// `None` for any other status.
///
/// Shared DRS 1.4 deferred-response interpretation for singular and bulk reads.
///
/// # Errors
///
/// Fails if a `202` response has no `Retry-After`, or one that isn't a whole
/// number of seconds up to 3600.
pub fn deferred_delay(response: &reqwest::Response) -> crate::Result<Option<u64>> {
    if response.status() != reqwest::StatusCode::ACCEPTED {
        return Ok(None);
    }
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
    Ok(Some(seconds.max(1)))
}
