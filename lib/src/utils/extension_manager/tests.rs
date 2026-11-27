use super::*;
use crate::utils::extension::HOST_TARGET;

#[tokio::test]
async fn optional_entries_skip_fresh_admission_failures_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut manager = ExtensionManager::open(dir.path().join("registry.json")).unwrap();
    let source = dir.path().join("source");
    fs::write(&source, b"not native code").unwrap();
    let manifest = dir.path().join("manifest.json");
    let value = serde_json::json!({
        "manifest_version": 1,
        "name": "optional-race",
        "version": "1.0.0",
        "target": HOST_TARGET,
        "abi_version": 1,
        "digest": extension::hash_artifact(&source).unwrap(),
        "path": "source",
        "capabilities": {}
    });
    fs::write(&manifest, value.to_string()).unwrap();
    manager.add_extension(manifest).await.unwrap();
    for expected in [ExtensionErrorKind::Disabled, ExtensionErrorKind::Missing] {
        manager.enable_extension("optional-race").unwrap();
        let resolved = manager
            .resolve("optional-race", None, None, &Capabilities::new())
            .unwrap();
        // Resolve succeeds; another manager changes metadata before admission.
        let mut concurrent = manager.clone();
        if expected == ExtensionErrorKind::Disabled {
            concurrent.disable_extension("optional-race").unwrap();
        } else {
            concurrent.remove_extension("optional-race").unwrap();
        }
        assert!(
            optional_entry(false, unsafe { manager.create_instance(&resolved, b"{}") })
                .unwrap()
                .is_none()
        );
        assert_eq!(
            optional_entry(true, unsafe { manager.create_instance(&resolved, b"{}") })
                .unwrap_err()
                .kind,
            expected
        );
    }
    assert_eq!(
        optional_entry::<()>(
            false,
            Err(ExtensionError::new(
                ExtensionErrorKind::InitializationFailed
            ))
        )
        .unwrap_err()
        .kind,
        ExtensionErrorKind::InitializationFailed
    );
}
