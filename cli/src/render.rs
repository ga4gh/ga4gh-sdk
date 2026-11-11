//! How the CLI shows values from services in text output.
//!
//! Text output, such as tables, bare IDs and error messages, shows control
//! and bidirectional formatting characters as escapes, so that a value can't
//! add lines, terminal sequences or reordered text. JSON output retains every
//! value while escaping these characters inside strings. Raw JSON response
//! layout and number spelling stay intact.

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

/// Escapes unsafe characters inside JSON strings without changing the values.
/// Whitespace, number spelling, field order, and existing escapes stay intact.
/// The caller must validate JSON before passing an untrusted response here.
pub(crate) fn json_printable(json: &str) -> String {
    let mut output = String::with_capacity(json.len());
    let (mut in_string, mut escaped) = (false, false);
    for c in json.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            } else if needs_escape(c) {
                // JSON permits these raw Unicode characters in strings, but
                // a terminal could interpret them as controls or bidi marks.
                output.push_str(&format!("\\u{:04x}", u32::from(c)));
                continue;
            }
        } else if c == '"' {
            in_string = true;
        }
        output.push(c);
    }
    output
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

    #[test]
    fn json_rendering_preserves_layout_and_existing_escapes() {
        let json = " \t{\"id\":\"a\u{202e}b\",\"literal\":\"\\\\u202e\",\"n\":1.00}\r\n";
        let rendered = json_printable(json);
        assert_eq!(
            rendered,
            " \t{\"id\":\"a\\u202eb\",\"literal\":\"\\\\u202e\",\"n\":1.00}\r\n"
        );
        let before: serde_json::Value = serde_json::from_str(json).unwrap();
        let after: serde_json::Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(before, after);
    }
}
