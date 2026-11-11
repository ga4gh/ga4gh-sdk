//! Time limits of the HTTP transport and of the high-level clients.
//!
//! A [`Configuration`](crate::utils::configuration::Configuration) carries one
//! [`Timeouts`]; [`Transport::new`](crate::utils::transport::Transport::new) and the
//! TES, WES, DRS and TRS clients read it from there. `connect` limits opening each
//! connection. Each request then has one of these limits:
//!
//! - service-info requests of `new`, `with_transport`, `discover`, `refresh` and
//!   `TRS::service_info`: `discovery`, in place of `request`;
//! - task and run submissions of `TES::create`, `TES::create_value`,
//!   `WES::run_workflow` and `WES::run_workflow_with_attachments`: `submission`,
//!   in place of `request`;
//! - `Transport::send_once` and `WES::run_workflow_streaming`: the deadline of the
//!   caller's [`Operation`](crate::utils::operation::Operation), in place of
//!   `request`;
//! - every other request: `request`. Calls with an `Operation` budget of their
//!   own, such as waiting, paging and DRS resolution, also stop at its deadline,
//!   whichever comes first.
//!
//! So `discovery` and `submission` can be longer or shorter than `request`. With
//! [`Transport::with_client`](crate::utils::transport::Transport::with_client), the
//! client's own connect and request timeouts apply instead of `connect` and
//! `request`. The `max_age` of capability snapshots and the `Retry-After` limits
//! of retries and DRS resolution are not time limits of requests and don't
//! depend on these.
use std::time::Duration;

/// Time limits of the HTTP transport and of the high-level clients.
///
/// Build it with `..Default::default()`, so that limits added later don't break
/// your code:
///
/// ```
/// use ga4gh_sdk::utils::configuration::Configuration;
/// use ga4gh_sdk::utils::timeouts::Timeouts;
/// use std::time::Duration;
///
/// let config = Configuration::new(url::Url::parse("https://example.org/ga4gh/tes/v1").unwrap())
///     .with_timeouts(Timeouts {
///         discovery: Duration::from_secs(30),
///         submission: Duration::from_secs(300),
///         ..Default::default()
///     });
/// # let _ = config;
/// ```
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Timeouts {
    /// Opening a connection, including TLS. Default: 10 seconds.
    pub connect: Duration,
    /// One request, from sending it to reading the whole response, unless one of
    /// the limits below applies instead. Default: 60 seconds.
    pub request: Duration,
    /// The service-info request with which clients check a service. Default:
    /// 10 seconds.
    pub discovery: Duration,
    /// One TES task or WES run submission: a single attempt, including uploads
    /// and reading the answer. A submission that runs out of time after it was
    /// sent fails with `AmbiguousSubmissionError`, because the service may have
    /// accepted it. Default: 60 seconds.
    pub submission: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(60),
            discovery: Duration::from_secs(10),
            submission: Duration::from_secs(60),
        }
    }
}
