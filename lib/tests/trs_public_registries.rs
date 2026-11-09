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
        .unwrap()
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
    let tools = trs.list_tools(&filter).await.unwrap();
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
        .unwrap();
    assert!(!descriptor.content.unwrap_or_default().is_empty());
}

#[tokio::test]
#[ignore = "needs network access to dockstore.org"]
async fn test_dockstore_fetches_a_workflow_by_encoded_id() {
    let trs = client("https://dockstore.org/api/ga4gh/trs/v2/").await;
    let id = "#workflow/github.com/denis-yuen/dockstore-whalesay";
    let tool = trs.get_tool(id).await.unwrap();
    assert_eq!(tool.id, id);
    let version = &trs.list_versions(id).await.unwrap()[0].name;
    let version = version.as_deref().unwrap();

    let descriptor = trs
        .get_descriptor(id, version, DescriptorTypeWithPlain::PlainCwl)
        .await
        .unwrap();
    assert!(descriptor.content.unwrap().contains("class: Workflow"));
    let files = trs
        .list_files(id, version, DescriptorType::Cwl)
        .await
        .unwrap();
    assert!(!files.is_empty());
}
