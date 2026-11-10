use std::env;
use std::sync::Once;
use std::time::Duration;

pub const FUNNEL_HOST: &str = "http://localhost";
pub const FUNNEL_PORT: u16 = 8000;
pub static INIT: Once = Once::new();

pub fn setup() {
    INIT.call_once(|| {
        env::set_var("RUST_LOG", "debug");
        env_logger::init();
    });
}

pub async fn ensure_funnel_running() -> String {
    let funnel_url = format!("{}:{}", FUNNEL_HOST, FUNNEL_PORT);
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .expect("Failed to create HTTP client")
        .get(format!("{funnel_url}/healthz"))
        .send()
        .await
        .unwrap_or_else(|error| panic!("Funnel health check failed at {funnel_url}: {error}"));
    assert!(
        response.status().is_success(),
        "Funnel health check failed at {funnel_url}: HTTP {}",
        response.status()
    );
    funnel_url
}
