//! Prepare and publish verified immutable artifacts before registry registration.
use super::ExtensionManager;
use crate::utils::{
    extension::{
        self, Extension, ExtensionError, ExtensionErrorKind, ExtensionResult, InstalledArtifact,
    },
    json_store::JsonStore,
};
use sha2::{Digest, Sha512};
use std::{fs, io::Write, path::Path};

pub(super) async fn install(
    manager: &ExtensionManager,
    path: &Path,
) -> ExtensionResult<InstalledArtifact> {
    let mut manifest = Extension::from_file(path)?;
    manager.check_root(&manifest.artifact.name)?;
    fs::create_dir_all(&manager.root).map_err(extension::io)?;
    let root = fs::canonicalize(&manager.root).map_err(extension::io)?;
    let staging = tempfile::tempdir_in(&root).map_err(extension::io)?;
    let filename = library_filename();
    let prepared = staging.path().join(&filename);
    let mut file = fs::File::create(&prepared).map_err(extension::io)?;
    let digest = copy_source(&manifest.path, path, &mut file).await?;
    file.sync_all().map_err(extension::io)?;
    drop(file);
    if digest != manifest.artifact.digest {
        return Err(ExtensionError::new(ExtensionErrorKind::DigestMismatch));
    }
    // Installation sources may contain query tokens or private local paths.
    // Persist only the relative filename of the verified managed artifact.
    manifest.path = filename.clone();
    JsonStore::new(staging.path().join("manifest.json"), "Extension manifest")
        .write(&manifest)
        .map_err(extension::storage)?;
    let name = manifest.artifact.name.clone();
    let digest = manifest.artifact.digest.clone();
    let destination = root.join(&name).join(&digest);
    let installation = InstalledArtifact {
        manifest,
        library_path: destination.join(&filename),
    };
    publish(manager, &root, staging.path(), &installation)?;
    Ok(installation)
}

fn library_filename() -> String {
    format!("library{}", std::env::consts::DLL_SUFFIX)
}

async fn copy_source(
    source: &str,
    manifest_path: &Path,
    file: &mut fs::File,
) -> ExtensionResult<String> {
    let parsed_url = url::Url::parse(source).ok();
    match parsed_url.filter(|url| matches!(url.scheme(), "http" | "https")) {
        Some(url) => download(url, file).await,
        None => copy_local(source, manifest_path, file),
    }
}

async fn download(url: url::Url, file: &mut fs::File) -> ExtensionResult<String> {
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
    }
    let mut response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?
        .get(url)
        .send()
        .await
        .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?
        .error_for_status()
        .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?;
    if !response.status().is_success() {
        return Err(ExtensionError::new(ExtensionErrorKind::Storage));
    }
    extension::artifact_size(response.content_length().unwrap_or(0))?;
    let mut digest = Sha512::new();
    let mut length = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| ExtensionError::caused(ExtensionErrorKind::Storage, e))?
    {
        length += chunk.len() as u64;
        extension::artifact_size(length)?;
        digest.update(&chunk);
        file.write_all(&chunk).map_err(extension::io)?;
    }
    Ok(extension::digest_hex(digest.finalize()))
}

fn copy_local(source: &str, manifest_path: &Path, file: &mut fs::File) -> ExtensionResult<String> {
    if source.contains("://") || source.is_empty() {
        return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
    }
    let source = manifest_path
        .parent()
        .unwrap_or(Path::new("."))
        .join(source);
    let source = fs::File::open(source).map_err(extension::io)?;
    extension::artifact_size(source.metadata().map_err(extension::io)?.len())?;
    extension::copy_artifact(source, file)
}

fn publish(
    manager: &ExtensionManager,
    root: &Path,
    staging: &Path,
    installation: &InstalledArtifact,
) -> ExtensionResult<()> {
    let name = &installation.manifest.artifact.name;
    let digest = &installation.manifest.artifact.digest;
    let filename = library_filename();
    let destination = root.join(name).join(digest);
    // Publication is coordinated by the shared JsonStore lock mechanism,
    // independently of any registry using this installation root. Hashing an
    // existing artifact must not hold up instance admission on a registry lock.
    {
        let publication = JsonStore::new(
            root.join(name).join(format!("{digest}.artifact")),
            "Extension artifact",
        );
        let _lock = publication.lock().map_err(extension::storage)?;
        manager.check_root(name)?;
        if destination.is_symlink() {
            return Err(ExtensionError::new(ExtensionErrorKind::Storage));
        }
        if destination.exists() {
            if destination.join(&filename).is_symlink() {
                return Err(ExtensionError::new(ExtensionErrorKind::Storage));
            }
            let existing_digest = extension::hash_artifact(&destination.join(&filename))?;
            if existing_digest != *digest {
                return Err(ExtensionError::new(ExtensionErrorKind::DigestMismatch));
            }
            let previous = Extension::from_file(destination.join("manifest.json"))?;
            if previous.artifact != installation.manifest.artifact
                || previous.descriptor() != installation.manifest.descriptor()
            {
                return Err(ExtensionError::new(ExtensionErrorKind::InvalidMetadata));
            }
        } else {
            sync_artifact_directories(staging)?;
            fs::rename(staging, &destination).map_err(extension::io)?;
        }
    }
    sync_artifact_directories(&destination)?;
    Ok(())
}

// JsonStore syncs a replaced file's immediate parent. Artifact publication also
// needs its renamed directory and ancestor entries persisted before registration.
fn sync_artifact_directories(directory: &Path) -> ExtensionResult<()> {
    #[cfg(unix)]
    for path in directory.ancestors() {
        fs::File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(extension::io)?;
    }
    #[cfg(not(unix))]
    let _ = directory;
    Ok(())
}
