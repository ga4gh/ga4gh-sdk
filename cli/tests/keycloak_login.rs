//! `login`, a command with the stored token, refreshes after expiry and
//! `logout` with the `ga4gh-cli` binary against a real OpenID Connect
//! provider: a throwaway Keycloak with the test realm in
//! `tests/keycloak/realm.json`.
//!
//! Run through `tests/run-keycloak-integration.sh`, which starts Keycloak
//! and sets `KEYCLOAK_ISSUER`. The test user's approval in the browser is
//! scripted by `tests/keycloak/approve-device.sh`. The realm issues access
//! tokens for 40 seconds and rotates refresh tokens, so the test waits about
//! 20 seconds for two refreshes.
#![cfg(all(unix, feature = "keycloak_integration_tests"))]

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tempfile::TempDir;

/// The public client of the test realm.
const CLIENT_ID: &str = "ga4gh-cli-test";

/// The user that `approve-device.sh` signs in as.
const USERNAME: &str = "test-user";

/// The CLI refreshes a token this long before it expires (see `login.rs`).
const EXPIRY_MARGIN: u64 = 30;

/// The time limit of a `login`, from the request to the approval.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(120);

fn issuer() -> String {
    std::env::var("KEYCLOAK_ISSUER").unwrap_or_else(|_| {
        panic!("KEYCLOAK_ISSUER is unset; run tests/run-keycloak-integration.sh")
    })
}

fn approve_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/keycloak/approve-device.sh")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A Registry service on loopback that answers every request with an empty
/// service list and records the `Authorization` header of each request.
struct Service {
    base_path: String,
    authorizations: Arc<Mutex<Vec<Option<String>>>>,
}

impl Service {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_path = format!("http://{}/registry", listener.local_addr().unwrap());
        let authorizations = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&authorizations);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut authorization = None;
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap_or(0) > 0 && line != "\r\n" {
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("authorization") {
                            authorization = Some(value.trim().to_owned());
                        }
                    }
                    line.clear();
                }
                recorded.lock().unwrap().push(authorization);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Content-Length: 2\r\nConnection: close\r\n\r\n[]",
                );
            }
        });
        Self {
            base_path,
            authorizations,
        }
    }

    fn requests(&self) -> Vec<Option<String>> {
        self.authorizations.lock().unwrap().clone()
    }
}

/// A home directory whose CLI configuration has a `REGISTRY` section that
/// points to `service` and logs in with the test realm's client.
fn home_for(service: &Service) -> TempDir {
    let home = TempDir::new().unwrap();
    let dir = home.path().join(".ga4gh-cli");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(
        dir.join("config.json"),
        json!({"REGISTRY": {
            "base_path": service.base_path,
            "oidc": {"issuer": issuer(), "client_id": CLIENT_ID},
        }})
        .to_string(),
    )
    .unwrap();
    home
}

fn store_path(home: &TempDir) -> PathBuf {
    home.path().join(".ga4gh-cli/tokens.json")
}

/// The only stored login.
fn stored_login(home: &TempDir) -> Value {
    let store: Value =
        serde_json::from_str(&std::fs::read_to_string(store_path(home)).unwrap()).unwrap();
    let logins = store["logins"].as_array().unwrap();
    assert_eq!(logins.len(), 1, "{}", logins.len());
    logins[0].clone()
}

fn field(login: &Value, name: &str) -> String {
    login[name]
        .as_str()
        .unwrap_or_else(|| panic!("the stored login has no {name}"))
        .to_owned()
}

fn cli(home: &TempDir, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home.path())
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

/// Runs `login registry`, answers the device login with `answer` (`accept`
/// or `deny`) through `approve-device.sh` once the CLI shows the URL, and
/// returns whether `login` succeeded and its standard error.
fn login(home: &TempDir, answer: &str) -> (bool, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(["login", "registry"])
        .env("HOME", home.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = BufReader::new(child.stderr.take().unwrap());
    let mut shown = String::new();
    let url = loop {
        let mut line = String::new();
        if stderr.read_line(&mut line).unwrap() == 0 {
            let _ = child.wait();
            panic!("login ended before it showed the URL: {shown}");
        }
        shown.push_str(&line);
        if let Some(url) = line.trim_end().strip_prefix("or open ") {
            break url.to_owned();
        }
    };
    assert!(
        url.starts_with(&format!("{}/device?user_code=", issuer())),
        "{url}"
    );
    let approval = Command::new("bash")
        .arg(approve_script())
        .args([url.as_str(), answer])
        .output()
        .unwrap();
    assert!(
        approval.status.success(),
        "approve-device.sh failed: {}",
        String::from_utf8_lossy(&approval.stderr)
    );

    let deadline = Instant::now() + LOGIN_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            panic!("login did not finish within {LOGIN_TIMEOUT:?}: {shown}");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    stderr.read_to_string(&mut shown).unwrap();
    (status.success(), shown)
}

/// Checks the access token at the provider's userinfo endpoint, which
/// accepts only valid tokens of the realm.
fn assert_valid_at_provider(access_token: &str) {
    let output = Command::new("curl")
        .args([
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "30",
            "--header",
        ])
        .arg(format!("Authorization: Bearer {access_token}"))
        .arg(format!("{}/protocol/openid-connect/userinfo", issuer()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "userinfo rejected the token: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let userinfo: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(userinfo["preferred_username"], USERNAME, "{userinfo}");
}

/// Sleeps until the CLI considers the stored access token due for a
/// refresh.
fn wait_for_refresh(login: &Value) {
    let due = login["expires_at"].as_u64().unwrap() - EXPIRY_MARGIN + 1;
    let now = now();
    if due > now {
        std::thread::sleep(Duration::from_secs(due - now));
    }
}

/// Runs `registry list` and checks that the service got the stored access
/// token, that the provider accepts it, and returns the stored login.
fn list_with_stored_token(home: &TempDir, service: &Service) -> Value {
    let before = service.requests().len();
    let (success, stdout, stderr) = cli(home, &["registry", "list"]);
    assert!(success, "{stderr}");
    let login = stored_login(home);
    let access = field(&login, "access_token");
    assert!(!stdout.contains(&access) && !stderr.contains(&access));
    let requests = service.requests();
    // Messages without the tokens, which would end up in the CI logs.
    assert_eq!(requests.len(), before + 1, "unexpected number of requests");
    assert!(
        requests[before].as_deref() == Some(format!("Bearer {access}").as_str()),
        "the service didn't get the stored access token"
    );
    assert_valid_at_provider(&access);
    login
}

#[test]
fn login_refresh_and_logout_with_keycloak() {
    let service = Service::start();
    let home = home_for(&service);

    let (success, stderr) = login(&home, "accept");
    assert!(success, "{stderr}");
    assert!(stderr.contains("Logged in"), "{stderr}");
    assert!(!stderr.contains("refresh token"), "{stderr}");
    let first = stored_login(&home);
    assert_eq!(field(&first, "issuer"), issuer());
    assert!(!stderr.contains(&field(&first, "access_token")));
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(store_path(&home))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // The fresh token from the login.
    let used = list_with_stored_token(&home, &service);
    assert!(
        field(&used, "access_token") == field(&first, "access_token"),
        "the fresh token of the login was refreshed"
    );

    // Refreshed once the token expires within the margin. The realm rotates
    // refresh tokens and ends the session when an old one is reused, so the
    // second refresh succeeds only if the CLI stored the rotated one.
    let mut previous = used;
    for _ in 0..2 {
        wait_for_refresh(&previous);
        let refreshed = list_with_stored_token(&home, &service);
        assert!(
            field(&refreshed, "access_token") != field(&previous, "access_token"),
            "the access token wasn't refreshed"
        );
        assert!(
            field(&refreshed, "refresh_token") != field(&previous, "refresh_token"),
            "the rotated refresh token wasn't stored"
        );
        previous = refreshed;
    }

    let (success, _, stderr) = cli(&home, &["logout", "registry"]);
    assert!(success, "{stderr}");
    assert!(stderr.contains("Deleted the stored tokens"), "{stderr}");
    let store: Value =
        serde_json::from_str(&std::fs::read_to_string(store_path(&home)).unwrap()).unwrap();
    assert_eq!(store["logins"], json!([]));

    let requests = service.requests().len();
    let (success, _, stderr) = cli(&home, &["registry", "list"]);
    assert!(!success);
    assert!(stderr.contains("Not logged in"), "{stderr}");
    assert_eq!(service.requests().len(), requests);
}

#[test]
fn a_denied_login_stores_no_tokens() {
    let service = Service::start();
    let home = home_for(&service);

    let (success, stderr) = login(&home, "deny");
    assert!(!success);
    assert!(stderr.contains("The login was denied"), "{stderr}");
    assert!(
        !store_path(&home).exists() || {
            let store: Value =
                serde_json::from_str(&std::fs::read_to_string(store_path(&home)).unwrap()).unwrap();
            store["logins"].as_array().is_none_or(Vec::is_empty)
        }
    );
    assert!(service.requests().is_empty());
}
