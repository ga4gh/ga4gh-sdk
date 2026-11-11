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
///         submission: Duration::from_secs(600),
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
    /// and reading the answer. A submission that runs out of time after its
    /// request started fails with `AmbiguousSubmissionError`, because the
    /// service may have accepted it. Default: 300 seconds, which leaves time for
    /// uploads.
    pub submission: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            request: Duration::from_secs(60),
            discovery: Duration::from_secs(10),
            submission: Duration::from_secs(300),
        }
    }
}

impl Timeouts {
    /// The names of the limits, as in configuration files and the Python bindings.
    pub const NAMES: [&'static str; 4] = ["connect", "request", "discovery", "submission"];

    /// Sets the limit called `name` (one of [`Self::NAMES`]) to `seconds`.
    ///
    /// # Errors
    ///
    /// Returns a message if `name` is unknown, or if `seconds` isn't a positive,
    /// finite number of seconds that a `Duration` can hold.
    pub fn set_seconds(&mut self, name: &str, seconds: f64) -> Result<(), String> {
        let limit = match name {
            "connect" => &mut self.connect,
            "request" => &mut self.request,
            "discovery" => &mut self.discovery,
            "submission" => &mut self.submission,
            _ => {
                return Err(format!(
                    "unknown timeout '{name}'; use connect, request, discovery or submission"
                ))
            }
        };
        *limit = Duration::try_from_secs_f64(seconds)
            .ok()
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| format!("{name} must be a positive number of seconds"))?;
        Ok(())
    }

    /// The limits in seconds, by name, in the order of [`Self::NAMES`].
    pub fn as_seconds(&self) -> [(&'static str, f64); 4] {
        [
            ("connect", self.connect.as_secs_f64()),
            ("request", self.request.as_secs_f64()),
            ("discovery", self.discovery.as_secs_f64()),
            ("submission", self.submission.as_secs_f64()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::Timeouts;
    use std::time::Duration;

    #[test]
    fn set_seconds_sets_one_limit_by_name() {
        for (index, name) in Timeouts::NAMES.iter().enumerate() {
            let mut timeouts = Timeouts::default();
            timeouts.set_seconds(name, 1.5).unwrap();
            let seconds = timeouts.as_seconds();
            assert_eq!(seconds[index], (*name, 1.5));
            let mut expected = Timeouts::default().as_seconds();
            expected[index] = (*name, 1.5);
            assert_eq!(seconds, expected);
        }
        let mut timeouts = Timeouts::default();
        timeouts.set_seconds("connect", 0.25).unwrap();
        assert_eq!(timeouts.connect, Duration::from_millis(250));
    }

    #[test]
    fn set_seconds_rejects_unknown_names_and_invalid_values() {
        let mut timeouts = Timeouts::default();
        assert_eq!(
            timeouts.set_seconds("read", 1.0).unwrap_err(),
            "unknown timeout 'read'; use connect, request, discovery or submission"
        );
        for seconds in [0.0, -1.0, f64::NAN, f64::INFINITY, 1e300] {
            assert_eq!(
                timeouts.set_seconds("request", seconds).unwrap_err(),
                "request must be a positive number of seconds",
                "{seconds}"
            );
        }
        assert_eq!(timeouts, Timeouts::default());
    }
}
