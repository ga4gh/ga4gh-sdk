//! `login`, `logout` and the stored tokens of sections with `oidc` settings,
//! against a fake OpenID Connect provider on the shared mock server.
//!
//! Each test uses its own paths on the mock server, its own configuration
//! directory and, where it needs one, its own environment variable, so that
//! tests running in parallel don't affect each other.

use ga4gh_cli::CliContext;
use ga4gh_sdk::clients::ServiceType;
use mockito::Matcher;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// The issuer of the fake provider under `prefix`.
fn issuer(server: &mockito::ServerGuard, prefix: &str) -> String {
    format!("{}{prefix}/idp", server.url())
}

/// A configuration with a `REGISTRY` section under `prefix` that logs in
/// with the fake provider, plus `extra` section fields.
fn write_config(
    server: &mockito::ServerGuard,
    dir: &Path,
    prefix: &str,
    extra: Value,
) -> CliContext {
    let mut section = json!({
        "base_path": format!("{}{prefix}/registry", server.url()),
        "oidc": {"issuer": issuer(server, prefix), "client_id": "test-cli", "scope": "openid"},
    });
    section
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let path = dir.join("config.json");
    std::fs::write(&path, json!({"REGISTRY": section}).to_string()).unwrap();
    CliContext::new("test-cli", "1.0.0").with_config_path(path)
}

fn discovery(server: &mut mockito::ServerGuard, prefix: &str) -> mockito::Mock {
    let issuer = issuer(&*server, prefix);
    server
        .mock(
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
fn write_store(
    server: &mockito::ServerGuard,
    dir: &Path,
    prefix: &str,
    access: &str,
    refresh: &str,
    expires_at: u64,
) {
    let issuer = issuer(server, prefix);
    let store = json!({"logins": [{
        "issuer": issuer,
        "client_id": "test-cli",
        "scope": "openid",
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

fn services(server: &mut mockito::ServerGuard, prefix: &str, token: &str) -> mockito::Mock {
    server
        .mock("GET", format!("{prefix}/registry/services").as_str())
        .match_header("authorization", format!("Bearer {token}").as_str())
        .with_body("[]")
        .expect(1)
        .create()
}

#[tokio::test]
async fn login_stores_the_tokens_privately_and_commands_send_them() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-flow";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    let _discovery = discovery(&mut server, prefix);
    let device = server
        .mock("POST", format!("{prefix}/idp/device").as_str())
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
    let pending = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .with_status(400)
        .with_body(json!({"error": "authorization_pending"}).to_string())
        .expect(1)
        .create();
    let token = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
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

    let request = services(&mut server, prefix, "access-secret");
    let (stdout, stderr, success) = run(&context, &["test-cli", "registry", "list"]).await;
    assert!(success, "{stderr}");
    assert!(!stdout.contains("secret") && !stderr.contains("secret"));
    request.assert();
}

#[tokio::test]
async fn an_expired_token_is_refreshed_and_stored() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-refresh";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(
        &server,
        dir.path(),
        prefix,
        "old-access",
        "old-refresh",
        now() - 10,
    );
    let refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
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
    let request = services(&mut server, prefix, "new-access");

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
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-rejected";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(
        &server,
        dir.path(),
        prefix,
        "revoked",
        "refresh",
        now() + 3600,
    );
    let rejected = server
        .mock("GET", format!("{prefix}/registry/services").as_str())
        .match_header("authorization", "Bearer revoked")
        .with_status(401)
        .expect(1)
        .create();
    let refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .with_body(
            json!({"access_token": "fresh", "token_type": "Bearer", "refresh_token": "next"})
                .to_string(),
        )
        .expect(1)
        .create();
    let request = services(&mut server, prefix, "fresh");

    let (_, stderr, success) = run(&context, &["test-cli", "registry", "list"]).await;
    assert!(success, "{stderr}");
    rejected.assert();
    refresh.assert();
    request.assert();
    assert_eq!(read_store(dir.path())["logins"][0]["refresh_token"], "next");
}

#[tokio::test]
async fn a_failed_refresh_asks_to_log_in_again() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-refresh-failed";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(
        &server,
        dir.path(),
        prefix,
        "old",
        "expired-refresh",
        now() - 10,
    );
    let _refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
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
    let server = mockito::Server::new();
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), "/login-missing", json!({}));
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("Not logged in"), "{error}");
    assert!(
        error.to_string().contains("run 'test-cli login REGISTRY'"),
        "{error}"
    );

    let context = write_config(
        &server,
        dir.path(),
        "/login-conflict",
        json!({"oauth_access_token": "configured"}),
    );
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("both 'oidc'"), "{error}");

    let context = write_config(
        &server,
        dir.path(),
        "/login-bad",
        json!({"oidc": {"issuer": 1}}),
    );
    let error = context.load_config(ServiceType::REGISTRY).unwrap_err();
    assert!(error.to_string().contains("issuer"), "{error}");
}

#[tokio::test]
async fn login_rejects_a_conflicting_credential_before_discovery() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-conflicting-credential";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(
        &server,
        dir.path(),
        prefix,
        json!({"oauth_access_token": "configured"}),
    );
    let discovery = server
        .mock(
            "GET",
            format!("{prefix}/idp/.well-known/openid-configuration").as_str(),
        )
        .expect(0)
        .create();
    let error = run_error(&context, &["test-cli", "login", "registry"]).await;
    assert!(error.contains("both 'oidc' and a credential"), "{error}");
    assert!(!dir.path().join("tokens.json").exists());
    discovery.assert();
}

#[tokio::test]
async fn dispatch_requires_a_caller_owned_prompt_for_login() {
    let server = mockito::Server::new_async().await;
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), "/login-prompt", json!({}));
    let matches = context
        .command()
        .try_get_matches_from(["test-cli", "login", "registry"])
        .unwrap();
    let error = context.dispatch(&matches).await.err().unwrap().to_string();
    assert!(error.contains("dispatch_to"), "{error}");
    assert!(!dir.path().join("tokens.json").exists());
}

#[tokio::test]
async fn provider_error_that_echoes_refresh_token_is_not_printed() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-echoed-refresh-token";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(
        &server,
        dir.path(),
        prefix,
        "old-access",
        "refresh-secret",
        now() - 10,
    );
    let response = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .match_body(Matcher::UrlEncoded(
            "refresh_token".into(),
            "refresh-secret".into(),
        ))
        .with_status(400)
        .with_body(json!({"error": "refresh-secret"}).to_string())
        .expect(1)
        .create();
    let error = run_error(&context, &["test-cli", "registry", "list"]).await;
    assert!(error.contains("unrecognized_error"), "{error}");
    assert!(!error.contains("refresh-secret"), "{error}");
    response.assert();
}

#[test]
fn the_token_variable_overrides_the_login() {
    let server = mockito::Server::new();
    const VARIABLE: &str = "GA4GH_SDK_LOGIN_TEST_OVERRIDE";
    let dir = tempfile::TempDir::new().unwrap();
    // No stored login: with the variable set, none is needed.
    let context = write_config(&server, dir.path(), "/login-env", json!({}))
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
    let server = mockito::Server::new_async().await;
    let prefix = "/login-logout";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(
        &server,
        dir.path(),
        prefix,
        "access",
        "refresh",
        now() + 3600,
    );
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
async fn sections_with_different_scopes_keep_separate_logins() {
    let server = mockito::Server::new_async().await;
    let prefix = "/login-distinct-scopes";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    let config_path = dir.path().join("config.json");
    let mut config: Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
    let mut tes = config["REGISTRY"].clone();
    tes["base_path"] = json!(format!("{}{prefix}/tes", server.url()));
    tes["oidc"]["scope"] = json!("openid tes.read");
    config["TES"] = tes;
    std::fs::write(&config_path, config.to_string()).unwrap();

    write_store(
        &server,
        dir.path(),
        prefix,
        "registry-access",
        "registry-refresh",
        now() + 3600,
    );
    let mut store = read_store(dir.path());
    let mut tes_login = store["logins"][0].clone();
    tes_login["scope"] = json!("openid tes.read");
    tes_login["access_token"] = json!("tes-access");
    store["logins"].as_array_mut().unwrap().push(tes_login);
    std::fs::write(dir.path().join("tokens.json"), store.to_string()).unwrap();

    assert!(context.load_config(ServiceType::REGISTRY).is_ok());
    assert!(context.load_config(ServiceType::TES).is_ok());
    let (_, stderr, success) = run(&context, &["test-cli", "logout", "registry"]).await;
    assert!(success, "{stderr}");
    let remaining = read_store(dir.path())["logins"].as_array().unwrap().clone();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["scope"], "openid tes.read");
    assert_eq!(remaining[0]["access_token"], "tes-access");
    assert!(context.load_config(ServiceType::TES).is_ok());
    assert!(context.load_config(ServiceType::REGISTRY).is_err());
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

/// Runs `args` and returns the error that dispatch returns.
async fn run_error(context: &CliContext, args: &[&str]) -> String {
    let matches = context.command().try_get_matches_from(args).unwrap();
    context
        .dispatch_to(&matches, &mut Vec::new(), &mut Vec::new())
        .await
        .expect_err("an error")
        .to_string()
}

#[tokio::test]
async fn an_expired_login_without_refresh_token_asks_to_log_in_again() {
    let server = mockito::Server::new_async().await;
    let prefix = "/login-no-refresh";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    let issuer = issuer(&server, prefix);
    let store = json!({"logins": [{
        "issuer": issuer,
        "client_id": "test-cli",
        "scope": "openid",
        "token_endpoint": format!("{issuer}/token"),
        "access_token": "old",
        "expires_at": now() - 10,
    }]});
    std::fs::write(dir.path().join("tokens.json"), store.to_string()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            dir.path().join("tokens.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
    }
    let error = run_error(&context, &["test-cli", "registry", "list"]).await;
    assert!(
        error.contains("run 'test-cli login REGISTRY' again"),
        "{error}"
    );
}

#[tokio::test]
async fn a_transient_refresh_error_is_reported_by_its_code() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-refresh-unavailable";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(&server, dir.path(), prefix, "old", "refresh", now() - 10);
    let _refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .with_status(503)
        .with_body(json!({"error": "temporarily_unavailable"}).to_string())
        .create();
    let error = run_error(&context, &["test-cli", "registry", "list"]).await;
    assert!(error.contains("temporarily_unavailable"), "{error}");
    assert!(!error.contains("login"), "{error}");
}

#[tokio::test]
async fn tokens_that_another_process_refreshed_are_used_without_refreshing() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-rotated";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(
        &server,
        dir.path(),
        prefix,
        "old",
        "rotated-away",
        now() - 10,
    );
    let config = context.load_config(ServiceType::REGISTRY).unwrap();
    // Another process refreshes in the meantime.
    write_store(
        &server,
        dir.path(),
        prefix,
        "newer",
        "current",
        now() + 3600,
    );
    let refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .expect(0)
        .create();
    let request = services(&mut server, prefix, "newer");
    let client = ga4gh_sdk::clients::registry::ServiceRegistry::new(&config);
    client.list_services().await.unwrap();
    request.assert();
    refresh.assert();
}

#[tokio::test]
async fn a_refresh_after_logout_does_not_store_the_tokens_again() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-logged-out";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    write_store(&server, dir.path(), prefix, "old", "refresh", now() - 10);
    let config = context.load_config(ServiceType::REGISTRY).unwrap();
    // `logout` runs while the command runs.
    let (_, stderr, success) = run(&context, &["test-cli", "logout", "registry"]).await;
    assert!(success, "{stderr}");
    let refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .expect(0)
        .create();
    let client = ga4gh_sdk::clients::registry::ServiceRegistry::new(&config);
    assert!(client.list_services().await.is_err());
    refresh.assert();
    assert_eq!(read_store(dir.path())["logins"], json!([]));
}

#[tokio::test]
async fn dispatch_logs_out_of_a_store_at_a_set_path() {
    let server = mockito::Server::new_async().await;
    let prefix = "/login-store-path";
    let dir = tempfile::TempDir::new().unwrap();
    let store_dir = dir.path().join("store");
    std::fs::create_dir(&store_dir).unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}))
        .with_token_store_path(store_dir.join("tokens.json"));
    write_store(
        &server,
        &store_dir,
        prefix,
        "access",
        "refresh",
        now() + 3600,
    );

    let matches = context
        .command()
        .try_get_matches_from(["test-cli", "logout", "registry"])
        .unwrap();
    let output = context
        .dispatch(&matches)
        .await
        .unwrap()
        .expect("built-in command");
    assert_eq!(output.exit_code, std::process::ExitCode::SUCCESS);
    assert!(output.stderr.contains("Deleted"), "{}", output.stderr);
    assert_eq!(read_store(&store_dir)["logins"], json!([]));
    assert!(!dir.path().join("tokens.json").exists());
}

#[tokio::test]
async fn logout_reports_an_unreadable_configuration() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, "{\"REGISTRY\": ").unwrap();
    let context = CliContext::new("test-cli", "1.0.0").with_config_path(path);
    let error = run_error(&context, &["test-cli", "logout", "registry"]).await;
    assert!(
        error.starts_with("Invalid JSON in configuration file"),
        "{error}"
    );
    assert!(!dir.path().join("tokens.json").exists());
}

#[tokio::test]
async fn login_without_a_refresh_token_says_that_it_expires() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-no-refresh-token";
    let dir = tempfile::TempDir::new().unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}));
    let _discovery = discovery(&mut server, prefix);
    let _device = server
        .mock("POST", format!("{prefix}/idp/device").as_str())
        .with_body(
            json!({
                "device_code": "device-code",
                "user_code": "WDJB-MJHT",
                "verification_uri": "https://idp.example.org/device",
                "expires_in": 60,
                "interval": 0,
            })
            .to_string(),
        )
        .create();
    let _token = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .with_body(json!({"access_token": "access-secret", "token_type": "Bearer"}).to_string())
        .expect(1)
        .create();

    let (_, stderr, success) = run(&context, &["test-cli", "login", "registry"]).await;
    assert!(success, "{stderr}");
    assert!(stderr.contains("Logged in"), "{stderr}");
    assert!(stderr.contains("issued no refresh token"), "{stderr}");
    let store = read_store(dir.path());
    assert_eq!(store["logins"][0]["access_token"], "access-secret");
    assert!(store["logins"][0].get("refresh_token").is_none());
}

#[tokio::test]
async fn refreshed_tokens_are_used_when_store_replacement_fails() {
    let mut server = mockito::Server::new_async().await;
    let prefix = "/login-refresh-write-failure";
    let dir = tempfile::TempDir::new().unwrap();
    let store_dir = dir.path().join("store");
    std::fs::create_dir(&store_dir).unwrap();
    let context = write_config(&server, dir.path(), prefix, json!({}))
        .with_token_store_path(store_dir.join("tokens.json"));
    write_store(
        &server,
        &store_dir,
        prefix,
        "old-access",
        "old-refresh",
        now() - 10,
    );
    let store_path = store_dir.join("tokens.json");
    let backup_path = dir.path().join("tokens-before-refresh.json");
    let saved = backup_path.clone();
    let destination = store_path.clone();
    let refresh = server
        .mock("POST", format!("{prefix}/idp/token").as_str())
        .with_chunked_body(move |writer| {
            // Refresh has already read the store. Make its replacement fail
            // independently of permissions, including for privileged users.
            std::fs::rename(&destination, &saved)?;
            std::fs::create_dir(&destination)?;
            std::fs::write(destination.join("entry"), "preserved")?;
            writer.write_all(
                br#"{"access_token":"new-access","token_type":"Bearer","expires_in":600}"#,
            )
        })
        .expect(1)
        .create();
    let request = services(&mut server, prefix, "new-access");

    let (_, stderr, success) = run(&context, &["test-cli", "registry", "list"]).await;
    assert!(success, "{stderr}");
    refresh.assert();
    request.assert();
    let store: Value = serde_json::from_slice(&std::fs::read(backup_path).unwrap()).unwrap();
    assert_eq!(store["logins"][0]["access_token"], "old-access");
    assert_eq!(
        std::fs::read_to_string(store_path.join("entry")).unwrap(),
        "preserved"
    );
}
