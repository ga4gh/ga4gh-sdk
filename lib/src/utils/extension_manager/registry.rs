//! Registry format, validation, and artifact registration. Callers own locking.
use crate::utils::{
    extension::{
        self, ExtensionError, ExtensionErrorKind, ExtensionResult, InstalledArtifact,
        InstalledExtension,
    },
    json_store::JsonStore,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Registry {
    registry_version: u32,
    pub(super) extensions: Vec<InstalledExtension>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            registry_version: 1,
            extensions: vec![],
        }
    }
}

impl Registry {
    pub(super) fn read(path: &Path) -> ExtensionResult<Registry> {
        let Some(value) = JsonStore::new(path, "Extension registry")
            .read::<serde_json::Value>()
            .map_err(extension::storage)?
        else {
            return Ok(Registry::default());
        };
        if value.get("registry_version").and_then(|v| v.as_u64()) != Some(1) {
            return Err(ExtensionError::new(ExtensionErrorKind::UnsupportedManifest));
        }
        let registry: Registry = serde_json::from_value(value)
            .map_err(|_| ExtensionError::new(ExtensionErrorKind::InvalidMetadata))?;
        let mut names = std::collections::HashSet::new();
        for record in &registry.extensions {
            extension::validate_name(&record.name)?;
            if !names.insert(&record.name)
                || record.artifacts.is_empty()
                || !record
                    .artifacts
                    .iter()
                    .any(|artifact| artifact.manifest.artifact.digest == record.selected)
            {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
            let mut digests = std::collections::HashSet::new();
            for artifact in &record.artifacts {
                artifact.manifest.validate()?;
                if artifact.manifest.artifact.name != record.name
                    || !artifact.library_path.is_absolute()
                    || !digests.insert(&artifact.manifest.artifact.digest)
                {
                    return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
                }
            }
        }
        Ok(registry)
    }

    pub(super) fn register(&mut self, installation: InstalledArtifact) -> ExtensionResult<()> {
        let name = installation.manifest.artifact.name.clone();
        let digest = installation.manifest.artifact.digest.clone();
        if let Some(record) = self
            .extensions
            .iter_mut()
            .find(|record| record.name == name)
        {
            if let Some(previous) = record
                .artifacts
                .iter()
                .find(|artifact| artifact.manifest.artifact.digest == digest)
            {
                if previous.manifest.artifact != installation.manifest.artifact
                    || previous.manifest.descriptor() != installation.manifest.descriptor()
                    || previous.library_path != installation.library_path
                {
                    return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
                }
            } else {
                record.artifacts.push(installation);
            }
            record.selected = digest;
        } else {
            self.extensions.push(InstalledExtension {
                name,
                enabled: false,
                selected: digest,
                artifacts: vec![installation],
            });
        }
        Ok(())
    }

    pub(super) fn remove(&mut self, name: &str) -> ExtensionResult<()> {
        let index = self
            .extensions
            .iter()
            .position(|record| record.name == name)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
        self.extensions.remove(index);
        Ok(())
    }

    pub(super) fn set_enabled(&mut self, name: &str, enabled: bool) -> ExtensionResult<()> {
        self.extensions
            .iter_mut()
            .find(|record| record.name == name)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?
            .enabled = enabled;
        Ok(())
    }

    pub(super) fn select(&mut self, name: &str, digest: &str) -> ExtensionResult<()> {
        let record = self
            .extensions
            .iter_mut()
            .find(|record| record.name == name)
            .ok_or_else(|| ExtensionError::new(ExtensionErrorKind::Missing))?;
        if !record
            .artifacts
            .iter()
            .any(|artifact| artifact.manifest.artifact.digest == digest)
        {
            return Err(ExtensionError::new(ExtensionErrorKind::Missing));
        }
        record.selected = digest.to_owned();
        Ok(())
    }
}
