//! Output interpretation and opt-in artifact evidence, independent of execution state.
//!
//! [`Profile::project_results`] checks each named output of a run against
//! the profile's output contract and gives a [`ResultRecord`]. HTTP(S) URLs
//! in output values, annotations and the profile document lose their
//! credentials, query and fragment, also where they are embedded in longer
//! strings; the context is stored as given. The `verify_` methods of
//! [`ResultRecord`] then add [`ArtifactEvidence`] for selected output
//! artifacts: whether they could be read, and whether their SHA-256 matches
//! an expected checksum.
use super::{digest, pointer_segment, ContractError, Diagnostic, Profile};
use crate::{
    clients::drs::uri::{AccessPlan, AccessPreferences, DrsResolver},
    utils::{operation::Operation, transport::read_bounded},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
/// Whether an output conforms to its schema in the output contract.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContractStatus {
    /// The output conforms to its schema, or is optional and absent.
    Conformant,
    /// The output is required but absent.
    Missing,
    /// The output doesn't conform to its schema.
    Invalid,
}
/// Whether an output artifact could be read.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AccessStatus {
    /// The output has a value, but its bytes weren't read.
    NotChecked,
    /// The bytes were read.
    Available,
    /// Resolving or reading the artifact failed or was interrupted.
    Inaccessible,
    /// The result has no value at the pointer.
    Missing,
}
/// Whether an output artifact's bytes match the expected checksum.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityStatus {
    /// No bytes were read, or no expected checksum was given.
    Unverified,
    /// The SHA-256 of the bytes matches the expected checksum.
    Verified,
    /// The SHA-256 of the bytes differs from the expected checksum.
    Mismatch,
}
/// A checksum that an artifact is expected to have, and where it comes
/// from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExpectedChecksum {
    /// The checksum algorithm; only SHA-256 (`sha256` or `SHA-256`, in any
    /// case) is supported.
    pub algorithm: String,
    /// The expected checksum, as 64 hex digits.
    pub digest: String,
    /// Where the expected checksum comes from, such as a DRS object's
    /// checksums; not blank.
    pub evidence_source: String,
}
/// What a check found out about one output artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactEvidence {
    /// The JSON pointer of the artifact in the outputs; its first segment
    /// names the output.
    pub pointer: String,
    /// The output value, if it is a `drs://` URI.
    pub stable_identity: Option<String>,
    /// Whether the artifact could be read.
    pub access_status: AccessStatus,
    /// Whether its bytes match `expected`.
    pub integrity_status: IntegrityStatus,
    /// The checksum that the artifact is expected to have.
    pub expected: Option<ExpectedChecksum>,
    /// The SHA-256 of the bytes that were read, as 64 lowercase hex digits.
    pub computed_sha256: Option<String>,
    /// Why the artifact is inaccessible. Holds no URL, header or provider
    /// detail.
    pub detail: Option<String>,
}
/// One named output of a run, checked against its schema.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TypedOutput {
    /// Whether the stored value differs from the run's: true if
    /// credentials, a query or a fragment were removed, but also if an
    /// HTTP(S) URL was only normalized, such as `https://example.com`
    /// to `https://example.com/`.
    #[serde(default)]
    pub access_details_redacted: bool,
    /// The output value. Strings that are whole HTTP(S) URLs are
    /// normalized and lose their credentials, query and fragment; HTTP(S)
    /// URLs embedded in longer strings lose them too. `None` if the run
    /// didn't give it.
    pub value: Option<Value>,
    /// Whether the value conforms to the output's schema.
    pub contract_status: ContractStatus,
    /// The output's schema from the profile, with the URL redaction of
    /// [`ResultRecord::contract_document`]; URLs aren't normalized
    /// otherwise.
    pub annotations: Value,
    /// The errors of validating the value against the output's schema.
    pub diagnostics: Vec<Diagnostic>,
}
/// The outputs of one run, interpreted under a profile's output contract,
/// with optional evidence about artifacts.
///
/// It is meant to be stored: it doesn't depend on the run still existing.
/// HTTP(S) URLs in output values, annotations and `contract_document` lose
/// their credentials, query and fragment, also where they are embedded in
/// longer strings. Other URL schemes, object keys and `context` are stored
/// as given, so they must hold no credentials.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResultRecord {
    /// The format version; 1.
    pub format_version: u32,
    /// The caller's reference to the run, such as its WES run ID.
    pub run_reference: String,
    /// The run's state, as the caller passed it; not interpreted.
    pub execution_state: String,
    /// The [`Profile::schema_digest`] of the profile.
    pub schema_digest: String,
    /// The profile's JSON document, which the schema references and
    /// annotations of the outputs resolve against. HTTP(S) URLs in its
    /// strings lose their credentials, query and fragment, by their text;
    /// other text stays as it is. The values of `$ref`, `$dynamicRef`, `$id`
    /// and `$schema` keep a fragment that is a JSON pointer or an anchor, so
    /// that references still resolve, and `pattern` values stay as they
    /// are. [`Self::schema_digest`] is computed over the profile as given.
    #[serde(default)]
    pub contract_document: Value,
    /// The errors of validating all outputs against the output contract.
    pub contract_diagnostics: Vec<Diagnostic>,
    /// The outputs that the contract names, by name.
    pub outputs: BTreeMap<String, TypedOutput>,
    /// Evidence about output artifacts, by JSON pointer; filled by the
    /// `verify_` methods.
    pub artifacts: BTreeMap<String, ArtifactEvidence>,
    /// Context of the run, such as engine or reference versions, that the
    /// caller adds; [`ComparisonPolicy`](super::compare::ComparisonPolicy)
    /// compares it.
    pub context: BTreeMap<String, String>,
}
// Parses `text` as an HTTP(S) URL and removes its credentials, query and
// fragment. `None` if it isn't an HTTP(S) URL.
fn strip_url(text: &str) -> Option<url::Url> {
    let mut u = url::Url::parse(text).ok()?;
    if !["http", "https"].contains(&u.scheme()) {
        return None;
    }
    let _ = u.set_username("");
    let _ = u.set_password(None);
    u.set_query(None);
    u.set_fragment(None);
    Some(u)
}
// Whether `c` ends a URL in text: whitespace, or a character that a URL
// can't hold unencoded.
fn url_end(c: char) -> bool {
    c.is_whitespace() || "\"<>`".contains(c)
}
// The length of the scheme at the start of `text`: `http:` or `https:`, in
// any case, followed by one or more `/` or `\`, which URL parsers also
// accept.
fn scheme_len(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let name = if bytes.len() >= 6 && bytes[..6].eq_ignore_ascii_case(b"https:") {
        6
    } else if bytes.len() >= 5 && bytes[..5].eq_ignore_ascii_case(b"http:") {
        5
    } else {
        return None;
    };
    let slashes = bytes[name..]
        .iter()
        .take_while(|b| matches!(b, b'/' | b'\\'))
        .count();
    (slashes > 0).then_some(name + slashes)
}
// Whether a fragment is empty, a JSON pointer or a plain-name anchor, which
// references use, rather than something like `access_token=...`.
fn reference_fragment(fragment: &str) -> bool {
    fragment.is_empty()
        || fragment.starts_with('/')
        || (fragment.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
            && fragment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "-._".contains(c)))
}
// Removes credentials, queries and fragments from the HTTP(S) URLs in
// `text`, by their text, without parsing them, so that URLs which don't
// parse, URLs inside other URLs and URLs in longer strings are covered too.
//
// A URL starts at its scheme (see `scheme_len`). Its credentials are
// everything up to the last `@` before the next `/`, `\`, `?`, `#` or
// `url_end` character. From the first `?` or `#` after the scheme on,
// everything up to the next `url_end` character goes, except a trailing
// run of `.,;:)]}`, and, if `keep_fragment`, a `reference_fragment`. Other
// text stays exactly as it is. Runs in linear time.
fn redact_text(text: &str, keep_fragment: bool) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_url = false;
    let mut i = 0;
    while let Some(c) = text[i..].chars().next() {
        let rest = &text[i..];
        if let Some(scheme) = scheme_len(rest) {
            let after = &rest[scheme..];
            let end = after
                .find(|c: char| url_end(c) || "/\\?#".contains(c))
                .unwrap_or(after.len());
            let authority = &after[..end];
            out.push_str(&rest[..scheme]);
            out.push_str(
                authority
                    .rfind('@')
                    .map_or(authority, |at| &authority[at + 1..]),
            );
            i += scheme + end;
            in_url = true;
        } else if in_url && (c == '?' || c == '#') {
            let tail = &rest[..rest.find(url_end).unwrap_or(rest.len())];
            let core = tail.trim_end_matches(|c: char| ".,;:)]}".contains(c));
            if keep_fragment {
                if let Some(fragment) = core.find('#').map(|at| &core[at + 1..]) {
                    if reference_fragment(fragment) {
                        out.push('#');
                        out.push_str(fragment);
                    }
                }
            }
            out.push_str(&tail[core.len()..]);
            i += tail.len();
            in_url = false;
        } else {
            in_url &= !url_end(c);
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}
// Removes credentials, query and fragment from HTTP(S) URLs in an output
// value, so that it can be stored. A string that is a whole HTTP(S) URL is
// also normalized.
fn durable(value: &Value) -> Value {
    match value {
        Value::String(s) => match strip_url(s) {
            Some(u) => json!(redact_text(u.as_str(), false)),
            None => json!(redact_text(s, false)),
        },
        Value::Array(a) => Value::Array(a.iter().map(durable).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), durable(v))).collect()),
        _ => value.clone(),
    }
}
// JSON Schema keywords whose URL fragment can be a JSON pointer or an anchor
// that references depend on.
const REFERENCE_KEYWORDS: [&str; 4] = ["$ref", "$dynamicRef", "$id", "$schema"];
// Removes credentials, query and fragment from the HTTP(S) URLs in a
// profile document or one of its schemas, as `redact_text` does. The
// values of `REFERENCE_KEYWORDS` keep a fragment that is a JSON pointer or
// an anchor, so that references still resolve, and `pattern` values, which
// are regular expressions, stay as they are. Object keys stay as they are.
fn durable_schema(value: &Value, keyword: Option<&str>) -> Value {
    match value {
        Value::String(_) if keyword == Some("pattern") => value.clone(),
        Value::String(s) => json!(redact_text(
            s,
            keyword.is_some_and(|k| REFERENCE_KEYWORDS.contains(&k))
        )),
        Value::Array(a) => Value::Array(a.iter().map(|v| durable_schema(v, None)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), durable_schema(v, Some(k))))
                .collect(),
        ),
        _ => value.clone(),
    }
}
impl Profile {
    /// Checks a run's outputs against the output contract and returns them
    /// as a [`ResultRecord`], with empty `artifacts` and `context`.
    ///
    /// Each output named in the contract's `properties` gets a
    /// [`TypedOutput`]. Other outputs aren't kept; they give a diagnostic
    /// only if the output schema disallows them. Reads no artifact.
    ///
    /// # Errors
    ///
    /// Fails if the profile has no output contract, if the contract has no
    /// named `properties`, or if an output's schema can't be compiled.
    pub fn project_results(
        &self,
        run_reference: &str,
        execution_state: &str,
        outputs: &Value,
    ) -> Result<ResultRecord, ContractError> {
        let all = self.validate_outputs(outputs)?;
        let contract = self
            .document()
            .pointer("/properties/outputs")
            .ok_or("no output contract")?;
        let properties = contract
            .get("properties")
            .and_then(Value::as_object)
            .ok_or("output contract needs named properties")?;
        let required = contract.get("required").and_then(Value::as_array);
        let mut result = ResultRecord {
            format_version: 1,
            run_reference: run_reference.into(),
            execution_state: execution_state.into(),
            schema_digest: self.schema_digest.clone(),
            contract_document: durable_schema(self.document(), None),
            contract_diagnostics: all,
            outputs: BTreeMap::new(),
            artifacts: BTreeMap::new(),
            context: BTreeMap::new(),
        };
        for (key, annotation) in properties {
            let value = outputs.get(key);
            let is_required = required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some(key)));
            let errors = if let Some(value) = value {
                self.validate_at(
                    &format!("/properties/outputs/properties/{}", pointer_segment(key)),
                    value,
                )?
            } else {
                vec![]
            };
            let status = if value.is_none() && is_required {
                ContractStatus::Missing
            } else if !errors.is_empty() {
                ContractStatus::Invalid
            } else {
                ContractStatus::Conformant
            };
            let stored = value.map(durable);
            result.outputs.insert(
                key.clone(),
                TypedOutput {
                    access_details_redacted: stored.as_ref() != value,
                    value: stored,
                    contract_status: status,
                    annotations: durable_schema(annotation, None),
                    diagnostics: errors,
                },
            );
        }
        Ok(result)
    }
}
impl ResultRecord {
    /// Returns the value at a JSON pointer into the outputs, whose first
    /// segment names an output; `None` if there is none.
    pub fn value_at(&self, pointer: &str) -> Option<&Value> {
        let path = pointer.strip_prefix('/')?;
        let (key, rest) = path
            .split_once('/')
            .map(|(a, b)| (a, Some(b)))
            .unwrap_or((path, None));
        let key = key.replace("~1", "/").replace("~0", "~");
        let value = self.outputs.get(&key)?.value.as_ref()?;
        match rest {
            Some(rest) => value.pointer(&format!("/{rest}")),
            None => Some(value),
        }
    }
    fn evidence(&self, pointer: &str, expected: Option<ExpectedChecksum>) -> ArtifactEvidence {
        let value = self.value_at(pointer);
        ArtifactEvidence {
            pointer: pointer.into(),
            stable_identity: value
                .and_then(Value::as_str)
                .filter(|s| s.starts_with("drs://"))
                .map(str::to_owned),
            access_status: if value.is_some() {
                AccessStatus::NotChecked
            } else {
                AccessStatus::Missing
            },
            integrity_status: IntegrityStatus::Unverified,
            expected,
            computed_sha256: None,
            detail: None,
        }
    }
    /// Records evidence for bytes that the caller read for the artifact at
    /// `pointer`, under that pointer in `artifacts`.
    ///
    /// The SHA-256 of `bytes` is compared with `expected`; without it, the
    /// integrity stays [`IntegrityStatus::Unverified`]. If the result has
    /// no value at `pointer`, the access is [`AccessStatus::Missing`],
    /// `bytes` are ignored, and `expected` is recorded as given, without
    /// checks.
    ///
    /// # Errors
    ///
    /// Fails, and records nothing, if there is a value at `pointer` and
    /// `expected` names an algorithm other than SHA-256, its digest isn't
    /// 64 hex digits, or its evidence source is blank.
    pub fn verify_bytes(
        &mut self,
        pointer: &str,
        bytes: &[u8],
        expected: Option<ExpectedChecksum>,
    ) -> Result<(), ContractError> {
        let mut evidence = self.evidence(pointer, expected);
        if self.value_at(pointer).is_none() {
            self.artifacts.insert(pointer.into(), evidence);
            return Ok(());
        }
        evidence.access_status = AccessStatus::Available;
        evidence.computed_sha256 = Some(digest(bytes));
        if let Some(expected) = &evidence.expected {
            if expected.algorithm.to_ascii_lowercase().replace('-', "") != "sha256" {
                return Err("unsupported checksum algorithm".into());
            }
            if expected.evidence_source.trim().is_empty()
                || expected.digest.len() != 64
                || !expected.digest.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("expected SHA-256 needs a valid digest and evidence source".into());
            }
            evidence.integrity_status = if expected
                .digest
                .eq_ignore_ascii_case(evidence.computed_sha256.as_ref().unwrap())
            {
                IntegrityStatus::Verified
            } else {
                IntegrityStatus::Mismatch
            };
        }
        self.artifacts.insert(pointer.into(), evidence);
        Ok(())
    }
    /// Reads the artifact at `pointer` with a DRS access plan and records
    /// its evidence, as [`Self::verify_bytes`] does.
    ///
    /// The plan's URL and headers are used once and never stored. The
    /// request uses a fresh client: it follows no redirects and sends no
    /// SDK credentials. At most `max_bytes` are read, within the bounds of
    /// `operation`. If the request fails, returns no success status or
    /// goes over `max_bytes`, the access is [`AccessStatus::Inaccessible`]
    /// and `expected` is recorded as given, without checks.
    ///
    /// # Errors
    ///
    /// Fails if the HTTP client can't be built, or, once the bytes are
    /// read, as [`Self::verify_bytes`] does.
    pub async fn verify_access_plan(
        &mut self,
        pointer: &str,
        plan: &AccessPlan,
        expected: Option<ExpectedChecksum>,
        operation: &Operation,
        max_bytes: usize,
    ) -> Result<(), ContractError> {
        if self.value_at(pointer).is_none() {
            let evidence = self.evidence(pointer, expected);
            self.artifacts.insert(pointer.into(), evidence);
            return Ok(());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let result = operation
            .run(async {
                let response = client
                    .get(plan.url.clone())
                    .headers(plan.headers.clone())
                    .send()
                    .await?;
                if !response.status().is_success() {
                    return Err::<Vec<u8>, ContractError>(
                        format!("artifact HTTP {}", response.status()).into(),
                    );
                }
                Ok(read_bounded(response, max_bytes).await?.body)
            })
            .await;
        match result {
            Ok(Ok(bytes)) => self.verify_bytes(pointer, &bytes, expected),
            _ => {
                let mut evidence = self.evidence(pointer, expected);
                evidence.access_status = AccessStatus::Inaccessible;
                evidence.detail=Some("selected artifact access failed or was interrupted; resolve again through the owning provider".into());
                self.artifacts.insert(pointer.into(), evidence);
                Ok(())
            }
        }
    }
    /// Resolves the DRS URI at `pointer` with `resolver` and reads the
    /// artifact, as [`Self::verify_access_plan`] does.
    ///
    /// Resolution takes at most `max_attempts` attempts, as
    /// [`DrsResolver::access_plan`] does. If it fails, the access is
    /// [`AccessStatus::Inaccessible`] and `expected` is recorded as given,
    /// without checks.
    ///
    /// # Errors
    ///
    /// Fails if the value at `pointer` is missing or isn't a string, or as
    /// [`Self::verify_access_plan`] does.
    #[allow(clippy::too_many_arguments)] // Explicit resolver, evidence and operation bounds stay caller-owned.
    pub async fn verify_drs(
        &mut self,
        pointer: &str,
        resolver: &DrsResolver,
        preferences: &AccessPreferences,
        expected: Option<ExpectedChecksum>,
        operation: &Operation,
        max_attempts: usize,
        max_bytes: usize,
    ) -> Result<(), ContractError> {
        let uri = self
            .value_at(pointer)
            .and_then(Value::as_str)
            .ok_or("selected output is not an artifact URI")?
            .to_owned();
        match resolver
            .access_plan(&uri, preferences, operation, max_attempts)
            .await
        {
            Ok(plan) => {
                self.verify_access_plan(pointer, &plan, expected, operation, max_bytes)
                    .await
            }
            Err(_) => {
                let mut evidence = self.evidence(pointer, expected);
                evidence.access_status = AccessStatus::Inaccessible;
                evidence.detail = Some("DRS resolution unavailable or interrupted".into());
                self.artifacts.insert(pointer.into(), evidence);
                Ok(())
            }
        }
    }
    /// Returns the outputs that the profile marks for an HTML viewer
    /// (`x-ga4gh-ui-widget: html-viewer`), with their key, media type,
    /// value and contract status. Consumers decide authorization and
    /// presentation.
    pub fn html_viewer_metadata(&self) -> Vec<Value> {
        self.outputs.iter().filter(|(_,v)|v.annotations.get("x-ga4gh-ui-widget").and_then(Value::as_str)==Some("html-viewer")).map(|(key,v)|json!({"key":key,"widget":"html-viewer","contentMediaType":v.annotations.get("contentMediaType"),"value":v.value,"contract_status":v.contract_status})).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redact_text_removes_access_details_by_text() {
        for (text, redacted) in [
            (
                "x https://u:p@h.example/a?q=1#f) y",
                "x https://h.example/a) y",
            ),
            ("HTTPS://h.example/?token=1", "HTTPS://h.example/"),
            (
                "<http://h.example/a#f>, http://h.example",
                "<http://h.example/a>, http://h.example",
            ),
            // URLs that don't parse
            (
                "https://u:p@h.example:99999/a?token=1#f",
                "https://h.example:99999/a",
            ),
            // Byte offsets stay on character boundaries around non-ASCII text
            (
                "é https://u:p@h.example/ü?t=ß#f ñ",
                "é https://h.example/ü ñ",
            ),
            // Several URLs, and a bare scheme at the end
            (
                "https://h.example/a?t=1 and https://h.example/b#f, see https://",
                "https://h.example/a and https://h.example/b, see https://",
            ),
            // A URL inside another URL's path
            (
                "mirrors: https://h.example/a,https://user:pw@h2.example/b",
                "mirrors: https://h.example/a,https://h2.example/b",
            ),
            (
                "https://h.example/https://user:pw@h2.example/b?",
                "https://h.example/https://h2.example/b",
            ),
            // Characters that are legal in credentials but end words
            (
                "x https://user:ab'cd@h.example/ y",
                "x https://h.example/ y",
            ),
            ("https://h.example/?sig=ab'cd!", "https://h.example/"),
            // Other spellings that URL parsers accept, and JSON escapes
            (r"http:\\user:pw@h.example/", r"http:\\h.example/"),
            ("https:/user:pw@h.example/", "https:/h.example/"),
            (r"https:\/\/user:pw@h.example\/b", r"https:\/\/h.example\/b"),
            (
                r#"{"url":"https://u:p@h.example/x?t=1","n":2}"#,
                r#"{"url":"https://h.example/x","n":2}"#,
            ),
            // Whitespace ends a URL, so a password with a space is cut there
            (
                "https://u:pass word@h.example/",
                "https://u:pass word@h.example/",
            ),
        ] {
            assert_eq!(redact_text(text, false), redacted, "{text}");
        }
        for unchanged in [
            "see https://h.example",
            "s3://bucket/key?sig=1",
            "http:// nothing",
            "what? who# no URL at all",
            "https",
        ] {
            assert_eq!(redact_text(unchanged, false), unchanged);
        }
    }
    #[test]
    fn redact_text_keeps_only_reference_fragments_if_asked() {
        for (text, redacted) in [
            ("https://h.example/a#/defs/x", "https://h.example/a#/defs/x"),
            ("https://h.example/a#anchor", "https://h.example/a#anchor"),
            ("https://h.example/a#", "https://h.example/a#"),
            (
                "https://u:p@h.example:99999/a?t=1#/defs/x",
                "https://h.example:99999/a#/defs/x",
            ),
            ("https://h.example/s#access_token=t", "https://h.example/s"),
        ] {
            assert_eq!(redact_text(text, true), redacted, "{text}");
        }
    }
    #[test]
    fn redact_text_is_linear() {
        let text = "http://a ".repeat(200_000) + &"x".repeat(200_000);
        assert_eq!(redact_text(&text, false), text);
    }
    #[test]
    fn durable_schema_keeps_reference_fragments_and_patterns() {
        let schema = json!({
            "$id": "https://h.example/s.json#",
            "$ref": "https://u:p@h.example/s.json?token=1#/$defs/a",
            "description": "https://h.example/s.json#frag",
            "enum": ["https://h.example/e?token=1"],
            "pattern": "^https://[^?#]+$",
            "properties": {"$ref": {"type": "string", "default": "https://h.example/d?token=1"}}
        });
        assert_eq!(
            durable_schema(&schema, None),
            json!({
                "$id": "https://h.example/s.json#",
                "$ref": "https://h.example/s.json#/$defs/a",
                "description": "https://h.example/s.json",
                "enum": ["https://h.example/e"],
                "pattern": "^https://[^?#]+$",
                "properties": {"$ref": {"type": "string", "default": "https://h.example/d"}}
            })
        );
    }
    #[test]
    fn durable_normalizes_whole_urls_and_redacts_nested_ones() {
        assert_eq!(
            durable(&json!("HTTPS://h.example?t=1")),
            json!("https://h.example/")
        );
        assert_eq!(
            durable(&json!("https://h.example/a,https://user:pw@h2.example/b")),
            json!("https://h.example/a,https://h2.example/b")
        );
    }
}
