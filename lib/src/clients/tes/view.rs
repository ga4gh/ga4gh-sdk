use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// The `view` of a TES task:
///
/// The fields the server returns:
/// - `MINIMAL`: only `id` and `state`. This is the TES default.
/// - `BASIC`: all fields except executor `stdout` and `stderr`, input `content` and
///   `system_logs`.
/// - `FULL`: all fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum TesView {
    #[default]
    Minimal,
    Basic,
    Full,
}

impl TesView {
    /// The value TES expects in the `view` query parameter.
    pub fn as_str(&self) -> &'static str {
        match self {
            TesView::Minimal => "MINIMAL",
            TesView::Basic => "BASIC",
            TesView::Full => "FULL",
        }
    }
}

impl fmt::Display for TesView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A string that is not a TES view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTesViewError(pub String);

impl fmt::Display for ParseTesViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Invalid view '{}': expected MINIMAL, BASIC, or FULL",
            self.0
        )
    }
}

impl std::error::Error for ParseTesViewError {}

impl FromStr for TesView {
    type Err = ParseTesViewError;

    /// Parses a view name, ignoring case and surrounding whitespace.
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_uppercase().as_str() {
            "MINIMAL" => Ok(TesView::Minimal),
            "BASIC" => Ok(TesView::Basic),
            "FULL" => Ok(TesView::Full),
            _ => Err(ParseTesViewError(value.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_views_round_trip() {
        for (view, name) in [
            (TesView::Minimal, "MINIMAL"),
            (TesView::Basic, "BASIC"),
            (TesView::Full, "FULL"),
        ] {
            assert_eq!(view.to_string(), name);
            assert_eq!(serde_json::to_value(view).unwrap(), name);
            assert_eq!(name.parse::<TesView>().unwrap(), view);
            assert_eq!(
                format!(" {} ", name.to_lowercase())
                    .parse::<TesView>()
                    .unwrap(),
                view
            );
        }
        assert_eq!(TesView::default(), TesView::Minimal);
    }

    #[test]
    fn test_invalid_views_are_rejected() {
        for value in ["", "ALL", "FULL VIEW", "\"FULL\""] {
            let error = value.parse::<TesView>().unwrap_err();
            assert!(
                error.to_string().contains("MINIMAL, BASIC, or FULL"),
                "{error}"
            );
        }
    }
}
