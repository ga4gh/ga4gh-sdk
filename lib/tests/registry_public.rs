//! Read-only smoke tests against live Service Registries.
//!
//! Both are ignored by default because they need network access. The official
//! GA4GH Registry smoke test exercises all four read endpoints using its own
//! stable ID. A separate, configurable test checks lookup for every listed ID.

use ga4gh_sdk::clients::registry::ServiceRegistry;
use ga4gh_sdk::utils::configuration::Configuration;
use url::Url;

#[tokio::test]
#[ignore = "needs network access to the official GA4GH Registry"]
async fn official_registry_reads_without_a_token() {
    let base_path = Url::parse("https://registry.ga4gh.org/v1").unwrap();
    let registry = ServiceRegistry::new(&Configuration::new(base_path));

    let info = registry.service_info().await.unwrap();
    assert!(!info.id.is_empty());

    let types = registry.list_service_types().await.unwrap();
    assert!(types.contains(&info.r#type));

    let services = registry.list_services().await.unwrap();
    assert!(services.iter().any(|service| service.id == info.id));

    let found = registry.get_service(&info.id).await.unwrap();
    assert_eq!(found.id, info.id);
    assert_eq!(found.r#type, info.r#type);
}

#[tokio::test]
#[ignore = "needs a live registry with every Service Registry 1.0 read endpoint"]
async fn configured_registry_supports_all_read_endpoints() {
    let base_path = std::env::var("GA4GH_REGISTRY_URL")
        .expect("set GA4GH_REGISTRY_URL to a Registry API root that permits anonymous reads");
    let registry = ServiceRegistry::new(&Configuration::new(Url::parse(&base_path).unwrap()));

    let info = registry.service_info().await.unwrap();
    assert!(!info.id.is_empty());

    let types = registry.list_service_types().await.unwrap();
    let services = registry.list_services().await.unwrap();
    for service in &services {
        assert!(types.contains(&service.r#type), "{:?}", service.r#type);
        let found = registry
            .get_service(&service.id)
            .await
            .unwrap_or_else(|error| panic!("could not get service {}: {error}", service.id));
        assert_eq!(found.url, service.url);
    }
}
