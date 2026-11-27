//! Installation, error and dynamic-library lifetime boundaries.

use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::extension::{Extension, InstalledExtension};
use ga4gh_sdk::utils::extension_manager::ExtensionManager;
use ga4gh_sdk::utils::json_store::JsonStore;
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::fs;
use std::path::{Path, PathBuf};

fn manifest(dir: &Path, name: &str, url: &str, checksum: &str) -> PathBuf {
    let path = dir.join("manifest.json");
    fs::write(
        &path,
        json!({"name":name,"version":"1.0","path":url,"checksum":checksum}).to_string(),
    )
    .unwrap();
    path
}

fn record(name: &str) -> Value {
    json!({"name":name,"version":"1","definition-path":"missing.json","library-path":"missing.so","enabled":false})
}

#[test]
fn stale_managers_preserve_updates_without_restoring_removed_records() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("extensions.json");
    fs::write(
        &registry,
        json!({"extensions":[record("first"),record("second")]}).to_string(),
    )
    .unwrap();
    let mut first = ExtensionManager::open(&registry).unwrap();
    let mut second = ExtensionManager::open(&registry).unwrap();
    first.enable_extension("first").unwrap();
    second.enable_extension("second").unwrap();
    assert!(ExtensionManager::open(&registry)
        .unwrap()
        .get_extensions()
        .iter()
        .all(|extension| extension.enabled));
    second.remove_extension("first").unwrap();
    first.disable_extension("second").unwrap();
    let stored: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    assert_eq!(stored, json!({"extensions":[record("second")]}));
}

#[test]
fn concurrent_managers_keep_each_enabled_installation() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("extensions.json");
    let names = ["first", "second", "third", "fourth"];
    fs::write(
        &registry,
        json!({"extensions": names.map(record)}).to_string(),
    )
    .unwrap();
    let ready = std::sync::Barrier::new(names.len());
    std::thread::scope(|scope| {
        let registry = &registry;
        let ready = &ready;
        let threads: Vec<_> = names
            .iter()
            .map(|name| {
                scope.spawn(move || {
                    let mut manager = ExtensionManager::open(registry).unwrap();
                    ready.wait();
                    manager.enable_extension(name).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    });
    let manager = ExtensionManager::open(&registry).unwrap();
    assert_eq!(manager.get_extensions().len(), names.len());
    assert!(manager
        .get_extensions()
        .iter()
        .all(|extension| extension.enabled));
}

#[tokio::test]
async fn installing_keeps_registry_changes_made_during_download() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("extensions.json");
    let mut manager = ExtensionManager::open(&registry).unwrap();
    let mut server = mockito::Server::new_async().await;
    let concurrent_registry = registry.clone();
    let bytes = b"downloaded library";
    let request = server
        .mock("GET", "/library.so")
        .with_chunked_body(move |writer| {
            JsonStore::new(&concurrent_registry, "Extension registry")
                .update::<Value, _>(|value| {
                    *value = json!({"extensions":[record("other")]});
                    Ok(())
                })
                .unwrap();
            writer.write_all(bytes)
        })
        .create_async()
        .await;
    let checksum: String = Sha512::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let definition = manifest(
        dir.path(),
        "downloaded",
        &format!("{}/library.so", server.url()),
        &checksum,
    );
    manager.add_extension(definition).await.unwrap();
    request.assert_async().await;
    let names: Vec<_> = manager
        .get_extensions()
        .iter()
        .map(|extension| extension.name.as_str())
        .collect();
    assert_eq!(names, ["other", "downloaded"]);
    assert_eq!(
        ExtensionManager::open(&registry)
            .unwrap()
            .get_extensions()
            .len(),
        2
    );
}

#[test]
fn malformed_extension_settings_and_metadata_do_not_expose_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extension.json");
    fs::write(&path, r#"{"name":{"secret":"PRIVATE_VALUE"}}"#).unwrap();
    for error in [
        Extension::from_file(&path).unwrap_err(),
        InstalledExtension::from_file(&path).unwrap_err(),
        ExtensionManager::open(&path).unwrap_err(),
    ] {
        assert!(!format!("{error} {error:?}").contains("PRIVATE_VALUE"));
    }
    let mut config = Configuration::default();
    config.extra.insert(
        "extensions".into(),
        json!([{"name":{"secret":"PRIVATE_VALUE"},"configuration":{}}]),
    );
    // SAFETY: malformed settings must fail before any native library loads.
    let error = unsafe { ExtensionManager::from_configuration(&path, &config) }.unwrap_err();
    assert!(matches!(error, ga4gh_sdk::Error::Configuration(_)));
    assert!(!format!("{error} {error:?}").contains("PRIVATE_VALUE"));
    let missing = dir.path().join("missing.json");
    assert_eq!(
        Extension::from_file(&missing)
            .unwrap_err()
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotFound
    );
    assert!(!missing.exists());
}

#[tokio::test]
async fn install_enable_reopen_disable_remove() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("extensions.json");
    let mut manager = ExtensionManager::open(&registry).unwrap();
    assert!(!registry.exists());
    let mut server = mockito::Server::new_async().await;
    let bytes = b"fixture bytes, not executable";
    let download = server
        .mock("GET", "/plugin.so")
        .with_body(bytes)
        .expect(1)
        .create_async()
        .await;
    let checksum: String = Sha512::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let definition = manifest(
        dir.path(),
        "example",
        &format!("{}/plugin.so", server.url()),
        &checksum,
    );
    manager.add_extension(&definition).await.unwrap();
    download.assert_async().await;
    let installed = &manager.get_extensions()[0];
    assert!(!installed.enabled);
    assert!(!installed.is_loaded());
    assert_eq!(fs::read(&installed.library_path).unwrap(), bytes);
    assert!(Path::new(&installed.definition_path).exists());
    assert!(installed
        .library_path
        .ends_with(std::env::consts::DLL_SUFFIX));
    let library = PathBuf::from(&installed.library_path);
    let original_registry = fs::read(&registry).unwrap();
    fs::remove_file(&registry).unwrap();
    fs::create_dir(&registry).unwrap();
    assert!(manager.remove_extension("example").is_err());
    assert_eq!(fs::read(&library).unwrap(), bytes);
    assert_eq!(manager.get_extensions().len(), 1);
    assert!(manager.enable_extension("example").is_err());
    assert!(!manager.get_extensions()[0].enabled);
    fs::remove_dir(&registry).unwrap();
    fs::write(&registry, original_registry).unwrap();
    assert!(manager.add_extension(&definition).await.is_err());
    manager.enable_extension("example").unwrap();
    let mut reopened = ExtensionManager::open(&registry).unwrap();
    assert!(reopened.get_extensions()[0].enabled);
    assert!(!reopened.get_extensions()[0].is_loaded());
    reopened.disable_extension("example").unwrap();
    assert!(!ExtensionManager::open(&registry).unwrap().get_extensions()[0].enabled);
    reopened.remove_extension("example").unwrap();
    assert!(!dir.path().join("extensions/example").exists());
    assert!(ExtensionManager::open(&registry)
        .unwrap()
        .get_extensions()
        .is_empty());
}

#[tokio::test]
async fn failed_downloads_and_checksums_do_not_install() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("extensions.json");
    let mut manager = ExtensionManager::open(&registry).unwrap();
    let mut server = mockito::Server::new_async().await;
    for (status, name) in [(503, "unavailable"), (200, "bad-checksum")] {
        let request = server
            .mock("GET", "/plugin.so")
            .with_status(status)
            .with_body("wrong bytes")
            .create_async()
            .await;
        let definition = manifest(
            dir.path(),
            name,
            &format!("{}/plugin.so", server.url()),
            &"0".repeat(128),
        );
        let error = manager.add_extension(definition).await.unwrap_err();
        if status == 503 {
            assert!(
                matches!(error, ga4gh_sdk::Error::Transport(ref source) if source.status().unwrap().as_u16() == 503)
            );
        } else {
            assert!(error.to_string().contains("checksum mismatch"));
        }
        assert!(!dir.path().join(format!("extensions/{name}")).exists());
        assert!(!registry.exists());
        assert!(manager.get_extensions().is_empty());
        request.assert_async().await;
    }
}

#[tokio::test]
async fn invalid_names_cannot_escape_installation_directory() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("extensions.json");
    let sentinel = dir.path().join("keep.txt");
    fs::write(&sentinel, "keep").unwrap();
    let mut manager = ExtensionManager::open(&registry).unwrap();
    for name in ["..", ".", "", "../outside", "a/b", "a\\b", "/tmp/outside"] {
        assert!(manager.remove_extension(name).is_err());
        assert!(manager.enable_extension(name).is_err());
        let definition = manifest(
            dir.path(),
            name,
            "http://127.0.0.1:1/plugin.so",
            &"0".repeat(128),
        );
        assert!(manager.add_extension(definition).await.is_err());
    }
    assert!(manager.remove_extension("unknown").is_err());
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "keep");
    assert!(!registry.exists());
}

#[test]
fn corrupt_registry_and_missing_required_extension_fail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    fs::write(&path, "{malformed").unwrap();
    assert!(ExtensionManager::open(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{malformed");
    fs::remove_file(&path).unwrap();
    let mut config = Configuration::default();
    config.extra.insert(
        "extensions".into(),
        json!([{"name":"missing","required":true,"configuration":{}}]),
    );
    // SAFETY: the empty registry cannot load native code.
    let result = unsafe { ExtensionManager::from_configuration(&path, &config) };
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Required extension"));
    config.extra["extensions"][0]["required"] = json!(false);
    assert!(unsafe { ExtensionManager::from_configuration(&path, &config) }.is_ok());
    config.extra.insert("extensions".into(), json!({}));
    assert!(unsafe { ExtensionManager::from_configuration(&path, &config) }.is_err());
}

#[test]
fn retained_methods_pin_the_library_until_last_drop() {
    let dir = tempfile::tempdir().unwrap();
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/native-extension/Cargo.toml");
    let build = std::process::Command::new(env!("CARGO"))
        .args([
            "build",
            "--offline",
            "--locked",
            "--message-format=json",
            "--manifest-path",
        ])
        .arg(fixture)
        .arg("--target-dir")
        .arg(dir.path().join("target"))
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let library = String::from_utf8(build.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["name"] == "ga4gh_extension_fixture"
        })
        .flat_map(|message| message["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|value| value.as_str().map(PathBuf::from))
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == &std::env::consts::DLL_SUFFIX[1..])
        })
        .expect("fixture build must produce a shared library");
    let registry = dir.path().join("extensions.json");
    let dropped = dir.path().join("dropped");
    fs::write(&registry, json!({"extensions":[{"name":"fixture","version":"1","definition-path":"fixture.json","library-path":library,"enabled":true}]}).to_string()).unwrap();
    let config_path = dir.path().join("service.json");
    fs::write(
        &config_path,
        json!({"profiles":{"site":{"CUSTOM":{
            "base_path":"https://service.example.org/",
            "bearer_access_token":"host-only-credential",
            "extensions":[{"name":"not-selected","required":true,"configuration":{}}]
        }}}})
        .to_string(),
    )
    .unwrap();
    let sources = Configuration::sources(&config_path, Some("site"), "CUSTOM")
        .unwrap()
        .merge(figment::providers::Serialized::defaults(
            json!({"extensions":[{
                "name":"fixture","required":true,"configuration":{"on_drop":dropped}
            }]}),
        ));
    let config = Configuration::from_provider(sources).unwrap();
    // SAFETY: the fixture is built alongside this host with the same dependencies
    // and implements the documented ABI without extension-specific restrictions.
    let mut manager = unsafe { ExtensionManager::from_configuration(&registry, &config) }.unwrap();
    let method = manager
        .lookup_extension_methods("test")
        .into_iter()
        .find(|method| method.internal_name == "configuration")
        .unwrap();
    assert_eq!(
        unsafe { method.call(Value::Null) },
        json!({"on_drop":dropped})
    );
    manager.enable_extension("fixture").unwrap();
    assert!(manager.get_extensions()[0].is_loaded());
    let mut cloned = manager.clone();
    let method = cloned.lookup_extension_methods("test")[0].clone();
    drop(manager);
    cloned.disable_extension("fixture").unwrap();
    drop(cloned);
    assert!(!dropped.exists());
    let input: Value = json!({"still":"callable"});
    assert_eq!(unsafe { method.call(input.clone()) }, input);
    drop(method);
    assert_eq!(fs::read_to_string(dropped).unwrap(), "deinitialized");
}
