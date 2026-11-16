//! Explicit exact reuse of trusted deterministic results. This module never submits a job.
//!
//! Receipt hashes authenticate nothing by themselves. A caller must obtain the pinned hash
//! and original-service identity from an independently trusted, authorized source and supply
//! a fresh access reader for the current principal. CrateGen remains the provenance converter.
//!
//! A [`ReuseReceipt`] records a finished run: the [`RecipeIdentity`] that it
//! ran, where it ran ([`ExecutionOrigin`]), its typed result and its output
//! artifacts. [`ReuseReceipt::assess`] checks whether the receipt can stand
//! in for a new run of a requested recipe and gives a [`ReuseReport`];
//! [`ReuseReceipt::reuse_for`] runs the same checks and gives a
//! [`ReusedResultRef`] only if the receipt is eligible. If every other
//! check passes, both read the artifacts again through the caller's reader
//! and compare their sizes and SHA-256 digests with the receipt.
//!
//! The caller supplies trust for each check as a [`ReceiptTrust`], and the
//! reader returns an [`ArtifactRead`] or an [`AccessFailure`]. Receipts are
//! built with [`ReuseReceipt::new`], and moved between systems with
//! [`ReuseReceipt::export`] and [`ReuseReceipt::import`].
use super::{
    digest,
    results::{ContractStatus, ExpectedChecksum, ResultRecord},
    ContractError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// The version of [`RecipeIdentity`] and [`ReuseReceipt`] that this module
/// writes and accepts.
pub const REUSE_VERSION: u32 = 1;

/// States whether a recipe uses a dependency, such as the network.
///
/// This is declared behavior, not a sandbox. Only `None` is eligible for
/// exact reuse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyUse {
    /// The recipe doesn't use the dependency.
    None,
    /// The recipe uses the dependency.
    Dependent,
    /// The author hasn't said whether the recipe uses it.
    Undeclared,
}

/// The recipe author's statement that the recipe gives the same outputs for
/// the same inputs. Only a recipe that opts in, uses no network and no
/// secrets, and has no other nondeterminism is eligible for reuse.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeterminismContract {
    /// Whether the author allows exact reuse of the recipe's results.
    pub opted_in: bool,
    /// Who made the statement; not blank.
    pub author: String,
    /// The revision of the statement; not blank.
    pub revision: String,
    /// Whether the recipe uses the network.
    pub network: DependencyUse,
    /// Whether the recipe uses secrets.
    pub secrets: DependencyUse,
    /// Whether the recipe is nondeterministic in another way, for example
    /// random without a fixed seed.
    pub other_nondeterminism: bool,
}

/// Typed values do not equate strings, integers, decimals or booleans.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum TypedValue {
    /// A string.
    String(String),
    /// A signed integer.
    Integer(i64),
    /// An unsigned integer.
    Unsigned(u64),
    /// A decimal number, written as a JSON number in a string, such as
    /// `"0.10"`.
    Decimal(String),
    /// A boolean.
    Boolean(bool),
    /// Any JSON value.
    Json(Value),
}

/// The content of an input file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentIdentity {
    /// The SHA-256 of the content, as 64 lowercase hex digits.
    pub sha256: String,
    /// The size of the content in bytes.
    pub size_bytes: u64,
}

/// A version of reference data, such as a genome assembly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceIdentity {
    /// The version name; not blank.
    pub version: String,
    /// The SHA-256 of the reference content, as 64 lowercase hex digits.
    pub sha256: String,
}

/// Identifies exactly what a run executes, so that two runs of the same
/// recipe can be recognized by its [`fingerprint`](Self::fingerprint).
///
/// The recipe author must declare all fields that affect execution.
/// Provider IDs, temporary access URLs and timestamps are deliberately
/// outside the fingerprint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeIdentity {
    /// The format version; must be [`REUSE_VERSION`].
    pub version: u32,
    /// The author's determinism statement.
    pub determinism: DeterminismContract,
    /// SHA-256 of the actual executable/script bytes, not a mutable URL or tag.
    pub executable_sha256: String,
    /// Immutable OCI manifest identity, e.g. `python@sha256:<64 lowercase hex>`.
    pub container_image: String,
    /// The command and its arguments; not empty, and no argument is empty
    /// or contains a NUL character.
    pub command: Vec<String>,
    /// The environment variables of the command.
    pub environment: BTreeMap<String, String>,
    /// Further configuration that affects the outputs, as JSON.
    pub configuration: Value,
    /// The parameters, by name, with their types.
    pub parameters: BTreeMap<String, TypedValue>,
    /// Keys are portable relative logical paths, not provider staging paths.
    pub inputs: BTreeMap<String, ContentIdentity>,
    /// Keys are reference names; values are immutable version/content identities.
    pub reference_versions: BTreeMap<String, ReferenceIdentity>,
    /// The SHA-256 of the output schema that the result must conform to.
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
    /// The SHA-256 of the recipe's canonical JSON (keys sorted at every
    /// level), which identifies it exactly.
    ///
    /// # Errors
    ///
    /// Fails if the recipe can't be serialized as JSON.
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

/// The run that produced a receipt's result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOrigin {
    /// A stable URL or URN of the service that ran it, without credentials,
    /// query or fragment.
    pub service: String,
    /// The run ID at that service; not blank.
    pub run_id: String,
    /// Exact durable run reference recorded in the accompanying result.
    pub run_reference: String,
}

/// An output artifact of a receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    /// The output path that the recipe's `output_paths` requires for it.
    pub relative_path: String,
    /// A stable `drs`, `https`, `http`, `file` or `urn` reference to the
    /// bytes, without credentials, query or fragment; the result's output
    /// value must be this reference.
    pub stable_reference: String,
    /// The size of the artifact in bytes.
    pub size_bytes: u64,
    /// The SHA-256 of the artifact, as 64 lowercase hex digits.
    pub sha256: String,
}

/// Records a finished run, so that its result can stand in for a new run of
/// the same recipe.
///
/// It holds the run's typed [`ResultRecord`]. It is an eligibility receipt,
/// not a replacement for a provenance package, a global cache or an artifact
/// upload protocol.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReuseReceipt {
    /// The format version; must be [`REUSE_VERSION`].
    pub version: u32,
    /// The recipe that the original run ran.
    pub recipe: RecipeIdentity,
    /// The recipe's [`RecipeIdentity::fingerprint`].
    pub recipe_fingerprint: String,
    /// The original run.
    pub original: ExecutionOrigin,
    /// Where the provenance of the run comes from; not blank.
    pub provenance_source: String,
    /// The typed result of the original run.
    pub result: ResultRecord,
    /// Artifact output JSON pointer -> durable manifest evidence.
    pub artifacts: BTreeMap<String, ArtifactReference>,
}

/// Says whether the caller trusts a receipt, for one check.
///
/// Trust is supplied for each check and never taken from the receipt
/// itself.
#[derive(Clone, Debug)]
pub enum ReceiptTrust {
    /// The caller trusts exactly this receipt, from a trusted source.
    Pinned {
        /// The receipt's [`ReuseReceipt::content_digest`].
        receipt_sha256: String,
        /// The service that must have run the original run.
        original_service: String,
        /// Where the caller got the hash and service from; not blank.
        evidence_source: String,
    },
    /// The caller's policy doesn't trust the receipt's source.
    Untrusted,
    /// The caller hasn't established trust yet.
    Unknown,
}

/// Whether a receipt can stand in for a new run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReuseStatus {
    /// Every check passed.
    Eligible,
    /// A check failed.
    Ineligible,
    /// No check definitively failed, but trust is unknown or an artifact's
    /// access couldn't be verified. With [`ReceiptTrust::Unknown`], no artifact is
    /// read, so their sizes and digests are unchecked too.
    Unverified,
}
/// Why a check of a receipt failed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    /// The receipt's or requested recipe lacks a complete, valid immutable
    /// identity, for example: its version, the determinism author or
    /// revision, executable, image digest, command, inputs, reference
    /// versions, decimal parameters, output schema, outputs, runtime, or
    /// safe and unique relative paths.
    InvalidIdentity,
    /// The author hasn't opted a recipe into reuse.
    RecipeNotOptedIn,
    /// A recipe uses the network or secrets, or is otherwise
    /// nondeterministic, or doesn't say.
    NondeterministicDependency,
    /// The requested recipe's fingerprint differs from the receipt's.
    RecipeChanged,
    /// The receipt's version isn't [`REUSE_VERSION`].
    UnsupportedReceiptVersion,
    /// The receipt's stored fingerprint doesn't match its recipe.
    FingerprintMismatch,
    /// The original run didn't end in `COMPLETE`.
    NotComplete,
    /// The result doesn't conform to the recipe's output schema.
    InvalidOutputs,
    /// The artifacts or the original run's references are incomplete or
    /// don't match the recipe and the result, or the receipt's
    /// `provenance_source` or run ID is blank.
    ArtifactManifestMismatch,
    /// [`ReceiptTrust::Unknown`] was given.
    TrustUnknown,
    /// [`ReceiptTrust::Untrusted`] was given.
    SourceUntrusted,
    /// The pinned hash or service doesn't match the receipt, or the pin is
    /// incomplete.
    ReceiptNotTrusted,
    /// The reader was denied access to an artifact.
    ArtifactAccessDenied,
    /// An artifact is missing.
    ArtifactMissing,
    /// The reader couldn't verify access to an artifact.
    ArtifactAccessUnverified,
    /// The artifacts are larger than the caller's byte limit.
    ArtifactTooLarge,
    /// An artifact's size differs from the receipt.
    ArtifactSizeMismatch,
    /// An artifact's SHA-256 differs from the receipt, or its evidence
    /// couldn't be verified.
    ArtifactDigestMismatch,
}
/// One failed check of a receipt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReuseReason {
    /// What failed.
    pub code: ReasonCode,
    /// A description of the failure.
    pub detail: String,
    /// The output JSON pointer of the artifact, for checks of a single
    /// artifact read; `None` otherwise.
    pub artifact_pointer: Option<String>,
}
/// The bytes that the artifact checks read.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReuseMeasurements {
    /// The bytes of all artifacts that were read, including a read that
    /// exceeded `max_bytes`.
    pub bytes_read: u64,
    /// The bytes of the artifacts that passed their checks.
    pub bytes_revalidated: u64,
    /// The bytes that the reader reports it transferred; 0 for local reads.
    pub bytes_transferred: u64,
}
/// The result of checking a receipt against a requested recipe.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReuseReport {
    /// Whether the receipt can stand in for a new run.
    pub status: ReuseStatus,
    /// The failed checks; empty if the receipt is eligible.
    pub reasons: Vec<ReuseReason>,
    /// The receipt's original run.
    pub original: ExecutionOrigin,
    /// The requested recipe's fingerprint; empty if it can't be computed.
    pub recipe_fingerprint: String,
    /// The receipt's [`ReuseReceipt::content_digest`]; empty if it can't be
    /// computed.
    pub receipt_sha256: String,
    /// The bytes that the artifact checks read.
    pub measurements: ReuseMeasurements,
}

/// Holds the bytes of one artifact read, as the caller's reader returns them.
///
/// Each read must be fresh and authorized for the current principal, and
/// the reader must bound its size before allocating. Don't return stale
/// cached bytes as evidence of current authorization or availability.
pub struct ArtifactRead {
    /// The artifact's bytes.
    pub bytes: Vec<u8>,
    /// The bytes that the reader transferred over the network; 0 for local
    /// reads.
    pub bytes_transferred: u64,
}
/// Why an artifact read failed.
#[derive(Clone, Debug)]
pub enum AccessFailure {
    /// The current principal may not read the artifact.
    Denied,
    /// The artifact doesn't exist anymore.
    Missing,
    /// Access couldn't be checked, for example because the storage didn't
    /// answer.
    Unverified,
}

/// Refers to a reused result, which [`ReuseReceipt::reuse_for`] returns.
///
/// It keeps the original run's identity; there is intentionally no new
/// run ID. It can't be deserialized, so it can't stand in for running the
/// checks again.
#[derive(Debug, Serialize)]
pub struct ReusedResultRef {
    /// The original run, whose result is reused.
    pub original: ExecutionOrigin,
    /// The service that the reused result was prepared for.
    pub prepared_for_service: String,
    /// The receipt's [`ReuseReceipt::content_digest`].
    pub receipt_sha256: String,
    /// The recipe's fingerprint.
    pub recipe_fingerprint: String,
    /// The original result, with the artifact evidence of this check.
    pub result: ResultRecord,
    /// The receipt's artifacts, by output JSON pointer.
    pub artifacts: BTreeMap<String, ArtifactReference>,
    /// The bytes that the artifact checks read.
    pub measurements: ReuseMeasurements,
}

impl ReuseReceipt {
    /// Builds a receipt of version [`REUSE_VERSION`] with the recipe's
    /// fingerprint. Checks nothing else; [`Self::assess`] does.
    ///
    /// # Errors
    ///
    /// Fails if the recipe's fingerprint can't be computed.
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
    /// Reads a receipt from its JSON. An imported receipt is not trusted;
    /// pass [`ReceiptTrust`] to [`Self::assess`].
    ///
    /// # Errors
    ///
    /// Fails if `bytes` aren't the JSON of a receipt, with no unknown fields.
    pub fn import(bytes: &[u8]) -> Result<Self, ContractError> {
        Ok(serde_json::from_slice(bytes)?)
    }
    /// Writes the receipt as pretty-printed JSON.
    ///
    /// # Errors
    ///
    /// Fails if the receipt can't be serialized as JSON.
    pub fn export(&self) -> Result<Vec<u8>, ContractError> {
        Ok(serde_json::to_vec_pretty(self)?)
    }
    /// Returns the SHA-256 of the receipt's canonical JSON (keys sorted at
    /// every level), which doesn't depend on whitespace or key order in an
    /// export.
    ///
    /// # Errors
    ///
    /// Fails if the receipt can't be serialized as JSON.
    pub fn content_digest(&self) -> Result<String, ContractError> {
        content_digest(self)
    }
    /// Checks whether the receipt can stand in for a run of `requested`.
    ///
    /// The receipt must be valid and trusted by `trust`, both recipes must be
    /// eligible and have the same fingerprint, and the original run must
    /// have completed with conformant outputs, and the declared artifact
    /// sizes must total at most `max_bytes`. Only then are the artifacts
    /// read with `read` and checked against the receipt's sizes and SHA-256
    /// digests. Sends no request itself.
    ///
    /// If the receipt's declared artifact sizes add up to more than
    /// `max_bytes`, or the bytes read so far exceed it, the report gets
    /// [`ReasonCode::ArtifactTooLarge`] and no further artifact is read. The
    /// SDK doesn't pass `max_bytes` to `read`, so the reader must bound each
    /// read itself, before allocating.
    ///
    /// Returns the [`ReuseReport`]; failed checks appear in its `reasons`.
    pub fn assess(
        &self,
        requested: &RecipeIdentity,
        trust: &ReceiptTrust,
        max_bytes: u64,
        read: impl FnMut(&ArtifactReference) -> Result<ArtifactRead, AccessFailure>,
    ) -> ReuseReport {
        self.check(requested, trust, max_bytes, read).0
    }
    /// Gives the result of the receipt as the result of a run of `requested`
    /// at `prepared_for_service`, if [`Self::assess`] finds it eligible.
    ///
    /// All checks run again, even after an assessment that returned
    /// [`ReuseStatus::Eligible`]. It takes no TES or WES client and submits
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns `Err` with the boxed [`ReuseReport`] if the status isn't
    /// [`ReuseStatus::Eligible`].
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
            || !stable_reference(&self.original.run_reference)
            || self.result.run_reference != self.original.run_reference
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
