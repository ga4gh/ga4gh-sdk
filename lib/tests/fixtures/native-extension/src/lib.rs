//! Native library used by the extension loader's boundary test.

use serde_json::Value;
use std::sync::Mutex;

static CONFIG: Mutex<Option<Value>> = Mutex::new(None);

/// Supplies one echo method using the experimental extension ABI.
#[no_mangle]
pub fn init(config: Value) -> Vec<&'static [&'static str]> {
    *CONFIG.lock().unwrap() = Some(config);
    vec![
        &["test", "echo", "echo"],
        &["test", "configuration", "configuration"],
    ]
}

/// Exposes the initializer's settings to the host's configuration boundary test.
#[no_mangle]
pub fn configuration(_: Value) -> Value {
    CONFIG.lock().unwrap().clone().unwrap()
}

/// Echoes the input to prove the function still works after its original owner drops.
#[no_mangle]
pub fn echo(input: Value) -> Value {
    input
}

/// Marks the final release so the test can check unload timing.
#[no_mangle]
pub extern "C" fn deinit() {
    if let Ok(mut config) = CONFIG.lock() {
        if let Some(path) = config
            .take()
            .and_then(|value| value["on_drop"].as_str().map(str::to_owned))
        {
            let _ = std::fs::write(path, "deinitialized");
        }
    }
}
