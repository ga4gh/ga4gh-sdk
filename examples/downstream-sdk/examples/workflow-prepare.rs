use anyhow::{Context, Result};
use ga4gh_sdk::utils::extension_manager::ExtensionManager;
use ga4gh_workflow::{Identity, Pins, Workflow};
use std::{env, fs, path::Path};

fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let registry = args
        .next()
        .context("expected path to a registry containing a trusted, enabled workflow provider")?;
    let directory = args
        .next()
        .context("expected path to examples/sequence-qc")?;
    let directory = Path::new(&directory);
    let manager = ExtensionManager::open(registry)?;
    let client = unsafe { Workflow::open(&manager)? };
    let revision = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let pins = Pins {
        profile_revision: revision.into(),
        vocabulary_revision: revision.into(),
    };
    let request = serde_json::from_slice(&fs::read(directory.join("request.json"))?)?;
    let identity = Identity {
        workflow: "public-qc@1".into(),
        descriptor_digest: None,
    };
    let plan = client.prepare_file(&directory.join("profile.json"), &pins, &request, &identity)?;
    println!("{}", plan.record().fingerprint);
    Ok(plan.release()?)
}
