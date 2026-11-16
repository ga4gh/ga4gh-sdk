//! Bounded comparisons of existing evidence under an explicit versioned policy.
//!
//! A [`ComparisonPolicy`] names the outputs to compare, with a
//! [`Comparator`] for each, and the context keys that must match.
//! [`ComparisonPolicy::compare`] compares two [`ResultRecord`]s under it
//! and gives a [`ComparisonReport`]. It reads no artifact; it compares only
//! the values and the evidence that the records hold.
use super::{
    results::{AccessStatus, ArtifactEvidence, ContractStatus, IntegrityStatus, ResultRecord},
    ContractError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
/// The outcome of a comparison.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// The values are the same under the policy.
    EquivalentUnderPolicy,
    /// The values differ under the policy.
    Different,
    /// The evidence isn't enough to decide.
    Inconclusive,
}
/// How to compare one output of two results.
///
/// For both comparators, the output that the pointer starts in must be
/// [`ContractStatus::Conformant`] in both results; otherwise the outcome is
/// [`Outcome::Inconclusive`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "comparator", rename_all = "snake_case")]
pub enum Comparator {
    /// Compares the SHA-256 of an artifact in both results.
    ///
    /// Both results need [`ArtifactEvidence`] at the pointer that is
    /// [`AccessStatus::Available`] and [`IntegrityStatus::Verified`] against
    /// an expected SHA-256; otherwise the outcome is inconclusive.
    Sha256 {
        /// The comparator version; must be 1.
        version: u32,
    },
    /// Compares the JSON values at the pointer, numbers within tolerances.
    ///
    /// Numbers are compared exactly as decimals. Objects must have the same
    /// keys, and arrays the same length. A `null` on either side, or a
    /// value larger than the bounds, makes the outcome inconclusive.
    JsonMetrics {
        /// The comparator version; must be 1.
        version: u32,
        /// Numbers are equal if they differ by at most this; finite and not
        /// negative.
        absolute_tolerance: f64,
        /// Numbers are equal if they differ by at most this share of the
        /// larger magnitude; finite and not negative.
        relative_tolerance: f64,
        /// JSON pointers, relative to the compared value, to skip.
        ignored_pointers: Vec<String>,
        /// Whether to sort arrays before comparing them element by element.
        /// Requires both tolerances to be 0.
        unordered_arrays: bool,
        /// The most JSON nodes to compare, from 1 to 1,000,000. Values
        /// nested deeper than 128 levels or with more than 8 MiB of strings
        /// and keys are also too large.
        max_nodes: usize,
    },
}
/// A versioned policy for comparing two results.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ComparisonPolicy {
    /// The policy's identifier; not blank.
    pub id: String,
    /// The policy version; must be 1.
    pub version: u32,
    /// The comparator for each output, by JSON pointer into the outputs;
    /// not empty.
    pub outputs: BTreeMap<String, Comparator>,
    /// The context keys that must be set, not empty and equal in both
    /// results. `schema_digest` is part of the context.
    pub required_context: Vec<String>,
}
/// The result of comparing two results under a policy.
#[derive(Debug, Serialize, Deserialize)]
pub struct ComparisonReport {
    /// The policy that was applied.
    pub policy: ComparisonPolicy,
    /// The run reference of the left result.
    pub left: String,
    /// The run reference of the right result.
    pub right: String,
    /// The overall outcome. It is inconclusive if required context is
    /// missing or differs, or if any output is inconclusive; otherwise
    /// different if any output differs.
    pub outcome: Outcome,
    /// The outcome for each output, by JSON pointer.
    pub outputs: BTreeMap<String, Outcome>,
    /// Every context key, including `schema_digest`, whose values differ,
    /// with the left and right value.
    pub context_drift: BTreeMap<String, (Option<String>, Option<String>)>,
    /// The required context keys that are missing or empty in either
    /// result.
    pub missing_context: Vec<String>,
}
fn verified_digest(evidence: &ArtifactEvidence) -> Option<String> {
    let computed = evidence.computed_sha256.as_ref()?;
    let expected = evidence.expected.as_ref()?;
    if evidence.integrity_status != IntegrityStatus::Verified
        || evidence.access_status != AccessStatus::Available
        || computed.len() != 64
        || !computed.bytes().all(|b| b.is_ascii_hexdigit())
        || !expected.digest.eq_ignore_ascii_case(computed)
        || expected.evidence_source.trim().is_empty()
        || expected.algorithm.to_ascii_lowercase().replace('-', "") != "sha256"
    {
        return None;
    }
    Some(computed.to_ascii_lowercase())
}
struct MetricRules<'a> {
    absolute: f64,
    relative: f64,
    ignored: &'a [String],
    unordered: bool,
}
// Compare JSON decimal numbers as rationals, retaining integers above 2^53.
fn decimal(text: &str) -> Option<num_rational::BigRational> {
    use num_bigint::BigInt;
    let (mantissa, exponent) = text
        .split_once(['e', 'E'])
        .map(|(a, b)| Some((a, b.parse::<i32>().ok()?)))
        .unwrap_or(Some((text, 0)))?;
    let decimals = mantissa.split_once('.').map_or(0, |(_, r)| r.len() as i32);
    let integer: BigInt = mantissa.replace('.', "").parse().ok()?;
    let power = exponent - decimals;
    let factor = BigInt::from(10).pow(power.unsigned_abs());
    Some(if power >= 0 {
        num_rational::BigRational::from_integer(integer * factor)
    } else {
        num_rational::BigRational::new(integer, factor)
    })
}
fn numeric(
    a: &serde_json::Number,
    b: &serde_json::Number,
    absolute: f64,
    relative: f64,
) -> Option<bool> {
    let a = decimal(&a.to_string())?;
    let b = decimal(&b.to_string())?;
    let zero = decimal("0")?;
    let delta = if a >= b { &a - &b } else { &b - &a };
    let aa = if a < zero { -a } else { a };
    let bb = if b < zero { -b } else { b };
    let scale = aa.max(bb);
    Some(
        delta <= decimal(&absolute.to_string())?
            || delta <= decimal(&relative.to_string())? * scale,
    )
}
fn bounded_shape(value: &Value, max_nodes: usize) -> bool {
    let mut stack = vec![(value, 0usize)];
    let mut nodes = 0;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > max_nodes || depth > 128 {
            return false;
        }
        match value {
            Value::String(s) => bytes = bytes.saturating_add(s.len()),
            Value::Array(a) => {
                if a.len() > max_nodes - nodes {
                    return false;
                }
                stack.extend(a.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(o) => {
                if o.len() > max_nodes - nodes {
                    return false;
                }
                for (key, v) in o {
                    bytes = bytes.saturating_add(key.len());
                    stack.push((v, depth + 1));
                }
            }
            _ => (),
        }
        if bytes > 8 * 1024 * 1024 {
            return false;
        }
    }
    true
}
fn canonical(value: &Value) -> Option<String> {
    Some(match value {
        Value::Number(n) => format!("N({})", decimal(&n.to_string())?),
        Value::String(_) => format!("S{}", serde_json::to_string(value).ok()?),
        Value::Bool(b) => format!("B{b}"),
        Value::Null => "Z".into(),
        Value::Array(a) => format!(
            "A[{}]",
            a.iter()
                .map(canonical)
                .collect::<Option<Vec<_>>>()?
                .join(",")
        ),
        Value::Object(o) => {
            let mut keys = o.keys().collect::<Vec<_>>();
            keys.sort();
            let mut members = vec![];
            for key in keys {
                members.push(format!(
                    "{}:{}",
                    serde_json::to_string(key).ok()?,
                    canonical(&o[key])?
                ));
            }
            format!("O{{{}}}", members.join(","))
        }
    })
}
fn metrics(
    a: &Value,
    b: &Value,
    pointer: &str,
    rules: &MetricRules<'_>,
    budget: &mut usize,
) -> Option<bool> {
    let MetricRules {
        absolute,
        relative,
        ignored,
        unordered,
    } = rules;
    if *budget == 0 || pointer.len() > 2048 {
        return None;
    }
    *budget -= 1;
    if ignored.iter().any(|p| p == pointer) {
        return Some(true);
    }
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => numeric(a, b, *absolute, *relative),
        (Value::Object(a), Value::Object(b)) => {
            let mut result = true;
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let next = format!("{pointer}/{}", super::pointer_segment(key));
                if ignored.contains(&next) {
                    continue;
                }
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => result &= metrics(a, b, &next, rules, budget)?,
                    _ => return Some(false),
                }
            }
            Some(result)
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(false);
            }
            let mut a = a.iter().collect::<Vec<_>>();
            let mut b = b.iter().collect::<Vec<_>>();
            if *unordered {
                a.sort_by_cached_key(|v| canonical(v));
                b.sort_by_cached_key(|v| canonical(v));
            }
            let mut result = true;
            for (index, (a, b)) in a.iter().zip(&b).enumerate() {
                result &= metrics(a, b, &format!("{pointer}/{index}"), rules, budget)?;
            }
            Some(result)
        }
        (Value::Null, _) | (_, Value::Null) => None,
        _ => Some(a == b),
    }
}
impl ComparisonPolicy {
    /// Compares two results under the policy.
    ///
    /// # Errors
    ///
    /// Fails if the policy is invalid: a blank `id`, a version other than
    /// 1, no outputs, an output key that isn't a JSON pointer, or a
    /// comparator with a version other than 1, invalid tolerances or
    /// bounds, or an ignored pointer that isn't a JSON pointer.
    pub fn compare(
        &self,
        left: &ResultRecord,
        right: &ResultRecord,
    ) -> Result<ComparisonReport, ContractError> {
        if self.id.trim().is_empty() || self.version != 1 || self.outputs.is_empty() {
            return Err("comparison requires id, version 1 and named outputs".into());
        }
        let mut outputs = BTreeMap::new();
        for (pointer, comparator) in &self.outputs {
            if !pointer.starts_with('/') {
                return Err("comparison output keys must be JSON pointers".into());
            }
            let top = pointer
                .trim_start_matches('/')
                .split('/')
                .next()
                .unwrap_or("")
                .replace("~1", "/")
                .replace("~0", "~");
            let conformant = [left, right].iter().all(|r| {
                r.outputs
                    .get(&top)
                    .is_some_and(|o| o.contract_status == ContractStatus::Conformant)
            });
            let outcome = match comparator {
                Comparator::Sha256 { version } => {
                    if *version != 1 {
                        return Err("unsupported digest comparator version".into());
                    }
                    match (left.artifacts.get(pointer), right.artifacts.get(pointer)) {
                        (Some(a), Some(b))
                            if conformant
                                && verified_digest(a).is_some()
                                && verified_digest(b).is_some() =>
                        {
                            if verified_digest(a) == verified_digest(b) {
                                Outcome::EquivalentUnderPolicy
                            } else {
                                Outcome::Different
                            }
                        }
                        _ => Outcome::Inconclusive,
                    }
                }
                Comparator::JsonMetrics {
                    version,
                    absolute_tolerance,
                    relative_tolerance,
                    ignored_pointers,
                    unordered_arrays,
                    max_nodes,
                } => {
                    if *version != 1
                        || !absolute_tolerance.is_finite()
                        || !relative_tolerance.is_finite()
                        || *absolute_tolerance < 0.0
                        || *relative_tolerance < 0.0
                        || (*unordered_arrays
                            && (*absolute_tolerance != 0.0 || *relative_tolerance != 0.0))
                        || *max_nodes == 0
                        || *max_nodes > 1_000_000
                        || !pointer.starts_with('/')
                        || ignored_pointers.iter().any(|p| !p.starts_with('/'))
                    {
                        return Err(
                            "invalid metric comparator version, bounds, tolerance or pointer"
                                .into(),
                        );
                    }
                    match (left.value_at(pointer), right.value_at(pointer)) {
                        (Some(a), Some(b))
                            if conformant
                                && bounded_shape(a, *max_nodes)
                                && bounded_shape(b, *max_nodes) =>
                        {
                            match metrics(
                                a,
                                b,
                                "",
                                &MetricRules {
                                    absolute: *absolute_tolerance,
                                    relative: *relative_tolerance,
                                    ignored: ignored_pointers,
                                    unordered: *unordered_arrays,
                                },
                                &mut max_nodes.to_owned(),
                            ) {
                                Some(true) => Outcome::EquivalentUnderPolicy,
                                Some(false) => Outcome::Different,
                                None => Outcome::Inconclusive,
                            }
                        }
                        _ => Outcome::Inconclusive,
                    }
                }
            };
            outputs.insert(pointer.clone(), outcome);
        }
        let mut lc = left.context.clone();
        let mut rc = right.context.clone();
        lc.insert("schema_digest".into(), left.schema_digest.clone());
        rc.insert("schema_digest".into(), right.schema_digest.clone());
        let missing_context = self
            .required_context
            .iter()
            .filter(|key| {
                lc.get(*key).is_none_or(String::is_empty)
                    || rc.get(*key).is_none_or(String::is_empty)
            })
            .cloned()
            .collect::<Vec<_>>();
        let keys: std::collections::BTreeSet<_> = lc.keys().chain(rc.keys()).collect();
        let context_drift = keys
            .into_iter()
            .filter(|key| lc.get(*key) != rc.get(*key))
            .map(|key| (key.clone(), (lc.get(key).cloned(), rc.get(key).cloned())))
            .collect::<BTreeMap<_, _>>();
        let identity_drift = self
            .required_context
            .iter()
            .any(|key| context_drift.contains_key(key));
        let outcome = if !missing_context.is_empty()
            || identity_drift
            || outputs.values().any(|x| *x == Outcome::Inconclusive)
        {
            Outcome::Inconclusive
        } else if outputs.values().any(|x| *x == Outcome::Different) {
            Outcome::Different
        } else {
            Outcome::EquivalentUnderPolicy
        };
        Ok(ComparisonReport {
            policy: self.clone(),
            left: left.run_reference.clone(),
            right: right.run_reference.clone(),
            outcome,
            outputs,
            context_drift,
            missing_context,
        })
    }
}
