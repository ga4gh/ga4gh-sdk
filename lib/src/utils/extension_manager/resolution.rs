//! Match immutable metadata and negotiate interfaces without loading native code.
use crate::utils::extension::{
    ArtifactIdentity, Capabilities, ExtensionError, ExtensionErrorKind, ExtensionResult,
    InstalledArtifact, InstalledExtension, NegotiatedCapabilities,
};
use semver::VersionReq;
use std::cmp::Ordering;

struct Candidate<'a> {
    installation: &'a InstalledArtifact,
    capabilities: NegotiatedCapabilities,
}

pub(super) fn select_artifact(
    record: &InstalledExtension,
    requirement: Option<&VersionReq>,
    digest: Option<&str>,
    required: &Capabilities,
) -> ExtensionResult<(InstalledArtifact, NegotiatedCapabilities)> {
    let mut candidates = Vec::new();
    let mut failure = ExtensionErrorKind::NoMatchingArtifact;

    for installation in &record.artifacts {
        let identity = &installation.manifest.artifact;
        if !matches_request(identity, &record.selected, requirement, digest) {
            continue;
        }
        let compatibility = installation
            .manifest
            .compatible()
            .and_then(|()| negotiate_capabilities(&installation.manifest.capabilities, required));
        match compatibility {
            Ok(capabilities) => candidates.push(Candidate {
                installation,
                capabilities,
            }),
            Err(error) => {
                if failure_priority(error.kind) > failure_priority(failure) {
                    failure = error.kind;
                }
            }
        }
    }

    candidates.sort_by(descending_release);
    let mut candidates = candidates.into_iter();
    let Some(selected) = candidates.next() else {
        return Err(ExtensionError::new(failure));
    };
    if let Some(next) = candidates.next() {
        if descending_release(&selected, &next).is_eq() {
            return Err(ExtensionError::new(ExtensionErrorKind::AmbiguousArtifact));
        }
    }
    Ok((selected.installation.clone(), selected.capabilities))
}

fn matches_request(
    identity: &ArtifactIdentity,
    selected_digest: &str,
    requirement: Option<&VersionReq>,
    digest: Option<&str>,
) -> bool {
    if digest.is_some_and(|digest| digest != identity.digest) {
        return false;
    }
    if requirement.is_some_and(|requirement| !requirement.matches(&identity.version)) {
        return false;
    }
    if requirement.is_none() && digest.is_none() {
        return identity.digest == selected_digest;
    }
    true
}

fn negotiate_capabilities(
    available: &Capabilities,
    required: &Capabilities,
) -> ExtensionResult<NegotiatedCapabilities> {
    let mut negotiated = NegotiatedCapabilities::new();
    for (capability, acceptable) in required {
        let revision = available
            .get(capability)
            .and_then(|supported| highest_common_revision(supported, acceptable))
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::MissingCapability))?;
        negotiated.insert(capability.clone(), revision);
    }
    Ok(negotiated)
}

fn highest_common_revision(supported: &[u32], acceptable: &[u32]) -> Option<u32> {
    supported
        .iter()
        .filter(|revision| acceptable.contains(revision))
        .max()
        .copied()
}

fn descending_release(left: &Candidate<'_>, right: &Candidate<'_>) -> Ordering {
    let left_version = &left.installation.manifest.artifact.version;
    let right_version = &right.installation.manifest.artifact.version;
    right_version.cmp_precedence(left_version)
}

// Prefer the candidate closest to being usable: supported ABI, then host target,
// then capabilities. Installation order must not affect error classification.
fn failure_priority(kind: ExtensionErrorKind) -> u8 {
    match kind {
        ExtensionErrorKind::MissingCapability => 3,
        ExtensionErrorKind::TargetMismatch => 2,
        ExtensionErrorKind::UnsupportedAbi => 1,
        _ => 0,
    }
}
