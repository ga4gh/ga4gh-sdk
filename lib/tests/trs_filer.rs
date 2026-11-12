//! Runs the TRS client against a real TRS-Filer (TRS 2.0.2).
//!
//! Run through `tests/run-trs-filer-integration.sh`, which starts TRS-Filer
//! and sets `TRS_FILER_BASE_URL`. The test registers its own tool with a
//! unique tag, so it doesn't depend on other data in the registry.
#![cfg(feature = "trs_filer_integration_tests")]

use ga4gh_sdk::clients::trs::models::tool_file::FileType;
use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::{json, Value};
use url::Url;

const MAIN: &str = "cwlVersion: v1.2\nclass: Workflow\ninputs:\n  message: string\nsteps:\n  echo:\n    run: tools/echo.cwl\n    in: {message: message}\n    out: []\noutputs: []\n";
const ECHO: &str = "cwlVersion: v1.2\nclass: CommandLineTool\nbaseCommand: echo\ninputs:\n  message: {type: string, inputBinding: {position: 1}}\noutputs: []\n";
const SCHEMA: &str = r#"{"type":"object","properties":{"message":{"type":"string"}}}"#;

fn base_url() -> String {
    std::env::var("TRS_FILER_BASE_URL")
        .expect("TRS_FILER_BASE_URL is unset; run through tests/run-trs-filer-integration.sh")
}

fn file(path: &str, file_type: &str, content: &str) -> Value {
    json!({
        "tool_file": {"path": path, "file_type": file_type},
        "file_wrapper": {"content": content},
        "type": "CWL",
    })
}

/// Registers one CWL workflow with a primary and a secondary descriptor and
/// an `OTHER` file; returns its tool ID.
async fn register(tag: &str) -> String {
    let tool = json!({
        "organization": "GA4GH SDK integration",
        "name": "echo-workflow",
        "description": "Echoes a message",
        "toolclass": {"name": "Workflow", "description": "A workflow"},
        "tags": [tag, "sdk-integration"],
        "versions": [{
            "name": "1.0.0",
            "description": "First release",
            "is_production": false,
            "descriptor_type": ["CWL"],
            "files": [
                file("main.cwl", "PRIMARY_DESCRIPTOR", MAIN),
                file("tools/echo.cwl", "SECONDARY_DESCRIPTOR", ECHO),
                file("inputs.schema.json", "OTHER", SCHEMA),
            ],
        }],
    });
    let response = reqwest::Client::new()
        .post(format!("{}/tools", base_url()))
        .json(&tool)
        .send()
        .await
        .unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert!(status.is_success(), "registration failed: {status} {body}");
    serde_json::from_str(&body).unwrap()
}

// File names are stored uncompressed in a ZIP archive's headers.
fn contains(archive: &[u8], name: &str) -> bool {
    archive
        .windows(name.len())
        .any(|window| window == name.as_bytes())
}

#[tokio::test]
async fn trs_filer_lists_tools_and_serves_their_files() {
    let tag = format!("sdk-{}", std::process::id());
    let id = register(&tag).await;
    let config = Configuration::new(Url::parse(&base_url()).unwrap());
    let trs = TRS::new(&config).await.unwrap();

    // Tags filter: both tags must match; an unknown tag matches nothing.
    let filter = ToolFilter {
        tags: vec![tag.clone(), "sdk-integration".into()],
        ..Default::default()
    };
    let tools = trs.list_tools(&filter).await.unwrap();
    assert_eq!(
        tools.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        [id.as_str()]
    );
    let filter = ToolFilter {
        tags: vec![tag.clone(), "no-such-tag".into()],
        ..Default::default()
    };
    assert!(trs.list_tools(&filter).await.unwrap().is_empty());

    // Versions, including the 2.0.2 `description`.
    let versions = trs.list_versions(&id).await.unwrap();
    assert_eq!(versions.len(), 1);
    let version = trs.get_version(&id, &versions[0].id).await.unwrap();
    assert_eq!(version.description.as_deref(), Some("First release"));
    assert_eq!(version.descriptor_type, Some(vec![DescriptorType::Cwl]));
    let version_id = version.id;

    // Primary descriptor. TRS-Filer doesn't serve the `PLAIN_` types yet; the
    // mock tests cover them.
    let descriptor = trs
        .get_descriptor(&id, &version_id, DescriptorTypeWithPlain::Cwl)
        .await
        .unwrap();
    assert_eq!(descriptor.content.as_deref(), Some(MAIN));

    // File list with all three file types, then the `OTHER` file itself.
    let mut files: Vec<_> = trs
        .list_files(&id, &version_id, DescriptorType::Cwl)
        .await
        .unwrap()
        .into_iter()
        .map(|f| (f.path.unwrap(), f.file_type.unwrap()))
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        files,
        [
            ("inputs.schema.json".to_owned(), FileType::Other),
            ("main.cwl".to_owned(), FileType::PrimaryDescriptor),
            ("tools/echo.cwl".to_owned(), FileType::SecondaryDescriptor),
        ]
    );
    // A file by relative path. Only a top-level one: TRS-Filer doesn't find
    // nested paths sent as one encoded segment (`tools%2Fecho.cwl`), as the
    // client sends them; the mock tests cover nested paths.
    let schema = trs
        .get_descriptor_file(
            &id,
            &version_id,
            DescriptorTypeWithPlain::Cwl,
            "inputs.schema.json",
        )
        .await
        .unwrap();
    assert_eq!(schema.content.as_deref(), Some(SCHEMA));

    // All files as one ZIP archive.
    let archive = trs
        .get_files_zip(&id, &version_id, DescriptorType::Cwl, 1024 * 1024)
        .await
        .unwrap();
    assert!(archive.starts_with(b"PK\x03\x04"));
    for name in ["inputs.schema.json", "main.cwl", "tools/echo.cwl"] {
        assert!(
            contains(&archive, name),
            "{name} is missing from the archive"
        );
    }
}
