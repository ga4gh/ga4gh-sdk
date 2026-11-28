//! Public CLI command discovery, immutable parsing and lazy execution of a non-workflow provider.
#![cfg(unix)]

use ga4gh_cli::CliContext;
use ga4gh_sdk::utils::{extension::HOST_TARGET, extension_manager::ExtensionManager};
use serde_json::{json, Value};
use sha2::{Digest, Sha512};
use std::{
    fs,
    path::Path,
    process::{Command, ExitCode},
};

fn descriptor(group: &str) -> Value {
    json!({"name":"word-provider", "version":"1.0.0", "target":HOST_TARGET,"abi_version":1,
        "capabilities":{"ga4gh.cli.commands":[1]}, "capability_metadata":{"ga4gh.cli.commands":{"1":[
        {"name":group,"about":"Word tools","commands":[{"name":"echo","arguments":[{"name":"word","required":true}]}]}
    ]}}})
}

fn compile(dir: &Path, descriptor: &Value) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    fs::write(
        dir.join("fixture.h"),
        format!(
            "#define DESCRIPTOR {}\n#define MARKER {}\n",
            json!(descriptor.to_string()),
            json!(dir.join("loaded"))
        ),
    )
    .unwrap();
    assert!(Command::new("cc")
        .args(["-shared", "-fPIC", "-I"])
        .arg(dir)
        .arg("-I")
        .arg(root.join("../extension-abi/include"))
        .arg(root.join("tests/fixtures/command-provider.c"))
        .arg("-o")
        .arg(dir.join("provider"))
        .status()
        .unwrap()
        .success());
}

async fn add(
    manager: &mut ExtensionManager,
    dir: &Path,
    mut manifest: Value,
) -> Result<(), ga4gh_sdk::utils::extension::ExtensionError> {
    manifest["manifest_version"] = json!(1);
    manifest["path"] = json!("provider");
    manifest["digest"] = json!(Sha512::digest(fs::read(dir.join("provider")).unwrap())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>());
    fs::write(dir.join("manifest.json"), manifest.to_string()).unwrap();
    manager.add_extension(dir.join("manifest.json")).await
}

#[tokio::test]
async fn help_management_and_parsing_do_not_load_and_execution_uses_the_parsed_artifact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    let context = CliContext::default().with_config_path(path.join("unused.json"));
    let parse = |args: &[&str]| context.parse_from(context.command(), args.iter().copied());
    let args = ["cli", "words", "echo", "hello", "--profile", "remote"];
    assert!(parse(&args)
        .err()
        .unwrap()
        .to_string()
        .contains("install and enable"));
    let mut manager = ExtensionManager::open(path.join("extensions.json")).unwrap();
    let first = descriptor("words");
    compile(path, &first);
    add(&mut manager, path, first.clone()).await.unwrap();
    assert!(add(&mut manager, path, descriptor("changed"))
        .await
        .is_err());
    let disabled = parse(&args)
        .unwrap()
        .dispatch_extension(None)
        .await
        .err()
        .unwrap();
    assert!(disabled.to_string().contains("Disabled"));
    let enable = parse(&["cli", "extension", "enable", "word-provider"]).unwrap();
    assert_eq!(
        enable
            .dispatch_to(&mut Vec::new(), &mut Vec::new())
            .await
            .unwrap(),
        Some(ExitCode::SUCCESS)
    );
    assert!(parse(&["cli", "--help"])
        .err()
        .unwrap()
        .to_string()
        .contains("Word tools"));
    assert!(parse(&["cli", "words", "echo", "--help"])
        .err()
        .unwrap()
        .to_string()
        .contains("<word>"));
    let parsed = parse(&args).unwrap();
    let failure = parse(&["cli", "words", "echo", "private"]).unwrap();
    let mut second = first;
    second["target"] = json!("not-this-target");
    fs::write(path.join("provider"), b"incompatible second artifact").unwrap();
    add(&mut manager, path, second).await.unwrap();
    assert!(parse(&args)
        .unwrap()
        .dispatch_extension(None)
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("TargetMismatch"));
    assert!(!path.join("loaded").exists());
    let output = parsed.dispatch_extension(None).await.unwrap().unwrap();
    assert!(path.join("loaded").exists());
    assert_eq!(
        (
            output.stdout.as_str(),
            output.stderr.as_str(),
            output.exit_code
        ),
        (r"ok\u{1b}", "warning\n", ExitCode::from(3))
    );
    let error = failure.dispatch_extension(None).await.err().unwrap();
    assert!(format!("{error:?}").contains("InvocationFailed"));
    assert!(!format!("{error:?}").contains("PRIVATE"));
    manager.disable_extension("word-provider").unwrap();
    assert!(parsed
        .dispatch_extension(None)
        .await
        .err()
        .unwrap()
        .to_string()
        .contains("Disabled"));
}

#[tokio::test]
async fn collisions_fail_without_loading_and_management_can_recover() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path();
    fs::write(path.join("provider"), b"not executable").unwrap();
    let context = CliContext::default().with_config_path(path.join("unused.json"));
    let mut manager = ExtensionManager::open(path.join("extensions.json")).unwrap();
    let parse = || {
        context
            .parse_from(context.command(), ["cli", "--help"])
            .err()
            .unwrap()
            .to_string()
    };
    add(&mut manager, path, descriptor("words")).await.unwrap();
    manager.enable_extension("word-provider").unwrap();
    for name in ["words", "tes"] {
        let mut conflict = descriptor(name);
        conflict["name"] = json!(format!("collides-{name}"));
        add(&mut manager, path, conflict).await.unwrap();
        manager
            .enable_extension(&format!("collides-{name}"))
            .unwrap();
        assert!(parse().contains("collision"));
        let parsed = context
            .parse_from(
                context.command(),
                ["cli", "extension", "disable", &format!("collides-{name}")],
            )
            .unwrap();
        parsed
            .dispatch_to(&mut Vec::new(), &mut Vec::new())
            .await
            .unwrap();
        assert!(parse().contains("Word tools"));
    }
}
