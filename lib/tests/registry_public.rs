//! Read-only smoke test against a public Service Registry.
//!
//! Ignored by default because it needs network access. Run it with
//! `cargo test -p ga4gh-lib --test registry_public -- --ignored` after setting
//! `GA4GH_REGISTRY_URL` to a Registry API root that permits anonymous reads.

use ga4gh_sdk::clients::registry::ServiceRegistry;
use ga4gh_sdk::utils::configuration::Configuration;
use url::Url;

#[tokio::test]
#[ignore = "needs network access to a public registry"]
async fn public_registry_reads_without_a_token() {
    let base_path = std::env::var("GA4GH_REGISTRY_URL")
        .expect("set GA4GH_REGISTRY_URL to a Registry API root that permits anonymous reads");
    let registry = ServiceRegistry::new(&Configuration::new(Url::parse(&base_path).unwrap()));

    let info = registry.service_info().await.unwrap();
    assert!(!info.id.is_empty());

    let types = registry.list_service_types().await.unwrap();
    let services = registry.list_services().await.unwrap();
    for service in &services {
        assert!(types.contains(&service.r#type), "{:?}", service.r#type);
        let found = registry.get_service(&service.id).await.unwrap();
        assert_eq!(found.url, service.url);
    }
}
