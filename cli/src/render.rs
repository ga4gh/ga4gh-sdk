//! How the CLI shows values from services in text output.
//!
//! Text output, such as tables, bare IDs and error messages, shows control
//! and bidirectional formatting characters as escapes, so that a value can't
//! add lines, terminal sequences or reordered text. JSON output keeps every
//! value: commands that print the server's JSON print it as received, and
//! JSONL records use JSON escapes, which decode to the original value.

/// Escapes control and bidirectional formatting characters in a value for
/// text output, as Rust escapes such as `\n` or `\u{1b}`.
///
/// Other characters, including backslashes, are kept, so escaping text that
/// is already escaped doesn't change it. Downstream CLIs can use it for their
/// own text output, as the `ga4gh-cli` binary does for its log messages.
pub fn printable(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if needs_escape(c) {
                c.escape_default().collect()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// Escapes the same characters as [`printable`] in serialised JSON, as JSON
/// `\uXXXX` escapes, so that the output stays valid JSON with the same
/// values. Serde escapes C0 controls in strings already; the remaining ones
/// are all in the Basic Multilingual Plane and can only occur inside strings.
pub(crate) fn json_printable(json: &str) -> String {
    json.chars()
        .map(|c| {
            // Raw newlines are the layout of pretty JSON; serde escapes the rest.
            if c != '\n' && needs_escape(c) {
                format!("\\u{:04x}", u32::from(c))
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn needs_escape(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

#[cfg(test)]
mod tests {
    use super::{json_printable, printable};

    #[test]
    fn ordinary_values_are_unchanged() {
        for value in ["t1", "org.exämple/tes 1", r"a\nb", "", "日本"] {
            assert_eq!(printable(value), value);
        }
    }

    #[test]
    fn control_and_bidirectional_characters_are_escaped_once() {
        let escaped = printable("t1\u{1b}[2K\r\nx\u{9b}\u{202E}\u{2066}\u{61C}");
        assert_eq!(escaped, r"t1\u{1b}[2K\r\nx\u{9b}\u{202e}\u{2066}\u{61c}");
        assert_eq!(printable(&escaped), escaped);
    }

    #[test]
    fn json_escapes_decode_to_the_original_value() {
        let value = "a\u{202E}b\u{9b}c";
        let json = serde_json::to_string_pretty(&serde_json::json!({"id": value})).unwrap();
        let escaped = json_printable(&json);
        assert!(!escaped.contains(['\u{202E}', '\u{9b}']), "{escaped}");
        let decoded: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(decoded["id"], value);
    }
}
