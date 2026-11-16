//! Absolute operation budgets shared by retries, polling and multipart uploads.
use std::{error::Error, fmt, future::Future, time::Duration};
use tokio::time::Instant;
pub use tokio_util::sync::CancellationToken;

/// Why an [`Operation`] ended before its work did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationError {
    /// The operation's deadline passed.
    Deadline,
    /// The operation's cancellation token was cancelled.
    Cancelled,
}
impl fmt::Display for OperationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Deadline => "operation deadline exceeded",
            Self::Cancelled => "operation canceled locally",
        })
    }
}
impl Error for OperationError {}

/// A time budget with an absolute deadline and a cancellation token, shared by
/// all steps of one operation: its requests, response bodies, retries and the
/// sleeps between them.
#[derive(Clone, Debug)]
pub struct Operation {
    /// When the operation ends.
    pub deadline: Instant,
    /// Cancels the operation when cancelled; clones of the token share it.
    pub cancellation: CancellationToken,
}
impl Operation {
    /// Starts an operation that ends `budget` from now. A budget beyond what
    /// the platform clock can represent ends in about 30 years instead.
    pub fn new(budget: Duration) -> Self {
        let now = Instant::now();
        Self {
            deadline: now
                .checked_add(budget)
                .unwrap_or_else(|| now + Duration::from_secs(30 * 365 * 86_400)),
            cancellation: CancellationToken::new(),
        }
    }
    /// Applies the same absolute deadline to headers, body and every sleep.
    pub async fn run<F: Future>(&self, future: F) -> Result<F::Output, OperationError> {
        if self.cancellation.is_cancelled() {
            return Err(OperationError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(OperationError::Deadline);
        }
        tokio::select! { biased;
            _ = self.cancellation.cancelled() => Err(OperationError::Cancelled),
            _ = tokio::time::sleep_until(self.deadline) => Err(OperationError::Deadline),
            result = future => Ok(result),
        }
    }
    /// Sleeps for `delay`, but not beyond the deadline.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Deadline`] if the deadline passes first,
    /// [`OperationError::Cancelled`] if the operation is cancelled.
    pub async fn sleep(&self, delay: Duration) -> Result<(), OperationError> {
        // Retry-After is untrusted and may overflow a platform Instant. Never
        // construct a sleep beyond the absolute remaining budget.
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        self.run(tokio::time::sleep(delay.min(remaining))).await
    }
}

/// Safety is asserted by the caller for the *operation*, not inferred from POST.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaySafety {
    /// The operation changes nothing on the server, so a request may be sent
    /// again, for example after a network error.
    ReadOnly,
    /// The operation may change state on the server, such as a submission, so
    /// a request is never sent twice.
    NonReplayable,
}

/// How often, and how far apart, a read-only request is tried.
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// The most attempts, the first included (1 to 32; default 1: no retries).
    pub max_attempts: usize,
    /// The longest delay before the first retry (default 200 ms); it doubles
    /// with each retry.
    pub initial_delay: Duration,
    /// The longest delay before any retry (default 10 s).
    pub max_delay: Duration,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
        }
    }
}
impl RetryPolicy {
    /// Checks the policy.
    ///
    /// # Errors
    ///
    /// Returns an error unless `max_attempts` is 1 to 32, `initial_delay` is
    /// positive and `max_delay` is at least `initial_delay`.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_attempts == 0
            || self.max_attempts > 32
            || self.initial_delay.is_zero()
            || self.max_delay < self.initial_delay
        {
            return Err("retry policy requires 1..=32 attempts and positive ordered delays");
        }
        Ok(())
    }
    /// The delay before retry number `attempt` (1 for the first retry): a random
    /// duration between half and all of a cap, which is `initial_delay`
    /// doubled `attempt - 1` times, but at most `max_delay`.
    pub fn delay(&self, attempt: usize) -> Duration {
        let cap = self
            .initial_delay
            .saturating_mul(
                1u32.checked_shl(attempt.saturating_sub(1).min(31) as u32)
                    .unwrap_or(u32::MAX),
            )
            .min(self.max_delay);
        let cap_ms = cap.as_millis().min(u64::MAX as u128) as u64;
        let half = cap_ms / 2;
        Duration::from_millis(
            half + (uuid::Uuid::new_v4().as_u128() % (cap_ms - half + 1) as u128) as u64,
        )
    }
}

/// A non-replayable operation lost its response. Reconcile remote state first.
#[derive(Debug)]
pub struct AmbiguousSubmissionError {
    /// What went wrong after the request was sent, such as a timeout.
    pub reason: String,
}
impl fmt::Display for AmbiguousSubmissionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "submission may have been accepted; reconcile before resubmitting: {}",
            self.reason
        )
    }
}
impl Error for AmbiguousSubmissionError {}

/// Delta seconds or HTTP date. Invalid values are reported to the caller.
pub fn retry_after(value: &str, now: std::time::SystemTime) -> Result<Duration, &'static str> {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return value
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|_| "Retry-After delta overflow");
    }
    let date = httpdate::parse_http_date(value).map_err(|_| "invalid Retry-After")?;
    Ok(date.duration_since(now).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::Operation;
    use std::time::Duration;
    use tokio::time::Instant;

    #[test]
    fn a_huge_budget_does_not_overflow() {
        let operation = Operation::new(Duration::MAX);
        assert!(operation.deadline > Instant::now() + Duration::from_secs(365 * 86_400));
    }
}
