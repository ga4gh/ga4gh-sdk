//! Validation of execution acknowledgements. Never replay an ambiguous submission.

use std::{error::Error, fmt};

/// A successful HTTP response did not identify the accepted operation.
/// The remote operation may already have succeeded; reconcile before resubmitting.
#[derive(Debug)]
pub struct AcknowledgementError {
    pub operation: &'static str,
}

impl fmt::Display for AcknowledgementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} returned an invalid acknowledgement; the remote operation may have succeeded. Reconcile before retrying", self.operation)?;
        if self.operation == "WES cancellation" {
            write!(f, "; cancellation cannot be confirmed")?;
        }
        Ok(())
    }
}

impl Error for AcknowledgementError {}

/// Decode exactly once, require a nonempty string, and preserve its opaque value.
pub fn execution_id(
    body: &str,
    field: &str,
    operation: &'static str,
    expected: Option<&str>,
) -> Result<String, AcknowledgementError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|_| AcknowledgementError { operation })?;
    value
        .get(field)
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty() && expected.is_none_or(|expected| expected == *id))
        .map(str::to_owned)
        .ok_or(AcknowledgementError { operation })
}
