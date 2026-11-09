#[cfg(feature = "integration_tests")]
#[cfg(test)]
mod tests {
    use ga4gh_sdk::utils::configuration::Configuration;
    use ga4gh_sdk::clients::serviceinfo::ServiceInfo;
    use ga4gh_sdk::utils::test_utils::{ensure_funnel_running, setup};

    #[tokio::test]
    async fn test_get_service_info_from_funnel() {
        setup();
        let mut config = Configuration::default();
        let funnel_url = ensure_funnel_running().await;
        let funnel_url = url::Url::parse(&format!("{funnel_url}/ga4gh/tes/v1")).expect("Invalid URL");
        config.set_base_path(funnel_url);
        let service_info = ServiceInfo::new(&config).unwrap();

        // Call get_service_info and print the result
        match service_info.get().await {
            Ok(service) => {
                println!("Service Info: {:?}", service);
            }
            Err(e) => {
                log::error!("ServiceInfo error in 'lib/src/serviceinfo/mod.rs' during operation: {}", e);
            }
        }
    }
}
