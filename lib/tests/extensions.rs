//! Public management, native ABI, concurrency, and lifecycle boundaries.
use ga4gh_sdk::utils::{
    configuration::Configuration, extension::*, extension_manager::ExtensionManager,
    json_store::JsonStore,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

fn digest(bytes: &[u8]) -> String {
    Sha512::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn manifest(dir: &Path, name: &str, version: &str, source: &str, bytes: &[u8]) -> PathBuf {
    let file = dir.join(format!("{name}-{version}-{}.json", digest(bytes)));
    fs::write(
        &file,
        json!({
            "manifest_version": 1,
            "name": name,
            "version": version,
            "path": source,
            "digest": digest(bytes),
            "target": HOST_TARGET,
            "abi_version": 1,
            "capabilities": {
                "test": [1, 2]
            }
        })
        .to_string(),
    )
    .unwrap();
    file
}

fn edit(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut value);
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

fn caps() -> Capabilities {
    serde_json::from_value(json!({"test": [1,2,3]})).unwrap()
}

fn manager(dir: &Path) -> ExtensionManager {
    ExtensionManager::with_paths(dir.join("registry.json"), dir.join("separate-artifacts")).unwrap()
}

async fn install_bytes(m: &mut ExtensionManager, dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let local = dir.join(format!("{name}-source"));
    fs::write(&local, bytes).unwrap();
    let path = manifest(dir, name, "1.0.0", local.to_str().unwrap(), bytes);
    m.add_extension(&path).await.unwrap();
    path
}

fn compile(dir: &Path, name: &str, version: &str, extra: &[(&str, &str)]) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let output = dir.join(format!("{name}-{version}{}", std::env::consts::DLL_SUFFIX));
    let mut build = cc::Build::new();
    build
        .cargo_metadata(false)
        .host(HOST_TARGET)
        .target(HOST_TARGET)
        .opt_level(0)
        .out_dir(dir);
    let compiler = build.get_compiler();
    let mut command = compiler.to_command();
    if compiler.is_like_msvc() {
        command.args(["/LD", "/W4"]);
        command.arg(format!("/Fe:{}", output.display()));
        command.arg(format!(
            "/I{}",
            root.join("extension-abi/include").display()
        ));
    } else {
        command
            .arg(if cfg!(target_os = "macos") {
                "-dynamiclib"
            } else {
                "-shared"
            })
            .arg("-fPIC")
            .arg("-o")
            .arg(&output)
            .arg("-I")
            .arg(root.join("extension-abi/include"));
    }
    for (key, value) in [
        ("PLUGIN_NAME", format!("\"{name}\"")),
        ("PLUGIN_VERSION", format!("\"{version}\"")),
        ("TARGET_TRIPLE", format!("\"{HOST_TARGET}\"")),
    ]
    .iter()
    .map(|(a, b)| (*a, b.as_str()))
    .chain(extra.iter().copied())
    {
        command.arg(format!(
            "{}{}={}",
            if compiler.is_like_msvc() { "/D" } else { "-D" },
            key,
            value
        ));
    }
    let result = command
        .arg(root.join("lib/tests/fixtures/native-extension/extension.c"))
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    output
}

fn wait(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(Instant::now() < deadline, "timed out: {}", path.display());
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn input(path: &Path) -> Vec<u8> {
    path.to_str().unwrap().as_bytes().to_vec()
}

#[tokio::test]
async fn metadata_operations_preserve_stale_and_concurrent_changes_without_loading() {
    let dir = tempfile::tempdir().unwrap();
    let mut first = manager(dir.path());
    assert!(!dir.path().join("registry.json").exists());
    for name in ["first", "second", "third", "fourth"] {
        install_bytes(&mut first, dir.path(), name, b"not executable").await;
    }
    let barrier = Arc::new(std::sync::Barrier::new(4));
    std::thread::scope(|scope| {
        let threads: Vec<_> = ["first", "second", "third", "fourth"]
            .into_iter()
            .map(|name| {
                let mut m = first.clone();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    m.enable_extension(name).unwrap();
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
    });
    first.refresh().unwrap();
    assert!(first.get_extensions().iter().all(|e| e.enabled));
    let mut stale = first.clone();
    first.remove_extension("first").unwrap();
    stale.disable_extension("second").unwrap();
    first.refresh().unwrap();
    assert_eq!(first.get_extensions().len(), 3);
    assert!(
        !first
            .get_extensions()
            .iter()
            .find(|e| e.name == "second")
            .unwrap()
            .enabled
    );
    // Source-independent immutable copies survive removal and can be registered again.
    let artifact = first.get_extensions()[0].artifacts[0].library_path.clone();
    first.remove_extension("second").unwrap();
    assert!(artifact.exists());
    for name in [
        "..",
        ".",
        "",
        "../outside",
        "a/b",
        "a\\b",
        "CON",
        "nul",
        "Com1",
        "LPT9.dll",
        "trailing.",
    ] {
        assert_eq!(
            first.remove_extension(name).unwrap_err().kind,
            ExtensionErrorKind::InvalidMetadata
        );
    }
}

#[tokio::test]
async fn download_failures_and_concurrent_registry_updates() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let registry = dir.path().join("registry.json");
    let mut server = mockito::Server::new_async().await;
    let bytes = b"downloaded bytes";
    let registry_update = registry.clone();
    let request = server
        .mock("GET", "/plugin.so")
        .with_chunked_body(move |writer| {
            JsonStore::new(&registry_update, "Extension registry")
                .update::<Value, _>(|v| {
                    *v = json!({"registry_version": 1,"extensions": []});
                    Ok(())
                })
                .unwrap();
            writer.write_all(bytes)
        })
        .create_async()
        .await;
    let path = manifest(
        dir.path(),
        "downloaded",
        "1.0.0",
        &format!("{}/plugin.so", server.url()),
        bytes,
    );
    m.add_extension(&path).await.unwrap();
    request.assert_async().await;
    let before = fs::read(&registry).unwrap();
    for status in [503, 200] {
        let request = server
            .mock("GET", "/bad.so")
            .with_status(status)
            .with_body("wrong bytes")
            .create_async()
            .await;
        let path = manifest(
            dir.path(),
            "bad",
            "1.0.0",
            &format!("{}/bad.so", server.url()),
            b"expected bytes",
        );
        let error = m.add_extension(path).await.unwrap_err();
        if status == 200 {
            assert_eq!(error.kind, ExtensionErrorKind::DigestMismatch);
        } else {
            use std::error::Error;
            let source = error
                .source()
                .unwrap()
                .downcast_ref::<ga4gh_sdk::Error>()
                .unwrap();
            assert!(
                matches!(source,ga4gh_sdk::Error::Transport(e) if e.status().unwrap().as_u16()==503)
            );
        }
        assert_eq!(before, fs::read(&registry).unwrap());
        request.assert_async().await;
    }
}

#[tokio::test]
async fn explicit_resolution_negotiates_compatible_releases_without_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let first = install_bytes(&mut m, dir.path(), "resolver", b"first bytes").await;
    m.enable_extension("resolver").unwrap();
    let old = m.resolve("resolver", None, None, &caps()).unwrap();
    assert_eq!(old.capabilities()["test"], 2);
    let second = manifest(dir.path(), "resolver", "2.0.0", "other", b"second bytes");
    fs::write(dir.path().join("other"), b"second bytes").unwrap();
    m.add_extension(&second).await.unwrap();
    assert_eq!(
        m.resolve("resolver", None, None, &caps())
            .unwrap()
            .artifact()
            .version
            .to_string(),
        "2.0.0"
    );
    assert_eq!(
        m.resolve("resolver", Some("^1"), None, &caps())
            .unwrap()
            .artifact(),
        old.artifact()
    );
    // A same-version different digest cannot be selected by version alone.
    fs::write(dir.path().join("other"), b"repacked").unwrap();
    edit(&second, |v| v["digest"] = json!(digest(b"repacked")));
    m.add_extension(second).await.unwrap();
    assert_eq!(
        m.resolve("resolver", Some("^2"), None, &caps())
            .unwrap_err()
            .kind,
        ExtensionErrorKind::AmbiguousArtifact
    );
    assert_eq!(
        m.resolve(
            "resolver",
            Some("^2"),
            Some(&digest(b"second bytes")),
            &caps()
        )
        .unwrap()
        .artifact()
        .digest,
        digest(b"second bytes")
    );
    let incompatible = manifest(dir.path(), "resolver", "3.0.0", "other", b"repacked");
    edit(&incompatible, |v| {
        v["target"] = json!("other-os-architecture")
    });
    // The same bytes cannot be registered with conflicting metadata.
    assert!(m.add_extension(incompatible).await.is_err());
    assert_eq!(
        m.resolve(
            "resolver",
            None,
            None,
            &serde_json::from_value(json!({"missing": [1]})).unwrap()
        )
        .unwrap_err()
        .kind,
        ExtensionErrorKind::MissingCapability
    );
    assert_eq!(
        m.resolve("resolver", Some("nonsense"), None, &caps())
            .unwrap_err()
            .kind,
        ExtensionErrorKind::InvalidMetadata
    );
    // Old resolution is not re-resolved even though selection has changed.
    let error = unsafe { m.create_instance(&old, b"{}") }.unwrap_err();
    assert_eq!(error.kind, ExtensionErrorKind::LoadFailed);
    assert!(first.exists());
}

#[tokio::test]
async fn malformed_legacy_and_required_configuration_are_explicit_and_redacted() {
    let dir = tempfile::tempdir().unwrap();
    let registry = dir.path().join("registry.json");
    fs::write(&registry, r#"{"extensions": []}"#).unwrap();
    assert_eq!(
        manager_error(dir.path()),
        ExtensionErrorKind::UnsupportedManifest
    );
    fs::write(
        &registry,
        r#"{"registry_version": 1,"extensions": [{"name": {"secret": "PRIVATE"}}]}"#,
    )
    .unwrap();
    let error = ExtensionManager::open(&registry).unwrap_err();
    assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    fs::remove_file(&registry).unwrap();
    let m = manager(dir.path());
    let mut config = Configuration::default();
    config.extra.insert(
        "extensions".into(),
        json!([
            {
                "name": "missing",
                "required": true,
                "configuration": {
                    "secret": "PRIVATE"
                },
                "capabilities": {
                    "test": [
                        1
                    ]
                }
            }
        ]),
    );
    assert_eq!(
        unsafe { m.from_configuration(&config) }.unwrap_err().kind,
        ExtensionErrorKind::Missing
    );
    config.extra["extensions"][0]["required"] = json!(false);
    config.extra["extensions"][0]
        .as_object_mut()
        .unwrap()
        .remove("configuration");
    let entries = config
        .extra_as::<ga4gh_sdk::utils::configuration::ServiceExtensionsConfiguration>("extensions")
        .unwrap()
        .unwrap();
    assert_eq!(entries.get_extension_config("missing"), Some(&json!({})));
    assert!(unsafe { m.from_configuration(&config) }.unwrap().is_empty());
    config.extra["extensions"] = json!({"secret": "PRIVATE"});
    let error = unsafe { m.from_configuration(&config) }.unwrap_err();
    assert!(!format!("{error:?} {error}").contains("PRIVATE"));
    assert!(!registry.exists());
    let mut present = manager(dir.path());
    install_bytes(&mut present, dir.path(), "present", b"not executable").await;
    present.enable_extension("present").unwrap();
    config.extra["extensions"] =
        json!([{"name": "present","required": false,"version": "^2","configuration": {}}]);
    assert_eq!(
        unsafe { present.from_configuration(&config) }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::NoMatchingArtifact
    );
    config.extra["extensions"] = json!([{"name": "present","required": false,"capabilities": {"absent": [1]},"configuration": {}}]);
    assert_eq!(
        unsafe { present.from_configuration(&config) }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::MissingCapability
    );
    let path = manifest(dir.path(), "unsupported", "1.0.0", "unused", b"bytes");
    edit(&path, |v| v["manifest_version"] = json!(2));
    assert_eq!(
        Extension::from_file(path).unwrap_err().kind,
        ExtensionErrorKind::UnsupportedManifest
    );
}

fn manager_error(dir: &Path) -> ExtensionErrorKind {
    ExtensionManager::open(dir.join("registry.json"))
        .unwrap_err()
        .kind
}

#[tokio::test]
async fn downloads_bound_resources_redact_sources_and_never_follow_redirects() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let mut server = mockito::Server::new_async().await;
    let bytes = b"verified artifact";
    let request = server
        .mock("GET", "/artifact?token=PRIVATE_TOKEN")
        .with_body(bytes)
        .create_async()
        .await;
    let source =
        format!("{}/artifact?token=PRIVATE_TOKEN", server.url()).replacen("http:", "HtTp:", 1);
    let path = manifest(dir.path(), "download-policy", "1.0.0", &source, bytes);
    assert!(!format!("{:?}", Extension::from_file(&path).unwrap()).contains("PRIVATE_TOKEN"));
    m.add_extension(&path).await.unwrap();
    request.assert_async().await;
    let installed = &m.get_extensions()[0].artifacts[0];
    assert_eq!(
        installed.manifest.path,
        installed
            .library_path
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
    );
    for path in [
        dir.path().join("registry.json"),
        installed
            .library_path
            .parent()
            .unwrap()
            .join("manifest.json"),
    ] {
        assert!(!fs::read_to_string(path).unwrap().contains("PRIVATE_TOKEN"));
    }
    assert!(!format!("{m:?}").contains("PRIVATE_TOKEN"));
    let before = fs::read(dir.path().join("registry.json")).unwrap();
    let destination = server
        .mock("GET", "/never-requested")
        .expect(0)
        .create_async()
        .await;
    let redirect = server
        .mock("GET", "/redirect")
        .with_status(302)
        .with_header("location", &format!("{}/never-requested", server.url()))
        .create_async()
        .await;
    let path = manifest(
        dir.path(),
        "redirected",
        "1.0.0",
        &format!("{}/redirect", server.url()),
        bytes,
    );
    assert_eq!(
        m.add_extension(path).await.unwrap_err().kind,
        ExtensionErrorKind::Storage
    );
    redirect.assert_async().await;
    destination.assert_async().await;
    // A raw response retains its declared length (mockito rewrites it to body size).
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let responder = std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = std::io::BufReader::new(&mut socket);
        let mut request = String::new();
        loop {
            assert!(reader.read_line(&mut request).unwrap() > 0);
            assert!(request.len() <= 16384);
            if request.ends_with("\r\n\r\n") {
                break;
            }
        }
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            MAX_ARTIFACT_BYTES + 1
        )
        .unwrap();
    });
    let path = manifest(
        dir.path(),
        "too-large",
        "1.0.0",
        &format!("http://{address}/large"),
        bytes,
    );
    assert_eq!(
        m.add_extension(path).await.unwrap_err().kind,
        ExtensionErrorKind::ArtifactTooLarge
    );
    responder.join().unwrap();
    let source = dir.path().join("sparse-library");
    fs::File::create(&source)
        .unwrap()
        .set_len(MAX_ARTIFACT_BYTES + 1)
        .unwrap();
    let path = manifest(
        dir.path(),
        "too-large",
        "1.0.0",
        source.to_str().unwrap(),
        bytes,
    );
    assert_eq!(
        m.add_extension(path).await.unwrap_err().kind,
        ExtensionErrorKind::ArtifactTooLarge
    );
    assert_eq!(before, fs::read(dir.path().join("registry.json")).unwrap());
}

#[cfg(unix)]
#[tokio::test]
async fn registry_aliases_share_updates_and_preserve_symlinks() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let mut first = manager(dir.path());
    install_bytes(&mut first, dir.path(), "aliased", b"not executable").await;
    let alias = dir.path().join("alias.json");
    symlink(dir.path().join("registry.json"), &alias).unwrap();
    let parent_alias = dir.path().join("parent-alias");
    symlink(dir.path(), &parent_alias).unwrap();
    let mut via_file = ExtensionManager::open(&alias).unwrap();
    let mut via_parent = ExtensionManager::open(parent_alias.join("registry.json")).unwrap();
    via_file.enable_extension("aliased").unwrap();
    first.refresh().unwrap();
    assert!(first.get_extensions()[0].enabled);
    assert!(alias.is_symlink());
    via_parent.disable_extension("aliased").unwrap();
    via_file.refresh().unwrap();
    assert!(!via_file.get_extensions()[0].enabled);
    // Missing descendants and .. spellings get the same registry identity too.
    let missing = parent_alias.join("new/../new/registry.json");
    let mut a = ExtensionManager::open(missing).unwrap();
    let mut b = ExtensionManager::open(dir.path().join("new/registry.json")).unwrap();
    install_bytes(&mut a, dir.path(), "new-alias", b"not executable").await;
    b.enable_extension("new-alias").unwrap();
    a.refresh().unwrap();
    assert!(a.get_extensions()[0].enabled);
}

#[tokio::test]
async fn invalid_targets_and_resolver_failure_precedence_are_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let path = manifest(dir.path(), "metadata", "1.0.0", "source", b"bytes");
    for target in ["bad\u{1b}[2J", "bad\nline", "bad\ttab"] {
        edit(&path, |v| v["target"] = json!(target));
        assert_eq!(
            Extension::from_file(&path).unwrap_err().kind,
            ExtensionErrorKind::InvalidMetadata
        );
    }
    for (version, bytes, abi, target, capabilities) in [
        (
            "1.0.0",
            b"one".as_slice(),
            99,
            HOST_TARGET,
            json!({"test": [1]}),
        ),
        (
            "2.0.0",
            b"two".as_slice(),
            1,
            "different-target",
            json!({"test": [1]}),
        ),
        ("3.0.0", b"three".as_slice(), 1, HOST_TARGET, json!({})),
    ] {
        fs::write(dir.path().join("source"), bytes).unwrap();
        let path = manifest(dir.path(), "failure-order", version, "source", bytes);
        edit(&path, |v| {
            v["abi_version"] = json!(abi);
            v["target"] = json!(target);
            v["capabilities"] = capabilities;
        });
        m.add_extension(path).await.unwrap();
    }
    m.enable_extension("failure-order").unwrap();
    for _ in 0..2 {
        assert_eq!(
            m.resolve("failure-order", Some("*"), None, &caps())
                .unwrap_err()
                .kind,
            ExtensionErrorKind::MissingCapability
        );
        assert_eq!(
            m.resolve("failure-order", Some("<3"), None, &caps())
                .unwrap_err()
                .kind,
            ExtensionErrorKind::TargetMismatch
        );
        assert_eq!(
            m.resolve("failure-order", Some("<2"), None, &caps())
                .unwrap_err()
                .kind,
            ExtensionErrorKind::UnsupportedAbi
        );
        edit(&dir.path().join("registry.json"), |v| {
            v["extensions"][0]["artifacts"]
                .as_array_mut()
                .unwrap()
                .reverse()
        });
    }
}

#[tokio::test]
async fn native_abi_lifetimes_instances_admission_and_process_versions() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<ExtensionManager>();
    send_sync::<ExtensionInstance>();
    send_sync::<ExtensionMethod>();
    send_sync::<LoadedModule>();
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let library = compile(
        dir.path(),
        "fixture",
        "1.0.0",
        &[(
            "TABLE_SIZE",
            "(sizeof(Ga4ghExtensionApiV1)+sizeof(uint64_t))",
        )],
    );
    let path = manifest(
        dir.path(),
        "fixture",
        "1.0.0",
        library.to_str().unwrap(),
        &fs::read(&library).unwrap(),
    );
    m.add_extension(&path).await.unwrap();
    m.enable_extension("fixture").unwrap();
    let resolved = m.resolve("fixture", None, None, &caps()).unwrap();
    let other = m.clone();
    let resolved_thread = resolved.clone();
    let instance = std::thread::spawn(move || {
        unsafe { other.create_instance(&resolved_thread, b"first config") }.unwrap()
    })
    .join()
    .unwrap();
    let second = unsafe { m.create_instance(&resolved, b"second config") }.unwrap();
    assert_eq!(
        instance.invoke("test", "configuration", b"").unwrap(),
        b"first config"
    );
    assert_eq!(
        second.invoke("test", "configuration", b"").unwrap(),
        b"second config"
    );
    assert_eq!(
        instance.invoke("absent", "echo", b"").unwrap_err().kind,
        ExtensionErrorKind::MissingCapability
    );
    let failed = unsafe { m.create_instance(&resolved, b"fail SECRET_CONFIG") }.unwrap_err();
    assert_eq!(failed.kind, ExtensionErrorKind::InitializationFailed);
    assert_eq!(failed.private_payload(), b"fail SECRET_CONFIG");
    assert!(!format!("{failed:?} {failed}").contains("SECRET_CONFIG"));
    let failed = instance
        .invoke("test", "fail", b"SECRET_RESULT")
        .unwrap_err();
    assert_eq!(failed.status, Some(8));
    assert_eq!(failed.private_payload(), b"SECRET_RESULT");
    assert!(!format!("{failed:?} {failed}").contains("SECRET_RESULT"));
    for operation in ["echo", "fail"] {
        let oversized = instance
            .invoke_bounded("test", operation, b"private", 4)
            .unwrap_err();
        assert_eq!(oversized.kind, ExtensionErrorKind::InvalidApi);
        assert!(oversized.private_payload().is_empty());
    }
    let dropped = dir.path().join("destroyed");
    instance
        .invoke("test", "set_drop_path", &input(&dropped))
        .unwrap();
    // Concurrent distinct instances enter native code before either is released.
    let prefixes = [dir.path().join("one"), dir.path().join("two")];
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            instance
                .invoke("test", "wait", &input(&prefixes[0]))
                .unwrap()
        });
        let second_call =
            scope.spawn(|| second.invoke("test", "wait", &input(&prefixes[1])).unwrap());
        for prefix in &prefixes {
            wait(&prefix.with_extension("entered"));
        }
        // Registry lock is not held across native invocation; stale manager must
        // read this new disable on its next admission.
        let stale = m.clone();
        m.disable_extension("fixture").unwrap();
        assert_eq!(
            unsafe { stale.create_instance(&resolved, b"{}") }
                .unwrap_err()
                .kind,
            ExtensionErrorKind::Disabled
        );
        for prefix in &prefixes {
            fs::write(prefix.with_extension("release"), "").unwrap();
        }
        first.join().unwrap();
        second_call.join().unwrap();
    });
    m.enable_extension("fixture").unwrap();
    // One instance serializes operations through copying/freeing the result.
    let blocked = dir.path().join("serialized");
    let touched = dir.path().join("touch");
    std::thread::scope(|scope| {
        let first = scope.spawn(|| instance.invoke("test", "wait", &input(&blocked)).unwrap());
        wait(&blocked.with_extension("entered"));
        let started = Arc::new(std::sync::Barrier::new(2));
        let ready = started.clone();
        let instance_ref = &instance;
        let touched_ref = &touched;
        let second_call = scope.spawn(move || {
            ready.wait();
            instance_ref
                .invoke("test", "touch", &input(touched_ref))
                .unwrap()
        });
        started.wait();
        std::thread::sleep(Duration::from_millis(30));
        assert!(!touched.exists());
        fs::write(blocked.with_extension("release"), "").unwrap();
        first.join().unwrap();
        second_call.join().unwrap();
        assert!(touched.exists());
    });
    // Service configuration uses shared Figment sources; only instance JSON goes in.
    let config = Configuration::from_provider(figment::providers::Serialized::defaults(json!({
        "base_path": "https://service.example/",
        "bearer_access_token": "HOST_SECRET",
        "extensions": [
            {
                "name": "fixture",
                "required": true,
                "capabilities": {
                    "test": [
                        1
                    ]
                },
                "configuration": {
                    "client": 3
                }
            }
        ]
    })))
    .unwrap();
    let selected = unsafe { m.from_configuration(&config) }.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&selected[0].invoke("test", "configuration", b"").unwrap())
            .unwrap(),
        json!({"client": 3})
    );
    drop(selected);
    // Installing/selecting v2 changes no active library and cannot retarget v1.
    let v2 = compile(dir.path(), "fixture", "2.0.0", &[]);
    let manifest_v2 = manifest(
        dir.path(),
        "fixture",
        "2.0.0",
        v2.to_str().unwrap(),
        &fs::read(&v2).unwrap(),
    );
    m.add_extension(&manifest_v2).await.unwrap();
    assert_eq!(
        unsafe { m.create_instance(&resolved, b"old exact") }
            .unwrap()
            .invoke("test", "configuration", b"")
            .unwrap(),
        b"old exact"
    );
    let updated = m.resolve("fixture", None, None, &caps()).unwrap();
    let restart = unsafe { m.create_instance(&updated, b"new") }.unwrap_err();
    assert_eq!(restart.kind, ExtensionErrorKind::RestartRequired);
    assert_eq!(restart.current.unwrap().digest, resolved.artifact().digest);
    // A fresh process loads the newly selected artifact.
    let result = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "fresh_process_load", "--nocapture"])
        .env("GA4GH_FIXTURE_REGISTRY", dir.path().join("registry.json"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{} {}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    // A different pinned artifact must report RestartRequired before touching disk.
    fs::remove_file(updated.library_path()).unwrap();
    assert_eq!(
        unsafe { m.load(&updated) }.unwrap_err().kind,
        ExtensionErrorKind::RestartRequired
    );
    fs::copy(&v2, updated.library_path()).unwrap();
    let method = instance.method("test", "echo").unwrap();
    let stale = m.clone();
    m.remove_extension("fixture").unwrap();
    assert!(resolved.library_path().exists());
    assert!(updated.library_path().exists());
    assert_eq!(
        unsafe { stale.create_instance(&resolved, b"{}") }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::Missing
    );
    drop(instance);
    drop(second);
    drop(stale);
    drop(m);
    assert!(!dropped.exists());
    assert_eq!(method.call(b"still callable").unwrap(), b"still callable");
    std::thread::spawn(move || drop(method)).join().unwrap();
    assert_eq!(fs::read_to_string(dropped).unwrap(), "destroyed");
    // Pin remains after the last runtime handle is released.
    let mut m = manager(dir.path());
    m.add_extension(manifest_v2).await.unwrap();
    m.enable_extension("fixture").unwrap();
    let updated = m.resolve("fixture", None, None, &caps()).unwrap();
    assert_eq!(
        unsafe { m.create_instance(&updated, b"{}") }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::RestartRequired
    );
}

#[test]
fn fresh_process_load() {
    let Some(registry) = std::env::var_os("GA4GH_FIXTURE_REGISTRY") else {
        return;
    };
    let m = ExtensionManager::open(registry).unwrap();
    let resolved = m.resolve("fixture", None, None, &caps()).unwrap();
    let instance = unsafe { m.create_instance(&resolved, b"child") }.unwrap();
    assert_eq!(instance.invoke("test", "echo", b"child").unwrap(), b"child");
}

#[tokio::test]
async fn api_header_checks_and_failed_validation_keep_the_process_pin() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    for (name, extra, kind) in [
        (
            "badabi",
            vec![("TABLE_ABI", "9")],
            ExtensionErrorKind::UnsupportedAbi,
        ),
        (
            "short",
            vec![("TABLE_SIZE", "sizeof(Ga4ghHeader)")],
            ExtensionErrorKind::InvalidApi,
        ),
    ] {
        let library = compile(dir.path(), name, "1.0.0", &extra);
        let path = manifest(
            dir.path(),
            name,
            "1.0.0",
            library.to_str().unwrap(),
            &fs::read(&library).unwrap(),
        );
        m.add_extension(path).await.unwrap();
        m.enable_extension(name).unwrap();
        let resolved = m.resolve(name, None, None, &caps()).unwrap();
        assert_eq!(
            unsafe { m.create_instance(&resolved, b"{}") }
                .unwrap_err()
                .kind,
            kind
        );
        let fixed = compile(dir.path(), name, "1.0.0", &[]);
        let path = manifest(
            dir.path(),
            name,
            "1.0.0",
            fixed.to_str().unwrap(),
            &fs::read(&fixed).unwrap(),
        );
        m.add_extension(path).await.unwrap();
        let resolved = m.resolve(name, None, None, &caps()).unwrap();
        assert_eq!(
            unsafe { m.create_instance(&resolved, b"{}") }
                .unwrap_err()
                .kind,
            ExtensionErrorKind::RestartRequired
        );
    }
    let library = compile(dir.path(), "descriptor", "1.0.0", &[]);
    let path = manifest(
        dir.path(),
        "descriptor",
        "1.0.0",
        library.to_str().unwrap(),
        &fs::read(&library).unwrap(),
    );
    edit(&path, |v| v["capabilities"] = json!({"invented": [1]}));
    m.add_extension(path).await.unwrap();
    m.enable_extension("descriptor").unwrap();
    let resolved = m
        .resolve("descriptor", None, None, &Capabilities::new())
        .unwrap();
    assert_eq!(
        unsafe { m.load(&resolved) }.unwrap_err().kind,
        ExtensionErrorKind::InvalidApi
    );
}

#[tokio::test]
async fn admitted_creation_finishes_after_disable_without_a_registry_lock() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let prefix = dir.path().join("admission");
    let quoted = serde_json::to_string(&prefix.to_str().unwrap().replace('\\', "/")).unwrap();
    let library = compile(
        dir.path(),
        "admission",
        "1.0.0",
        &[("ENTRY_WAIT_PREFIX", &quoted)],
    );
    let manifest = manifest(
        dir.path(),
        "admission",
        "1.0.0",
        library.to_str().unwrap(),
        &fs::read(&library).unwrap(),
    );
    m.add_extension(manifest).await.unwrap();
    m.enable_extension("admission").unwrap();
    let resolved = m.resolve("admission", None, None, &caps()).unwrap();
    let stale = m.clone();
    let loading = m.clone();
    let admitted = resolved.clone();
    let thread =
        std::thread::spawn(move || unsafe { loading.create_instance(&admitted, b"admitted") });
    wait(&prefix.with_extension("entered"));
    // API discovery is blocked in native code. This mutation must still complete.
    m.disable_extension("admission").unwrap();
    assert_eq!(
        unsafe { stale.create_instance(&resolved, b"late") }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::Disabled
    );
    m.remove_extension("admission").unwrap();
    fs::write(prefix.with_extension("release"), "").unwrap();
    let instance = thread.join().unwrap().unwrap();
    assert_eq!(
        instance.invoke("test", "configuration", b"").unwrap(),
        b"admitted"
    );
    assert_eq!(
        unsafe { stale.create_instance(&resolved, b"later") }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::Missing
    );
}

#[tokio::test]
async fn concurrent_artifacts_have_one_process_winner_across_managers() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let mut resolved = vec![];
    for version in ["1.0.0", "2.0.0"] {
        let library = compile(dir.path(), "race", version, &[]);
        let path = manifest(
            dir.path(),
            "race",
            version,
            library.to_str().unwrap(),
            &fs::read(&library).unwrap(),
        );
        m.add_extension(path).await.unwrap();
        m.enable_extension("race").unwrap();
        resolved.push(m.resolve("race", None, None, &caps()).unwrap());
    }
    let ready = std::sync::Barrier::new(2);
    std::thread::scope(|scope| {
        let threads: Vec<_> = resolved
            .iter()
            .map(|resolved| {
                let m = &m;
                let ready = &ready;
                scope.spawn(move || {
                    ready.wait();
                    unsafe { m.create_instance(resolved, b"race") }
                })
            })
            .collect();
        let outcomes: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(outcomes.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(
            outcomes.into_iter().find_map(Result::err).unwrap().kind,
            ExtensionErrorKind::RestartRequired
        );
    });
    // A different registry and installation root cannot bypass that process pin.
    let elsewhere = tempfile::tempdir().unwrap();
    let mut other = manager(elsewhere.path());
    let library = compile(elsewhere.path(), "race", "3.0.0", &[]);
    let path = manifest(
        elsewhere.path(),
        "race",
        "3.0.0",
        library.to_str().unwrap(),
        &fs::read(&library).unwrap(),
    );
    other.add_extension(path).await.unwrap();
    other.enable_extension("race").unwrap();
    let resolved = other.resolve("race", None, None, &caps()).unwrap();
    assert_eq!(
        unsafe { other.create_instance(&resolved, b"{}") }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::RestartRequired
    );
}

#[tokio::test]
async fn load_checks_target_abi_and_digest_before_executing_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut m = manager(dir.path());
    let path = install_bytes(&mut m, dir.path(), "preflight", b"not executable").await;
    m.enable_extension("preflight").unwrap();
    let resolved = m.resolve("preflight", None, None, &caps()).unwrap();
    fs::write(resolved.library_path(), b"tampered").unwrap();
    assert_eq!(
        unsafe { m.create_instance(&resolved, b"{}") }
            .unwrap_err()
            .kind,
        ExtensionErrorKind::DigestMismatch
    );
    m.remove_extension("preflight").unwrap();
    // Independent bytes avoid a conflicting re-registration of the original digest.
    fs::write(dir.path().join("source-target"), b"target bytes").unwrap();
    let target = manifest(
        dir.path(),
        "target",
        "1.0.0",
        "source-target",
        b"target bytes",
    );
    edit(&target, |v| v["target"] = json!("other-target"));
    m.add_extension(target).await.unwrap();
    m.enable_extension("target").unwrap();
    assert_eq!(
        m.resolve("target", None, None, &caps()).unwrap_err().kind,
        ExtensionErrorKind::TargetMismatch
    );
    edit(&path, |v| {
        v["name"] = json!("unsupported-abi");
        v["abi_version"] = json!(99);
    });
    m.add_extension(path).await.unwrap();
    m.enable_extension("unsupported-abi").unwrap();
    assert_eq!(
        m.resolve("unsupported-abi", None, None, &caps())
            .unwrap_err()
            .kind,
        ExtensionErrorKind::UnsupportedAbi
    );
}
