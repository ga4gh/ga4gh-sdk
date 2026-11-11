//! A downstream crate reads its own section of the configuration file.

use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;

/// `ServiceType` is `#[non_exhaustive]`, so a downstream `match` needs a wildcard arm.
fn describe(service_type: &ServiceType) -> &str {
    match service_type {
        ServiceType::TES => "task execution",
        ServiceType::Custom(name) => name,
        _ => "other",
    }
}

#[test]
fn downstream_crate_reads_custom_section_and_extra_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        r#"{
            "TES": {"base_path": "https://example.org/ga4gh/tes/v1"},
            "MY_SERVICE": {
                "base_path": "https://example.org/my-service/v1",
                "timeout_seconds": 30
            }
        }"#,
    )
    .unwrap();

    let service_type = ServiceType::Custom("MY_SERVICE".into());
    assert_eq!(describe(&service_type), "MY_SERVICE");
    assert_eq!(describe(&ServiceType::WES), "other");

    let config = Configuration::from_path(&path, service_type.as_str()).unwrap();
    assert_eq!(
        config.base_path.as_str(),
        "https://example.org/my-service/v1"
    );
    assert_eq!(
        config
            .extra
            .get("timeout_seconds")
            .and_then(|value| value.as_u64()),
        Some(30)
    );
}
