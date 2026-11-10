//! Explicit exact reuse of trusted deterministic results. This module never submits a job.
//!
//! Receipt hashes authenticate nothing by themselves. A caller must obtain the pinned hash
//! and original-service identity from an independently trusted, authorized source and supply
//! a fresh access reader for the current principal. CrateGen remains the provenance converter.
use super::{
    digest,
    results::{ContractStatus, ExpectedChecksum, ResultRecord},
    ContractError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const REUSE_VERSION: u32 = 1;

/// Declared behavior, not a sandbox. Only `None` is eligible in this exact-reuse policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyUse {
    None,
    Dependent,
    Undeclared,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeterminismContract {
    pub opted_in: bool,
    pub author: String,
    pub revision: String,
    pub network: DependencyUse,
    pub secrets: DependencyUse,
    pub other_nondeterminism: bool,
}

/// Typed values do not equate strings, integers, decimals or booleans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TypedValue {
    String(String),
    Integer(i64),
    Unsigned(u64),
    Decimal(String),
    Boolean(bool),
    Json(Value),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentIdentity {
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceIdentity {
    pub version: String,
    pub sha256: String,
}

/// All execution-relevant fields must be declared by the recipe author. Provider IDs,
/// temporary access URLs and timestamps are deliberately outside the fingerprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeIdentity {
    pub version: u32,
    pub determinism: DeterminismContract,
    /// SHA-256 of the actual executable/script bytes, not a mutable URL or tag.
    pub executable_sha256: String,
    /// Immutable OCI manifest identity, e.g. `python@sha256:<64 lowercase hex>`.
    pub container_image: String,
    pub command: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub configuration: Value,
    pub parameters: BTreeMap<String, TypedValue>,
    /// Keys are portable relative logical paths, not provider staging paths.
    pub inputs: BTreeMap<String, ContentIdentity>,
    /// Keys are reference names; values are immutable version/content identities.
    pub reference_versions: BTreeMap<String, ReferenceIdentity>,
    pub output_schema_sha256: String,
    /// Selected artifact JSON pointer -> required relative output path.
    pub output_paths: BTreeMap<String, String>,
    /// Exact compatibility requirements, including `os`, `architecture`, and `runtime`.
    pub runtime_compatibility: BTreeMap<String, String>,
    /// Optional existing phase-two prepared invocation identity, when applicable.
    pub invocation_fingerprint: Option<String>,
}

fn sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains(['\\', '\0', ':'])
        && value
            .split('/')
            .all(|s| !s.is_empty() && s != "." && s != "..")
}
fn stable_reference(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|u| {
        matches!(u.scheme(), "drs" | "https" | "http" | "file" | "urn")
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
    })
}
// Explicit recursive sorting also remains stable if a consumer enables serde_json's
// preserve_order feature through Cargo feature unification.
fn canonical(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let sorted: BTreeMap<_, _> = map.into_iter().collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k, canonical(v))).collect())
        }
        Value::Array(values) => Value::Array(values.into_iter().map(canonical).collect()),
        value => value,
    }
}
fn content_digest(value: &impl Serialize) -> Result<String, ContractError> {
    Ok(digest(&serde_json::to_vec(&canonical(
        serde_json::to_value(value)?,
    ))?))
}
impl RecipeIdentity {
    pub fn fingerprint(&self) -> Result<String, ContractError> {
        content_digest(self)
    }
    fn invalid(&self) -> bool {
        self.version != REUSE_VERSION
            || !sha256(&self.executable_sha256)
            || !self
                .container_image
                .split_once("@sha256:")
                .is_some_and(|(name, hash)| !name.is_empty() && sha256(hash))
            || self.command.is_empty()
            || self
                .command
                .iter()
                .any(|s| s.is_empty() || s.contains('\0'))
            || self.determinism.author.trim().is_empty()
            || self.determinism.revision.trim().is_empty()
            || !sha256(&self.output_schema_sha256)
            || self.inputs.is_empty()
            || self
                .inputs
                .iter()
                .any(|(p, c)| !relative_path(p) || !sha256(&c.sha256))
            || self.output_paths.is_empty()
            || self
                .output_paths
                .iter()
                .any(|(p, path)| !p.starts_with('/') || p.len() < 2 || !relative_path(path))
            || self.output_paths.values().collect::<BTreeSet<_>>().len() != self.output_paths.len()
            || ["os", "architecture", "runtime"].iter().any(|key| {
                self.runtime_compatibility
                    .get(*key)
                    .is_none_or(|v| v.trim().is_empty())
            })
            || self.reference_versions.iter().any(|(k, v)| {
                k.trim().is_empty() || v.version.trim().is_empty() || !sha256(&v.sha256)
            })
            || self.parameters.values().any(
                |v| matches!(v, TypedValue::Decimal(s) if s.parse::<serde_json::Number>().is_err()),
            )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOrigin {
    pub service: String,
    pub run_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub relative_path: String,
    pub stable_reference: String,
    pub size_bytes: u64,
    pub sha256: String,
}

/// Reuses the phase-two typed result record; this is an eligibility receipt, not a
/// replacement for a provenance package, global cache or artifact upload protocol.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReuseReceipt {
    pub version: u32,
    pub recipe: RecipeIdentity,
    pub recipe_fingerprint: String,
    pub original: ExecutionOrigin,
    pub provenance_source: String,
    pub result: ResultRecord,
    /// Artifact output JSON pointer -> durable manifest evidence.
    pub artifacts: BTreeMap<String, ArtifactReference>,
}

/// Trust is supplied for this action, never accepted from inside the receipt.
#[derive(Clone, Debug)]
pub enum ReceiptTrust {
    Pinned {
        receipt_sha256: String,
        original_service: String,
        evidence_source: String,
    },
    Untrusted,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReuseStatus {
    Eligible,
    Ineligible,
    Unverified,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    InvalidIdentity,
    RecipeNotOptedIn,
    NondeterministicDependency,
    RecipeChanged,
    UnsupportedReceiptVersion,
    FingerprintMismatch,
    NotComplete,
    InvalidOutputs,
    ArtifactManifestMismatch,
    TrustUnknown,
    SourceUntrusted,
    ReceiptNotTrusted,
    ArtifactAccessDenied,
    ArtifactMissing,
    ArtifactAccessUnverified,
    ArtifactTooLarge,
    ArtifactSizeMismatch,
    ArtifactDigestMismatch,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReuseReason {
    pub code: ReasonCode,
    pub detail: String,
    pub artifact_pointer: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReuseMeasurements {
    pub bytes_read: u64,
    pub bytes_revalidated: u64,
    /// Actual transport bytes reported by the access adapter; local reads are zero.
    pub bytes_transferred: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReuseReport {
    pub status: ReuseStatus,
    pub reasons: Vec<ReuseReason>,
    pub original: ExecutionOrigin,
    pub recipe_fingerprint: String,
    pub receipt_sha256: String,
    pub measurements: ReuseMeasurements,
}

/// A fresh authorized read, measured by the caller's access adapter. Do not return
/// stale cached bytes as evidence of current authorization or availability.
pub struct ArtifactRead {
    pub bytes: Vec<u8>,
    pub bytes_transferred: u64,
}
#[derive(Clone, Debug)]
pub enum AccessFailure {
    Denied,
    Missing,
    Unverified,
}

/// Original execution identity is retained. There is intentionally no new run ID.
/// This value cannot be deserialized as a substitute for performing current checks.
#[derive(Debug, Serialize)]
pub struct ReusedResultRef {
    pub original: ExecutionOrigin,
    pub prepared_for_service: String,
    pub receipt_sha256: String,
    pub recipe_fingerprint: String,
    pub result: ResultRecord,
    pub artifacts: BTreeMap<String, ArtifactReference>,
    pub measurements: ReuseMeasurements,
}

impl ReuseReceipt {
    pub fn new(
        recipe: RecipeIdentity,
        original: ExecutionOrigin,
        provenance_source: String,
        result: ResultRecord,
        artifacts: BTreeMap<String, ArtifactReference>,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            version: REUSE_VERSION,
            recipe_fingerprint: recipe.fingerprint()?,
            recipe,
            original,
            provenance_source,
            result,
            artifacts,
        })
    }
    pub fn import(bytes: &[u8]) -> Result<Self, ContractError> {
        Ok(serde_json::from_slice(bytes)?)
    }
    pub fn export(&self) -> Result<Vec<u8>, ContractError> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
    /// Canonical content hash, independent of whitespace/object order in an export.
    pub fn content_digest(&self) -> Result<String, ContractError> {
        content_digest(self)
    }
    pub fn assess(
        &self,
        requested: &RecipeIdentity,
        trust: &ReceiptTrust,
        max_bytes: u64,
        read: impl FnMut(&ArtifactReference) -> Result<ArtifactRead, AccessFailure>,
    ) -> ReuseReport {
        self.check(requested, trust, max_bytes, read).0
    }
    /// Explicit action: all current checks run again, even after an ELIGIBLE assessment.
    /// No TES/WES client is accepted and no submission occurs here.
    pub fn reuse_for(
        &self,
        prepared_for_service: &str,
        requested: &RecipeIdentity,
        trust: &ReceiptTrust,
        max_bytes: u64,
        read: impl FnMut(&ArtifactReference) -> Result<ArtifactRead, AccessFailure>,
    ) -> Result<ReusedResultRef, Box<ReuseReport>> {
        let (report, result) = self.check(requested, trust, max_bytes, read);
        if report.status != ReuseStatus::Eligible {
            return Err(Box::new(report));
        }
        Ok(ReusedResultRef {
            original: self.original.clone(),
            prepared_for_service: prepared_for_service.into(),
            receipt_sha256: report.receipt_sha256,
            recipe_fingerprint: report.recipe_fingerprint,
            result,
            artifacts: self.artifacts.clone(),
            measurements: report.measurements,
        })
    }
    fn check(
        &self,
        requested: &RecipeIdentity,
        trust: &ReceiptTrust,
        max_bytes: u64,
        mut read: impl FnMut(&ArtifactReference) -> Result<ArtifactRead, AccessFailure>,
    ) -> (ReuseReport, ResultRecord) {
        let requested_fingerprint = requested.fingerprint().ok();
        let mut report = ReuseReport {
            status: ReuseStatus::Eligible,
            reasons: vec![],
            original: self.original.clone(),
            recipe_fingerprint: requested_fingerprint.clone().unwrap_or_default(),
            receipt_sha256: self.content_digest().unwrap_or_default(),
            measurements: ReuseMeasurements::default(),
        };
        let mut reject = |code, detail: &str| {
            report.reasons.push(ReuseReason {
                code,
                detail: detail.into(),
                artifact_pointer: None,
            })
        };
        if self.version != REUSE_VERSION {
            reject(
                ReasonCode::UnsupportedReceiptVersion,
                "unsupported reuse receipt version",
            );
        }
        for recipe in [&self.recipe, requested] {
            if recipe.invalid() {
                reject(ReasonCode::InvalidIdentity, "recipe requires complete immutable executable/input/output/runtime identity and safe logical paths");
            }
            if !recipe.determinism.opted_in {
                reject(
                    ReasonCode::RecipeNotOptedIn,
                    "author has not opted this recipe into deterministic reuse",
                );
            }
            if recipe.determinism.network != DependencyUse::None
                || recipe.determinism.secrets != DependencyUse::None
                || recipe.determinism.other_nondeterminism
            {
                reject(
                    ReasonCode::NondeterministicDependency,
                    "undeclared/network/secret/stochastic behavior is ineligible for exact reuse",
                );
            }
        }
        if self.recipe.fingerprint().ok().as_ref() != Some(&self.recipe_fingerprint) {
            reject(
                ReasonCode::FingerprintMismatch,
                "stored recipe fingerprint does not match its recipe",
            );
        }
        // The canonical fingerprint is the exact identity contract. JSON value
        // equality is weaker (for example, -0.0 == 0.0), so never use PartialEq
        // to authorize substitution. Fingerprinting failure also rejects reuse.
        if requested_fingerprint.as_ref() != Some(&self.recipe_fingerprint) {
            reject(ReasonCode::RecipeChanged, "requested recipe/input/parameters/executable/reference/output/runtime fingerprint differs");
        }
        if self.result.execution_state != "COMPLETE" {
            reject(
                ReasonCode::NotComplete,
                "original execution is not COMPLETE",
            );
        }
        if self.result.format_version != 1
            || self.result.schema_digest != self.recipe.output_schema_sha256
            || !self.result.contract_diagnostics.is_empty()
            || self
                .result
                .outputs
                .values()
                .any(|o| o.contract_status != ContractStatus::Conformant)
        {
            reject(
                ReasonCode::InvalidOutputs,
                "typed outputs are not conformant with the pinned output contract",
            );
        }
        if !stable_reference(&self.original.service)
            || self.original.run_id.trim().is_empty()
            || self.result.run_reference.trim().is_empty()
            || self.provenance_source.trim().is_empty()
            || self.artifacts.len() != self.recipe.output_paths.len()
            || self.artifacts.iter().any(|(pointer, a)| {
                self.recipe.output_paths.get(pointer) != Some(&a.relative_path)
                    || !sha256(&a.sha256)
                    || !stable_reference(&a.stable_reference)
                    || self.result.value_at(pointer).and_then(Value::as_str)
                        != Some(a.stable_reference.as_str())
            })
        {
            reject(ReasonCode::ArtifactManifestMismatch, "artifact paths/references/digests or original execution provenance are incomplete or inconsistent");
        }
        match trust {
            ReceiptTrust::Unknown => reject(
                ReasonCode::TrustUnknown,
                "receipt-source trust has not been established for this action",
            ),
            ReceiptTrust::Untrusted => reject(
                ReasonCode::SourceUntrusted,
                "current policy does not trust the receipt source",
            ),
            ReceiptTrust::Pinned {
                receipt_sha256,
                original_service,
                evidence_source,
            } => {
                if !sha256(receipt_sha256)
                    || receipt_sha256 != &report.receipt_sha256
                    || original_service != &self.original.service
                    || evidence_source.trim().is_empty()
                {
                    reject(ReasonCode::ReceiptNotTrusted, "receipt content/original service is not bound to independently trusted evidence");
                }
            }
        }
        if self
            .artifacts
            .values()
            .try_fold(0u64, |sum, a| sum.checked_add(a.size_bytes))
            .is_none_or(|size| size > max_bytes)
        {
            reject(
                ReasonCode::ArtifactTooLarge,
                "artifact manifest exceeds the caller's total verification byte bound",
            );
        }
        let mut result = self.result.clone();
        // Imported verification evidence is historical, never current authorization.
        result.artifacts.clear();
        if report.reasons.is_empty() {
            for (pointer, artifact) in &self.artifacts {
                let mut reject_artifact = |code, detail: &str| {
                    report.reasons.push(ReuseReason {
                        code,
                        detail: detail.into(),
                        artifact_pointer: Some(pointer.clone()),
                    })
                };
                match read(artifact) {
                    Err(AccessFailure::Denied) => reject_artifact(
                        ReasonCode::ArtifactAccessDenied,
                        "current principal's artifact access was denied/revoked",
                    ),
                    Err(AccessFailure::Missing) => reject_artifact(
                        ReasonCode::ArtifactMissing,
                        "artifact bytes are currently missing",
                    ),
                    Err(AccessFailure::Unverified) => reject_artifact(
                        ReasonCode::ArtifactAccessUnverified,
                        "current artifact access/availability could not be verified",
                    ),
                    Ok(read) => {
                        let size = read.bytes.len() as u64;
                        report.measurements.bytes_read =
                            report.measurements.bytes_read.saturating_add(size);
                        report.measurements.bytes_transferred = report
                            .measurements
                            .bytes_transferred
                            .saturating_add(read.bytes_transferred);
                        if report.measurements.bytes_read > max_bytes {
                            reject_artifact(
                                ReasonCode::ArtifactTooLarge,
                                "fresh artifact bytes exceed the caller's total verification bound",
                            );
                            break;
                        }
                        if size != artifact.size_bytes {
                            reject_artifact(
                                ReasonCode::ArtifactSizeMismatch,
                                "fresh artifact size differs from the trusted manifest",
                            );
                        } else if digest(&read.bytes) != artifact.sha256 {
                            reject_artifact(
                                ReasonCode::ArtifactDigestMismatch,
                                "fresh artifact SHA-256 differs from the trusted manifest",
                            );
                        } else if result
                            .verify_bytes(
                                pointer,
                                &read.bytes,
                                Some(ExpectedChecksum {
                                    algorithm: "sha256".into(),
                                    digest: artifact.sha256.clone(),
                                    evidence_source: format!(
                                        "trusted-reuse-receipt:{}",
                                        report.receipt_sha256
                                    ),
                                }),
                            )
                            .is_ok()
                        {
                            report.measurements.bytes_revalidated += size;
                        } else {
                            reject_artifact(
                                ReasonCode::ArtifactDigestMismatch,
                                "artifact evidence could not be verified",
                            );
                        }
                    }
                }
            }
        }
        report.status = if report.reasons.is_empty() {
            ReuseStatus::Eligible
        } else if report.reasons.iter().all(|reason| {
            matches!(
                reason.code,
                ReasonCode::TrustUnknown | ReasonCode::ArtifactAccessUnverified
            )
        }) {
            ReuseStatus::Unverified
        } else {
            ReuseStatus::Ineligible
        };
        (report, result)
    }
}
