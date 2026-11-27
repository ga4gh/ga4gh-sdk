//! Configuration layering, compatibility and safe diagnostics at public APIs.

use figment::providers::{Env, Format, Json, Serialized};
use figment::Figment;
use ga4gh_sdk::utils::configuration::{AuthError, Configuration};
use serde::Deserialize;
use serde_json::json;
use std::fs;
use std::time::Duration;

#[test]
fn selected_file_environment_and_explicit_overrides_share_typed_settings() {
    const PREFIX: &str = "GA4GH_349_LAYER_TEST_";
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.json");
    fs::write(
        &path,
        json!({"profiles": {
            "Remote": {"my.service": {
                "base_path":"https://remote.example.org/",
                "timeouts":{"connect":3,"request":10},
                "routing":{"region":"eu","queue":"file"}
            }},
            "remote": {"my.service":{"base_path":"https://other.example.org/"}}
        }})
        .to_string(),
    )
    .unwrap();
    // These names belong only to this test; no other test changes them.
    std::env::set_var(format!("{PREFIX}TIMEOUTS__REQUEST"), "20");
    std::env::set_var(format!("{PREFIX}ROUTING__QUEUE"), "env");
    let original =
        Configuration::from_path_with_profile(&path, Some("Remote"), "my.service").unwrap();
    assert_eq!(original.timeouts.request, Duration::from_secs(10));
    let sources = Configuration::sources(&path, Some("Remote"), "my.service")
        .unwrap()
        .merge(Env::prefixed(PREFIX).split("__"))
        .merge(Serialized::defaults(json!({"timeouts":{"request":30}})))
        .join(Serialized::defaults(
            json!({"routing":{"queue":"default","tier":2}}),
        ));
    let config = Configuration::from_provider(sources).unwrap();
    std::env::remove_var(format!("{PREFIX}TIMEOUTS__REQUEST"));
    std::env::remove_var(format!("{PREFIX}ROUTING__QUEUE"));
    assert_eq!(config.base_path.as_str(), "https://remote.example.org/");
    assert_eq!(config.timeouts.connect, Duration::from_secs(3));
    assert_eq!(config.timeouts.request, Duration::from_secs(30));
    assert_eq!(
        config.extra["routing"],
        json!({"region":"eu","queue":"env","tier":2})
    );
    assert!(Configuration::sources(&path, Some("REMOTE"), "my.service").is_err());
    assert_eq!(
        Configuration::from_path_with_profile(&path, Some("remote"), "my.service")
            .unwrap()
            .base_path
            .as_str(),
        "https://other.example.org/"
    );
}

#[test]
fn nulls_clear_optional_values_arrays_replace_and_json_values_survive() {
    let sources = Figment::from(Serialized::defaults(json!({
        "base_path":"https://example.org/",
        "bearer_access_token":"old-token",
        "timeouts":{"request":2},
        "extensions":[{"name":"old","configuration":{}}],
        "custom":{"omitted":true,"cleared":1},
        "numbers":[u64::MAX,i64::MIN,-0.0]
    })))
    .merge(Serialized::defaults(json!({
        "bearer_access_token":null,
        "timeouts":null,
        "extensions":[{"name":"new","required":true,"configuration":{"key":null}}],
        "custom":{"cleared":null}
    })));
    let config = Configuration::from_provider(sources).unwrap();
    assert!(config.bearer_access_token.is_none());
    assert_eq!(config.timeouts, Default::default());
    assert_eq!(
        config.extra["custom"],
        json!({"omitted":true,"cleared":null})
    );
    assert_eq!(config.extra["numbers"], json!([u64::MAX, i64::MIN, -0.0]));
    #[derive(Deserialize)]
    struct Selection {
        name: String,
        configuration: serde_json::Value,
    }
    let settings: Vec<Selection> = config.extra_as("extensions").unwrap().unwrap();
    assert_eq!(settings.len(), 1);
    assert_eq!(settings[0].name, "new");
    assert_eq!(settings[0].configuration, json!({"key":null}));
    assert!(Configuration::from_provider(Serialized::defaults(json!({"base_path":null}))).is_err());
}

#[test]
fn layered_credentials_are_validated_instead_of_silently_selected() {
    let sources = Figment::from(Serialized::defaults(json!({
        "base_path":"https://example.org/",
        "basic_auth":{"username":"alice","password":"SENTINEL_PASSWORD"}
    })))
    .merge(Serialized::defaults(
        json!({"bearer_access_token":"SENTINEL_TOKEN"}),
    ));
    let error = Configuration::from_provider(sources).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AuthError>(),
        Some(AuthError::Conflicting { .. })
    ));
    assert!(!format!("{error} {error:?}").contains("SENTINEL"));
}

#[test]
fn selecting_a_missing_figment_profile_cannot_silently_use_defaults() {
    let profiles = Figment::from(
        Json::string(
            r#"{
        "default":{"base_path":"https://default.example.org/"},
        "remote":{"base_path":"https://remote.example.org/"}
    }"#,
        )
        .nested(),
    );
    assert_eq!(
        Configuration::from_provider(profiles.clone().select("remote"))
            .unwrap()
            .base_path
            .as_str(),
        "https://remote.example.org/"
    );
    let error = Configuration::from_provider(profiles.select("missing")).unwrap_err();
    assert!(error.to_string().contains("Profile 'missing' not found"));
}

#[test]
fn errors_identify_fields_and_files_without_echoing_values() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad-settings.json");
    fs::write(
        &path,
        r#"{"basic_auth":{"username":[],"password":"SENTINEL_PASSWORD"}}"#,
    )
    .unwrap();
    let sources = Figment::from(Serialized::defaults(
        json!({"base_path":"https://example.org/"}),
    ))
    .merge(Json::file(&path));
    let error = Configuration::from_provider(sources).unwrap_err();
    let displayed = format!("{error} {error:?}");
    assert!(displayed.contains("basic_auth.username"), "{displayed}");
    assert!(displayed.contains("bad-settings.json"), "{displayed}");
    assert!(!displayed.contains("SENTINEL"));

    fs::write(&path, "{\"TES\":{\"base_path\":SENTINEL_PASSWORD}}").unwrap();
    let error = Configuration::from_path(&path, "TES").unwrap_err();
    assert!(error.to_string().contains("Invalid JSON"));
    assert!(!format!("{error} {error:?}").contains("SENTINEL"));
}

#[test]
fn missing_section_errors_escape_control_characters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let section = "missing\n\r\t\u{1b}[31m";
    for (document, profile) in [
        (json!({}), None),
        (json!({"profiles":{"Remote":{}}}), Some("Remote")),
    ] {
        fs::write(&path, document.to_string()).unwrap();
        let error = Configuration::sources(&path, profile, section)
            .err()
            .unwrap();
        let displayed = error.to_string();
        assert!(
            displayed.contains(r"missing\n\r\t\u{1b}[31m"),
            "{displayed}"
        );
        assert!(!displayed.chars().any(char::is_control), "{displayed:?}");
    }
    // Escaping diagnostics must not change lookup of a real section name.
    fs::write(
        &path,
        json!({section:{"base_path":"https://example.org/"}}).to_string(),
    )
    .unwrap();
    assert!(Configuration::from_path(&path, section).is_ok());
}

#[test]
fn timeout_validation_errors_escape_provider_keys() {
    let key = "read\n\u{1b}[31m";
    let error = Configuration::from_provider(Serialized::defaults(json!({
        "base_path":"https://example.org/", "timeouts":{key:1}
    })))
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains(r"read\n\u{1b}[31m"), "{message}");
    assert!(!message.chars().any(char::is_control));
}

#[test]
fn extra_settings_use_a_caller_schema_without_exposing_parser_messages() {
    #[derive(Deserialize)]
    struct Routing {
        queue: String,
    }
    let config = Configuration::from_provider(Serialized::defaults(json!({
        "base_path":"https://example.org/",
        "routing":{"queue":"batch"},
        "invalid":{"queue":["SENTINEL_SECRET"]},
        "nullable":null
    })))
    .unwrap();
    assert_eq!(
        config
            .extra_as::<Routing>("routing")
            .unwrap()
            .unwrap()
            .queue,
        "batch"
    );
    assert!(config.extra_as::<Routing>("missing").unwrap().is_none());
    assert_eq!(
        config
            .extra_as::<Option<Routing>>("nullable")
            .unwrap()
            .map(|value| value.is_none()),
        Some(true)
    );
    let error = config.extra_as::<Routing>("invalid").err().unwrap();
    assert!(error.to_string().contains("invalid"));
    assert!(!format!("{error} {error:?}").contains("SENTINEL"));
}
