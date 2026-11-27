//! Extension management uses caller-owned paths and does not execute libraries.

use ga4gh_cli::CliContext;
use serde_json::json;
use std::fs;
use std::process::{Command, ExitCode};

#[tokio::test]
async fn context_manages_extensions_without_a_service_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let context =
        CliContext::new("custom-cli", "1").with_config_path(dir.path().join("custom-config.json"));
    let registry = dir.path().join("extensions.json");
    fs::write(&registry, json!({"registry_version":1,"extensions":[{"name":"example","selected":"0".repeat(128),"artifacts":[{"manifest":{"manifest_version":1,"name":"example","version":"1.0.0","target":"not-a-loadable-target","abi_version":1,"digest":"0".repeat(128),"capabilities":{},"path":"missing"},"library_path":dir.path().join("missing.so")}],"enabled":false}]}).to_string()).unwrap();
    fs::write(dir.path().join("missing.so"), b"not executable").unwrap();
    for (operation, enabled) in [("enable", true), ("disable", false)] {
        let matches = context
            .command()
            .try_get_matches_from(["custom-cli", "extension", operation, "example"])
            .unwrap();
        let output = context.dispatch(&matches).await.unwrap().unwrap();
        assert_eq!(output.exit_code, ExitCode::SUCCESS);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
        assert_eq!(persisted["extensions"][0]["enabled"], enabled);
    }
    let mut stored: serde_json::Value =
        serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    let mut second = stored["extensions"][0]["artifacts"][0].clone();
    second["manifest"]["version"] = json!("2.0.0");
    second["manifest"]["digest"] = json!("1".repeat(128));
    second["library_path"] = json!(dir.path().join("another-missing.so"));
    stored["extensions"][0]["artifacts"]
        .as_array_mut()
        .unwrap()
        .push(second);
    fs::write(&registry, serde_json::to_vec(&stored).unwrap()).unwrap();
    let matches = context
        .command()
        .try_get_matches_from(["custom-cli", "extension", "list"])
        .unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    assert_eq!(
        context
            .dispatch_to(&matches, &mut stdout, &mut stderr)
            .await
            .unwrap(),
        Some(ExitCode::SUCCESS)
    );
    let listed = String::from_utf8(stdout).unwrap();
    assert!(listed.contains("example"));
    assert!(listed.contains("1.0.0"));
    assert!(listed.contains("2.0.0"));
    assert!(listed.contains(&"1".repeat(128)));
    assert!(stderr.is_empty());
    let digest = "0".repeat(128);
    let matches = context
        .command()
        .try_get_matches_from(["custom-cli", "extension", "select", "example", &digest])
        .unwrap();
    assert!(context
        .dispatch(&matches)
        .await
        .unwrap()
        .unwrap()
        .stdout
        .is_empty());
    let matches = context
        .command()
        .try_get_matches_from(["custom-cli", "extension", "remove", "example"])
        .unwrap();
    let output = context.dispatch(&matches).await.unwrap().unwrap();
    assert!(output.stdout.contains("artifact files retained"));
    assert!(output.stdout.contains("not reclaimed"));
    assert_eq!(
        fs::read(dir.path().join("missing.so")).unwrap(),
        b"not executable"
    );
    assert!(!dir.path().join("custom-config.json").exists());
}

#[test]
fn binary_lists_without_creating_config_and_rejects_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
            .env("HOME", dir.path())
            .env_remove("GA4GH_CLI_PROFILE")
            .args(args)
            .output()
            .unwrap()
    };
    let listed = run(&["extension", "list"]);
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("Extension Name"));
    assert!(listed.stderr.is_empty());
    assert!(!dir.path().join(".ga4gh-cli").exists());
    let config_dir = dir.path().join(".ga4gh-cli");
    fs::create_dir(&config_dir).unwrap();
    fs::write(config_dir.join("keep"), "keep").unwrap();
    let rejected = run(&["extension", "remove", ".."]);
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("InvalidMetadata"));
    assert_eq!(fs::read_to_string(config_dir.join("keep")).unwrap(), "keep");
}
