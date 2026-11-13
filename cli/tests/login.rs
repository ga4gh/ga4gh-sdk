//! `login`, `logout` and the stored tokens of sections with `oidc` settings,
//! against a fake OpenID Connect provider on the shared mock server.
//!
//! Each test uses its own paths on the mock server, its own configuration
//! directory and, where it needs one, its own environment variable, so that
//! tests running in parallel don't affect each other.

use ga4gh_cli::CliContext;
use ga4gh_sdk::clients::ServiceType;
use mockito::{mock, Matcher};
use serde_json::{json, Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The issuer of the fake provider under `prefix`.
fn issuer(prefix: &str) -> String {
    format!("{}{prefix}/idp", mockito::server_url())
}

/// A configuration with a `REGISTRY` section under `prefix` that logs in
/// with the fake provider, plus `extra` section fields.
fn write_config(dir: &Path, prefix: &str, extra: Value) -> CliContext {
    let mut section = json!({
        "base_path": format!("{}{prefix}/registry", mockito::server_url()),
        "oidc": {"issuer": issuer(prefix), "client_id": "test-cli", "scope": "openid"},
    });
    section
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let path = dir.join("config.json");
    std::fs::write(&path, json!({"REGISTRY": section}).to_string()).unwrap();
    CliContext::new("test-cli", "1.0.0").with_config_path(path)
}

fn discovery(prefix: &str) -> mockito::Mock {
    let issuer = issuer(prefix);
    mock(
        "GET",
        format!("{prefix}/idp/.well-known/openid-configuration").as_str(),
    )
    .with_body(
        json!({
            "issuer": issuer,
            "device_authorization_endpoint": format!("{issuer}/device"),
            "token_endpoint": format!("{issuer}/token"),
        })
        .to_string(),
    )
    .create()
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Writes a stored login for the fake provider under `prefix`.
fn write_store(dir: &Path, prefix: &str, access: &str, refresh: &str, expires_at: u64) {
    let issuer = issuer(prefix);
    let store = json!({"logins": [{
        "issuer": issuer,
        "client_id": "test-cli",
        "token_endpoint": format!("{issuer}/token"),
        "access_token": access,
        "refresh_token": refresh,
        "expires_at": expires_at,
    }]});
    let path = dir.join("tokens.json");
    std::fs::write(&path, store.to_string()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

fn read_store(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("tokens.json")).unwrap()).unwrap()
}

async fn run(context: &CliContext, args: &[&str]) -> (String, String, bool) {
    let matches = context.command().try_get_matches_from(args).unwrap();
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = context
        .dispatch_to(&matches, &mut stdout, &mut stderr)
        .await
        .unwrap()
        .expect("built-in command");
    (
        String::from_utf8(stdout).unwrap(),
        String::from_utf8(stderr).unwrap(),
        exit_code == std::process::ExitCode::SUCCESS,
    )
}

fn services(prefix: &str, token: &str) -> mockito::Mock {
    mock("GET", format!("{prefix}/registry/services").as_str())
        .match_header("authorization", format!("Bearer {token}").as_str())
        .with_body("[]")
        .expect(1)
        .create()
}

#[tokio::test]
async fn login_stores_the_tokens_privately_and_commands_send_them() {
    let prefix = "/login-flow";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(dir.path(), prefix, json!({}));
    let _discovery = discovery(prefix);
    let device = mock("POST", format!("{prefix}/idp/device").as_str())
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("client_id".into(), "test-cli".into()),
            Matcher::UrlEncoded("scope".into(), "openid".into()),
        ]))
        .with_body(
            json!({
                "device_code": "device-code",
                "user_code": "WDJB-MJHT",
                "verification_uri": "https://idp.example.org/device",
                "verification_uri_complete": "https://idp.example.org/device?user_code=WDJB-MJHT",
                "expires_in": 60,
                "interval": 0,
            })
            .to_string(),
        )
        .expect(1)
        .create();
    let pending = mock("POST", format!("{prefix}/idp/token").as_str())
        .with_status(400)
        .with_body(json!({"error": "authorization_pending"}).to_string())
        .expect(1)
        .create();
    let token = mock("POST", format!("{prefix}/idp/token").as_str())
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded(
                "grant_type".into(),
                "urn:ietf:params:oauth:grant-type:device_code".into(),
            ),
            Matcher::UrlEncoded("device_code".into(), "device-code".into()),
        ]))
        .with_body(
            json!({
                "access_token": "access-secret",
                "token_type": "Bearer",
                "expires_in": 3600,
                "refresh_token": "refresh-secret",
            })
            .to_string(),
        )
        .expect(1)
        .create();

    let (stdout, stderr, success) = run(&context, &["test-cli", "login", "registry"]).await;
    assert!(success, "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    assert!(stderr.contains("WDJB-MJHT"), "{stderr}");
    assert!(
        stderr.contains("https://idp.example.org/device"),
        "{stderr}"
    );
    assert!(stderr.contains("Logged in"), "{stderr}");
    assert!(!stderr.contains("secret"), "{stderr}");
    device.assert();
    pending.assert();
    token.assert();

    let store = read_store(dir.path());
    assert_eq!(store["logins"][0]["access_token"], "access-secret");
    assert_eq!(store["logins"][0]["refresh_token"], "refresh-secret");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join("tokens.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let request = services(prefix, "access-secret");
    let (stdout, stderr, success) = run(&context, &["test-cli", "registry", "list"]).await;
    assert!(success, "{stderr}");
    assert!(!stdout.contains("secret") && !stderr.contains("secret"));
    request.assert();
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_stored() {
    let prefix = "/login-refresh";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(dir.path(), prefix, json!({}));
    write_store(dir.path(), prefix, "old-access", "old-refresh", now() - 10);
    let refresh = mock("POST", format!("{prefix}/idp/token").as_str())
        .match_body(Matcher::AllOf(vec![
            Matcher::UrlEncoded("grant_type".into(), "refresh_token".into()),
            Matcher::UrlEncoded("refresh_token".into(), "old-refresh".into()),
            Matcher::UrlEncoded("client_id".into(), "test-cli".into()),
        ]))
        .with_body(
            json!({"access_token": "new-access", "token_type": "bearer", "expires_in": 600})
                .to_string(),
        )
        .expect(1)
        .create();
    let request = services(prefix, "new-access");

    let (_, stderr, success) = run(&context, &["test-cli", "registry", "list"]).await;
    assert!(success, "{stderr}");
    refresh.assert();
    request.assert();
    let store = read_store(dir.path());
    assert_eq!(store["logins"][0]["access_token"], "new-access");
    // Without a new refresh token, the old one is kept.
    assert_eq!(store["logins"][0]["refresh_token"], "old-refresh");
    assert!(store["logins"][0]["expires_at"].as_u64().unwrap() > now() + 500);
}

#[tokio::test]
async fn a_rejected_token_is_refreshed_and_a_read_is_sent_again() {
    let prefix = "/login-rejected";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(dir.path(), prefix, json!({}));
    write_store(dir.path(), prefix, "revoked", "refresh", now() + 3600);
    let rejected = mock("GET", format!("{prefix}/registry/services").as_str())
        .match_header("authorization", "Bearer revoked")
        .with_status(401)
        .expect(1)
        .create();
    let refresh = mock("POST", format!("{prefix}/idp/token").as_str())
        .with_body(
            json!({"access_token": "fresh", "token_type": "Bearer", "refresh_token": "next"})
                .to_string(),
        )
        .expect(1)
        .create();
    let request = services(prefix, "fresh");

    let (_, stderr, success) = run(&context, &["test-cli", "registry", "list"]).await;
    assert!(success, "{stderr}");
    rejected.assert();
    refresh.assert();
    request.assert();
    assert_eq!(read_store(dir.path())["logins"][0]["refresh_token"], "next");
}

#[tokio::test]
async fn a_failed_refresh_asks_to_log_in_again() {
    let prefix = "/login-refresh-failed";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(dir.path(), prefix, json!({}));
    write_store(dir.path(), prefix, "old", "expired-refresh", now() - 10);
    let _refresh = mock("POST", format!("{prefix}/idp/token").as_str())
        .with_status(400)
        .with_body(json!({"error": "invalid_grant"}).to_string())
        .create();

    let matches = context
        .command()
        .try_get_matches_from(["test-cli", "registry", "list"])
        .unwrap();
    let error = context
        .dispatch_to(&matches, &mut Vec::new(), &mut Vec::new())
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("run 'test-cli login REGISTRY' again"),
        "{error}"
    );
    assert!(!error.contains("expired-refresh"), "{error}");
}

#[test]
fn a_section_with_oidc_needs_a_login_and_no_other_credential() {
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(dir.path(), "/login-missing", json!({}));
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("Not logged in"), "{error}");
    assert!(
        error.to_string().contains("run 'test-cli login REGISTRY'"),
        "{error}"
    );

    let context = write_config(
        dir.path(),
        "/login-conflict",
        json!({"oauth_access_token": "configured"}),
    );
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("both 'oidc'"), "{error}");

    let context = write_config(dir.path(), "/login-bad", json!({"oidc": {"issuer": 1}}));
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("issuer"), "{error}");
}

#[test]
fn the_token_variable_overrides_the_login() {
    const VARIABLE: &str = "GA4GH_SDK_LOGIN_TEST_OVERRIDE";
    let dir = tempfile::TempDir::new().unwrap();
    // No stored login: with the variable set, none is needed.
    let context = write_config(dir.path(), "/login-env", json!({}))
        .with_token_env(VARIABLE, [ServiceType::REGISTRY]);
    std::env::set_var(VARIABLE, "from-env");
    let config = context.load_config(ServiceType::REGISTRY).unwrap();
    std::env::remove_var(VARIABLE);
    assert_eq!(config.oauth_access_token.as_deref(), Some("from-env"));
    assert!(config.token_provider.is_none());

    // Without the variable, the login is used, and missing is an error.
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("Not logged in"), "{error}");
}

#[tokio::test]
async fn logout_deletes_only_the_sections_login() {
    let prefix = "/login-logout";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(dir.path(), prefix, json!({}));
    write_store(dir.path(), prefix, "access", "refresh", now() + 3600);
    let mut store = read_store(dir.path());
    let mut other = store["logins"][0].clone();
    other["issuer"] = json!("https://other.example.org");
    store["logins"].as_array_mut().unwrap().push(other);
    std::fs::write(dir.path().join("tokens.json"), store.to_string()).unwrap();

    let (_, stderr, success) = run(&context, &["test-cli", "logout", "REGISTRY"]).await;
    assert!(success, "{stderr}");
    assert!(stderr.contains("Deleted"), "{stderr}");
    let logins = read_store(dir.path())["logins"].clone();
    assert_eq!(logins.as_array().unwrap().len(), 1);
    assert_eq!(logins[0]["issuer"], "https://other.example.org");

    let (_, stderr, success) = run(&context, &["test-cli", "logout", "registry"]).await;
    assert!(success, "{stderr}");
    assert!(stderr.contains("Not logged in"), "{stderr}");
}

#[tokio::test]
async fn login_needs_oidc_settings() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(
        &path,
        json!({"TES": {"base_path": "https://tes.example.org/ga4gh/tes/v1"}}).to_string(),
    )
    .unwrap();
    let context = CliContext::new("test-cli", "1.0.0").with_config_path(path);
    let matches = context
        .command()
        .try_get_matches_from(["test-cli", "login", "tes"])
        .unwrap();
    let error = context
        .dispatch_to(&matches, &mut Vec::new(), &mut Vec::new())
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("no 'oidc' settings"), "{error}");
}
