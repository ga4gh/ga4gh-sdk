//! Absolute operation budgets shared by retries, polling and multipart uploads.
use std::{error::Error, fmt, future::Future, time::Duration};
use tokio::time::Instant;
pub use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationError {
    Deadline,
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

#[derive(Clone, Debug)]
pub struct Operation {
    pub deadline: Instant,
    pub cancellation: CancellationToken,
}
impl Operation {
    pub fn new(budget: Duration) -> Self {
        Self {
            deadline: Instant::now() + budget,
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
    pub async fn sleep(&self, delay: Duration) -> Result<(), OperationError> {
        self.run(tokio::time::sleep(delay)).await
    }
}

/// Safety is asserted by the caller for the *operation*, not inferred from POST.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplaySafety {
    ReadOnly,
    NonReplayable,
}

#[derive(Clone, Debug)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub initial_delay: Duration,
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
