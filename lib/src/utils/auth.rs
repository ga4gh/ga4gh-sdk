//! Bearer tokens from a caller-supplied provider.
//!
//! A [`TokenProvider`] gives the transport a token for each request, so that a
//! client can live longer than one token and can use a different token per
//! service. Set one with
//! [`Configuration::with_token_provider`](crate::utils::configuration::Configuration::with_token_provider).
//! The SDK only asks the provider; how it obtains tokens (a device flow, token
//! exchange, refresh) is up to the application.
//!
//! # Contract
//!
//! - The transport asks the provider before each request, passing the service's
//!   audience: the configured
//!   [`token_audience`](crate::utils::configuration::Configuration::token_audience)
//!   or, without one, the API root without user, password, query and fragment.
//!   It sends the token as `Authorization: Bearer <token>`.
//! - A token returned with [`AccessToken::reusable_until`] is reused for that
//!   audience until that instant, by all transports built from clones of the
//!   configuration. Return an earlier instant than the token's expiry to leave
//!   a margin.
//! - If the service answers `401 Unauthorized`, the transport drops the reused
//!   token, and asks the provider with [`TokenRequest::after_rejection`] set
//!   the next time. A replayable request is sent once more right away with the
//!   new token: a `GET` or `HEAD` of the transport's helpers and pages, or a
//!   request of
//!   [`Transport::request_with_policy`](crate::utils::transport::Transport::request_with_policy)
//!   with [`ReplaySafety::ReadOnly`](crate::utils::operation::ReplaySafety::ReadOnly).
//!   Other requests, such as task and run submissions, are never sent twice;
//!   their `401` comes back as an
//!   [`HttpError`](crate::utils::transport::HttpError).
//! - Asking the provider counts towards the time limit of discovery, of a page
//!   and of a submission. Running out of time there is not an ambiguous
//!   submission, as nothing was sent.
//! - A provider is one credential source on its own: it can't be combined with
//!   `basic_auth`, `oauth_access_token`, `bearer_access_token`, `api_key` or a
//!   user or password in `base_path`. Such a combination fails with an
//!   [`AuthConflictError`] before anything is sent. Without a provider, the
//!   transport behaves as before.
//! - A provider set on the configuration of
//!   [`Transport::with_client`](crate::utils::transport::Transport::with_client)
//!   applies to that client's requests too, and its header takes the place of an
//!   `Authorization` default header of the client.
//! - [`Transport::request_builder`](crate::utils::transport::Transport::request_builder)
//!   adds no token, so a request built with it and sent with
//!   [`Transport::send_once`](crate::utils::transport::Transport::send_once)
//!   carries none; use
//!   [`Transport::authorized_request_builder`](crate::utils::transport::Transport::authorized_request_builder).
//!   Fetches of descriptors and result artifacts from other URLs never carry it.
//! - A provider failure is a [`TokenProviderError`], not an HTTP error, and
//!   nothing is sent. Token values never appear in `Debug` output, errors or logs
//!   of the SDK. The text of a provider's own error is shown as the provider wrote
//!   it, so keep secrets out of it.

use async_trait::async_trait;
use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// The error type of [`TokenProvider::token`].
pub type BoxError = Box<dyn Error + Send + Sync>;

/// What the transport asks a [`TokenProvider`] for.
#[derive(Debug)]
#[non_exhaustive]
pub struct TokenRequest<'a> {
    /// The service's audience; see the [module documentation](self).
    pub audience: &'a str,
    /// Whether the service has just rejected the previous token for this
    /// audience with `401 Unauthorized`; a provider with a cache of its own
    /// should then get a new token.
    pub after_rejection: bool,
}

/// A bearer token, and how long the transport may reuse it.
#[derive(Clone)]
pub struct AccessToken {
    secret: String,
    reusable_until: Option<Instant>,
}

impl AccessToken {
    /// A token that the transport asks for again before each request.
    pub fn new(secret: impl Into<String>) -> Self {
        Self {
            secret: secret.into(),
            reusable_until: None,
        }
    }

    /// Lets the transport reuse the token until `until`.
    pub fn reusable_until(mut self, until: Instant) -> Self {
        self.reusable_until = Some(until);
        self
    }

    /// The token itself. Don't log it.
    pub fn secret(&self) -> &str {
        &self.secret
    }

    /// Until when the transport may reuse the token, if it may.
    pub fn reuse_deadline(&self) -> Option<Instant> {
        self.reusable_until
    }
}

impl fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessToken")
            .field("secret", &format_args!("<redacted>"))
            .field("reusable_until", &self.reusable_until)
            .finish()
    }
}

/// Gives bearer tokens on demand; see the [module documentation](self).
///
/// # Example
///
/// ```
/// use ga4gh_sdk::utils::auth::{AccessToken, BoxError, TokenProvider, TokenRequest};
/// use ga4gh_sdk::utils::configuration::Configuration;
/// use std::time::{Duration, Instant};
///
/// struct MyProvider;
///
/// #[async_trait::async_trait]
/// impl TokenProvider for MyProvider {
///     async fn token(&self, request: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
///         // Get a token for `request.audience` here, for example by token exchange.
///         let (token, lifetime) = ("my-token".to_owned(), Duration::from_secs(300));
///         // Reuse it until 30 seconds before it expires.
///         Ok(AccessToken::new(token).reusable_until(Instant::now() + lifetime - Duration::from_secs(30)))
///     }
/// }
///
/// let config = Configuration::new(url::Url::parse("https://example.org/ga4gh/tes/v1").unwrap())
///     .with_token_provider(MyProvider);
/// ```
#[async_trait]
pub trait TokenProvider: Send + Sync {
    /// Gives a token for `request.audience`.
    ///
    /// # Errors
    ///
    /// Any error; the transport returns it inside a [`TokenProviderError`]
    /// without sending the request.
    async fn token(&self, request: &TokenRequest<'_>) -> Result<AccessToken, BoxError>;
}

impl AccessToken {
    /// Whether the transport may still reuse the token at `now`.
    fn is_reusable(&self, now: Instant) -> bool {
        self.reusable_until.is_some_and(|until| now < until)
    }
}

/// The reused token of one audience.
#[derive(Default)]
struct Slot {
    token: Option<AccessToken>,
    /// Counts rejections, so that a provider call that started before one
    /// doesn't put back the token that was rejected.
    generation: u64,
    /// Whether the service rejected a token since the provider was last asked.
    rejected: bool,
}

/// A [`TokenProvider`] with the tokens it allowed to reuse, per audience.
///
/// Clones share the provider and the reused tokens.
#[derive(Clone)]
pub struct TokenSource {
    provider: Arc<dyn TokenProvider>,
    reused: Arc<Mutex<HashMap<String, Slot>>>,
}

impl fmt::Debug for TokenSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TokenSource")
    }
}

impl TokenSource {
    /// Wraps `provider`, without reused tokens.
    pub fn new(provider: Arc<dyn TokenProvider>) -> Self {
        Self {
            provider,
            reused: Default::default(),
        }
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        // A poisoned lock only means that another thread panicked while it held
        // it; the map stays usable.
        self.reused.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drops the reused token of `audience` after the service rejected a token,
    /// so that the next request asks the provider, with
    /// [`TokenRequest::after_rejection`] set.
    pub(crate) fn reject(&self, audience: &str) {
        let mut slots = self.slots();
        let slot = slots.entry(audience.to_owned()).or_default();
        slot.token = None;
        slot.generation += 1;
        slot.rejected = true;
    }

    /// A token for `audience`: a reusable one, unless `after_rejection`, or a
    /// new one from the provider.
    pub(crate) async fn token(
        &self,
        audience: &str,
        after_rejection: bool,
    ) -> Result<AccessToken, TokenProviderError> {
        if after_rejection {
            self.reject(audience);
        }
        let (generation, after_rejection) = {
            let mut slots = self.slots();
            let slot = slots.entry(audience.to_owned()).or_default();
            if let Some(token) = slot
                .token
                .as_ref()
                .filter(|t| t.is_reusable(Instant::now()))
            {
                return Ok(token.clone());
            }
            slot.token = None;
            (slot.generation, std::mem::take(&mut slot.rejected))
        };
        let request = TokenRequest {
            audience,
            after_rejection,
        };
        let token = match self.provider.token(&request).await {
            Ok(token) => token,
            Err(source) => {
                // The next call asks after the rejection again.
                if after_rejection {
                    self.slots()
                        .entry(audience.to_owned())
                        .or_default()
                        .rejected = true;
                }
                return Err(TokenProviderError::Failed {
                    audience: audience.to_owned(),
                    source,
                });
            }
        };
        if token.is_reusable(Instant::now()) {
            let mut slots = self.slots();
            let slot = slots.entry(audience.to_owned()).or_default();
            if slot.generation == generation {
                slot.token = Some(token.clone());
            }
        }
        Ok(token)
    }
}

/// The token provider failed or gave a token that can't be sent; nothing was sent.
#[derive(Debug)]
#[non_exhaustive]
pub enum TokenProviderError {
    /// The provider returned an error.
    Failed {
        /// The audience the token was asked for.
        audience: String,
        /// The provider's error.
        source: BoxError,
    },
    /// The token is empty or has characters that an HTTP header can't hold.
    InvalidToken {
        /// The audience the token was asked for.
        audience: String,
    },
}

impl fmt::Display for TokenProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { audience, source } => {
                write!(f, "The token provider failed for {audience}: {source}")
            }
            Self::InvalidToken { audience } => write!(
                f,
                "The token provider gave an empty token or one that isn't a valid HTTP header value for {audience}"
            ),
        }
    }
}

impl Error for TokenProviderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Failed { source, .. } => Some(source.as_ref()),
            Self::InvalidToken { .. } => None,
        }
    }
}

/// A token provider is configured together with other credentials; nothing was sent.
#[derive(Debug)]
#[non_exhaustive]
pub struct AuthConflictError {
    /// The other credentials, by field name, such as `oauth_access_token`.
    pub sources: Vec<&'static str>,
}

impl fmt::Display for AuthConflictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "A token provider can't be combined with other credentials; remove {} or the token provider",
            self.sources.join(", ")
        )
    }
}

impl Error for AuthConflictError {}

/// Gives the same token each time, without reuse.
#[cfg(test)]
pub(crate) struct FixedToken(pub &'static str);

#[cfg(test)]
#[async_trait]
impl TokenProvider for FixedToken {
    async fn token(&self, _: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
        Ok(AccessToken::new(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    /// Gives `token-<n>` for the n-th call, reusable for `lifetime` if given.
    struct Counting {
        calls: AtomicUsize,
        lifetime: Option<Duration>,
    }

    #[async_trait]
    impl TokenProvider for Counting {
        async fn token(&self, _: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            let token = AccessToken::new(format!("token-{n}"));
            Ok(match self.lifetime {
                Some(lifetime) => token.reusable_until(Instant::now() + lifetime),
                None => token,
            })
        }
    }

    fn counting(lifetime: Option<Duration>) -> (TokenSource, Arc<Counting>) {
        let provider = Arc::new(Counting {
            calls: AtomicUsize::new(0),
            lifetime,
        });
        (TokenSource::new(provider.clone()), provider)
    }

    #[tokio::test]
    async fn test_reuses_tokens_per_audience_until_their_deadline() {
        let (source, provider) = counting(Some(Duration::from_secs(60)));
        assert_eq!(source.token("a", false).await.unwrap().secret(), "token-1");
        assert_eq!(source.token("a", false).await.unwrap().secret(), "token-1");
        assert_eq!(
            source.clone().token("b", false).await.unwrap().secret(),
            "token-2"
        );
        assert_eq!(source.token("a", true).await.unwrap().secret(), "token-3");
        assert_eq!(source.token("a", false).await.unwrap().secret(), "token-3");
        assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_asks_again_without_or_after_deadline() {
        let (source, provider) = counting(None);
        source.token("a", false).await.unwrap();
        source.token("a", false).await.unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);

        // A deadline that has passed already.
        let (source, provider) = counting(Some(Duration::ZERO));
        source.token("a", false).await.unwrap();
        source.token("a", false).await.unwrap();
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    }

    /// Records `after_rejection` of each call.
    #[derive(Default)]
    struct Rejections(Mutex<Vec<bool>>);

    #[async_trait]
    impl TokenProvider for Rejections {
        async fn token(&self, request: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
            let mut calls = self.0.lock().unwrap();
            calls.push(request.after_rejection);
            let token = AccessToken::new(format!("token-{}", calls.len()));
            Ok(token.reusable_until(Instant::now() + Duration::from_secs(60)))
        }
    }

    #[tokio::test]
    async fn test_reject_asks_again_after_the_rejection() {
        let provider = Arc::new(Rejections::default());
        let source = TokenSource::new(provider.clone());
        assert_eq!(source.token("a", false).await.unwrap().secret(), "token-1");
        source.reject("a");
        assert_eq!(source.token("a", false).await.unwrap().secret(), "token-2");
        assert_eq!(source.token("a", false).await.unwrap().secret(), "token-2");
        assert_eq!(*provider.0.lock().unwrap(), [false, true]);
    }

    #[test]
    fn test_debug_output_hides_the_token() {
        let token = AccessToken::new("SENTINEL_TOKEN");
        let output = format!("{token:?}");
        assert_eq!(
            output,
            "AccessToken { secret: <redacted>, reusable_until: None }"
        );
        let (source, _) = counting(None);
        assert_eq!(format!("{source:?}"), "TokenSource");
    }

    #[test]
    fn test_error_messages() {
        let error = TokenProviderError::Failed {
            audience: "https://example.org/".to_owned(),
            source: "no session".into(),
        };
        assert_eq!(
            error.to_string(),
            "The token provider failed for https://example.org/: no session"
        );
        assert_eq!(error.source().unwrap().to_string(), "no session");
        let error = AuthConflictError {
            sources: vec!["basic_auth", "oauth_access_token"],
        };
        assert_eq!(
            error.to_string(),
            "A token provider can't be combined with other credentials; remove basic_auth, oauth_access_token or the token provider"
        );
    }
}
