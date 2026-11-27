//! Helpers for the integration tests that run against a local Funnel TES
//! server (see `tests/README.md`).

use std::env;
use std::sync::Once;
use std::time::Duration;

/// The scheme and host of the local Funnel server.
pub const FUNNEL_HOST: &str = "http://localhost";
/// The port of the local Funnel server.
pub const FUNNEL_PORT: u16 = 8000;
/// Guards the one-time setup of [`setup`].
pub static INIT: Once = Once::new();

/// Turns on debug logging, once per test process.
pub fn setup() {
    INIT.call_once(|| {
        env::set_var("RUST_LOG", "debug");
        env_logger::init();
    });
}

/// Checks that Funnel answers its health check and returns its URL.
///
/// # Panics
///
/// Panics if Funnel doesn't answer within 3 seconds or answers with an error
/// status.
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
