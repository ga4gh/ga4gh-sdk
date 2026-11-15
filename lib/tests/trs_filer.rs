//! Runs the TRS client against a real TRS-Filer (TRS 2.0.2).
//!
//! Run through `tests/run-trs-filer-integration.sh`, which starts a fresh
//! TRS-Filer, registers the tool in `tests/fixtures/trs-filer/tool.json` and
//! sets `TRS_FILER_BASE_URL` and `TRS_FILER_TOOL_ID`.
#![cfg(feature = "trs_filer_integration_tests")]

use ga4gh_sdk::clients::trs::models::tool_file::FileType;
use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
use ga4gh_sdk::utils::configuration::Configuration;
use serde_json::Value;
use url::Url;

const FIXTURE: &str = include_str!("../../tests/fixtures/trs-filer/tool.json");

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} is unset; run through tests/run-trs-filer-integration.sh")
    })
}

// The registered content of the fixture file at `path`.
fn content(fixture: &Value, path: &str) -> String {
    fixture["versions"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["tool_file"]["path"] == path)
        .and_then(|file| file["file_wrapper"]["content"].as_str())
        .unwrap_or_else(|| panic!("{path} is not in the fixture"))
        .to_owned()
}

// Checks the entry in the archive's central directory: its file name and
// uncompressed size. Contents are compared through the other endpoints.
fn has_entry(archive: &[u8], name: &str, size: usize) -> bool {
    const SIGNATURE: &[u8] = b"PK\x01\x02";
    archive
        .windows(SIGNATURE.len())
        .enumerate()
        .any(|(start, window)| {
            let header = &archive[start..];
            if window != SIGNATURE || header.len() < 46 {
                return false;
            }
            let u16_at = |offset: usize| {
                usize::from(u16::from_le_bytes([header[offset], header[offset + 1]]))
            };
            let u32_at = |offset: usize| {
                u32::from_le_bytes(header[offset..offset + 4].try_into().unwrap()) as usize
            };
            let name_len = u16_at(28);
            header.get(46..46 + name_len) == Some(name.as_bytes()) && u32_at(24) == size
        })
}

#[tokio::test]
async fn trs_filer_lists_tools_and_serves_their_files() {
    let fixture: Value = serde_json::from_str(FIXTURE).unwrap();
    let id = env("TRS_FILER_TOOL_ID");
    let config = Configuration::new(Url::parse(&env("TRS_FILER_BASE_URL")).unwrap());
    let trs = TRS::new(&config).await.unwrap();

    // Tags filter: all tags of the tool match; an unknown tag excludes it.
    let tags: Vec<String> = serde_json::from_value(fixture["tags"].clone()).unwrap();
    let filter = ToolFilter {
        tags: tags.clone(),
        ..Default::default()
    };
    let tools = trs.list_tools(&filter).await.unwrap();
    assert!(tools.iter().any(|tool| tool.id == id), "{tools:?}");
    let filter = ToolFilter {
        tags: vec![tags[0].clone(), "no-such-tag".into()],
        ..Default::default()
    };
    assert!(trs.list_tools(&filter).await.unwrap().is_empty());

    // Versions, including the 2.0.2 `description`.
    let versions = trs.list_versions(&id).await.unwrap();
    assert_eq!(versions.len(), 1);
    let version = trs.get_version(&id, &versions[0].id).await.unwrap();
    assert_eq!(
        version.description(),
        fixture["versions"][0]["description"].as_str()
    );
    assert_eq!(version.descriptor_type, Some(vec![DescriptorType::Cwl]));
    let version_id = version.id;

    // Primary descriptor. TRS-Filer doesn't serve the `PLAIN_` types yet; the
    // mock tests cover them.
    let descriptor = trs
        .get_descriptor(&id, &version_id, DescriptorTypeWithPlain::Cwl)
        .await
        .unwrap();
    assert_eq!(descriptor.content, Some(content(&fixture, "main.cwl")));

    // File list with all three file types.
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

    // The `OTHER` file by relative path. Only a top-level one: TRS-Filer
    // doesn't find nested paths sent as one encoded segment
    // (`tools%2Fecho.cwl`), as the client sends them; the mock tests cover
    // nested paths.
    let schema = trs
        .get_descriptor_file(
            &id,
            &version_id,
            DescriptorTypeWithPlain::Cwl,
            "inputs.schema.json",
        )
        .await
        .unwrap();
    assert_eq!(
        schema.content,
        Some(content(&fixture, "inputs.schema.json"))
    );

    // All files as one ZIP archive.
    let archive = trs
        .get_files_zip(&id, &version_id, DescriptorType::Cwl, 1024 * 1024)
        .await
        .unwrap();
    assert!(archive.starts_with(b"PK\x03\x04"));
    for (path, _) in files {
        let size = content(&fixture, &path).len();
        assert!(
            has_entry(&archive, &path, size),
            "{path} ({size} bytes) is not in the archive"
        );
    }
}
