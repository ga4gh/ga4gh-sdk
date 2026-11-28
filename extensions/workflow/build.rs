use serde_json::{json, Value};
use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=commands.json");
    let commands: Value = serde_json::from_slice(include_bytes!("commands.json")).unwrap();
    let mut capabilities = json!({"ga4gh.cli.commands": [1]});
    if cfg!(feature = "workflow-api-preview") {
        capabilities["ga4gh.workflow"] = json!([1]);
    }
    let descriptor = json!({
        "name": env!("CARGO_PKG_NAME"), "version": env!("CARGO_PKG_VERSION"),
        "abi_version": 1, "target": env::var("TARGET").unwrap(),
        "capabilities": capabilities,
        "capability_metadata": {"ga4gh.cli.commands": {"1": commands}}
    });
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("descriptor.json"),
        descriptor.to_string(),
    )
    .unwrap();
}
