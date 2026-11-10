//! Smoke tests of the TRS client against public registries.
//!
//! They need network access, so they are ignored by default. Run them with
//! `cargo test -p ga4gh-lib --test trs_public_registries -- --ignored`.

use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
use ga4gh_sdk::utils::configuration::Configuration;
use url::Url;

fn plain(descriptor_type: DescriptorType) -> DescriptorTypeWithPlain {
    match descriptor_type {
        DescriptorType::Cwl => DescriptorTypeWithPlain::PlainCwl,
        DescriptorType::Wdl => DescriptorTypeWithPlain::PlainWdl,
        DescriptorType::Nfl => DescriptorTypeWithPlain::PlainNfl,
        DescriptorType::Galaxy => DescriptorTypeWithPlain::PlainGalaxy,
        DescriptorType::Smk => DescriptorTypeWithPlain::PlainSmk,
    }
}

async fn client(base: &str) -> TRS {
    TRS::new(&Configuration::new(Url::parse(base).unwrap()))
        .await
        .unwrap_or_else(|e| panic!("TRS client for {base}: {e}"))
}

#[tokio::test]
#[ignore = "needs network access to workflowhub.eu"]
async fn test_workflowhub_lists_tools_and_fetches_a_descriptor() {
    let trs = client("https://workflowhub.eu/ga4gh/trs/v2").await;
    let filter = ToolFilter {
        tool_class: Some("Workflow".to_string()),
        limit: Some(5),
        ..Default::default()
    };
    let tools = trs.list_tools(&filter).await.expect("WorkflowHub tools");
    assert!(!tools.is_empty());

    let (tool, version, descriptor_type) = tools
        .iter()
        .find_map(|tool| {
            tool.versions.iter().find_map(|version| {
                let descriptor_type = *version.descriptor_type.as_ref()?.first()?;
                Some((tool, version, descriptor_type))
            })
        })
        .expect("a tool version with a descriptor type");
    let descriptor = trs
        .get_descriptor(&tool.id, &version.id, plain(descriptor_type))
        .await
        .expect("plain descriptor");
    assert!(!descriptor.content.unwrap_or_default().is_empty());
}

#[tokio::test]
#[ignore = "needs network access to dockstore.org"]
async fn test_dockstore_fetches_a_workflow_by_encoded_id() {
    let trs = client("https://dockstore.org/api/ga4gh/trs/v2/").await;
    let id = "#workflow/github.com/denis-yuen/dockstore-whalesay";
    let tool = trs
        .get_tool(id)
        .await
        .expect("Dockstore tool by encoded ID");
    assert_eq!(tool.id, id);
    let versions = trs.list_versions(id).await.expect("Dockstore versions");
    // Dockstore's version_id path parameter is the version's name; its
    // `ToolVersion.id` is `<tool ID>:<name>`.
    let version = versions
        .first()
        .and_then(|version| version.name.as_deref())
        .expect("a named version");

    let descriptor = trs
        .get_descriptor(id, version, DescriptorTypeWithPlain::PlainCwl)
        .await
        .expect("PLAIN_CWL descriptor");
    let content = descriptor.content.expect("descriptor content");
    assert!(content.contains("class: Workflow"), "{content}");
    let files = trs
        .list_files(id, version, DescriptorType::Cwl)
        .await
        .expect("CWL file list");
    assert!(!files.is_empty());
}
