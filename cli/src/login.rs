//! `login` and `logout`: tokens from an OpenID Connect provider through the
//! device authorization grant ([RFC 8628](https://www.rfc-editor.org/rfc/rfc8628)).
//!
//! A configuration section opts in with an `oidc` object that names the
//! provider's `issuer` and a public `client_id`, and optionally a `scope`.
//! `login <SECTION>` finds the provider's endpoints through
//! [OpenID Connect discovery](https://openid.net/specs/openid-connect-discovery-1_0.html),
//! shows a URL and a code to open in a browser, and waits for the user to
//! approve. It stores the tokens in the token store, a JSON file that only
//! its owner may read (mode `0600` on Unix). Sections with `oidc` then get
//! their token from the store, refreshed with the refresh token when it
//! expires. `logout <SECTION>` deletes the stored tokens.
//!
//! Tokens are stored per issuer and client ID, so all sections with the same
//! `oidc` settings share one login. Tokens are never printed. No client secret
//! is used or stored.

use crate::{CliContext, CommandOutput};
use clap::{Arg, ArgMatches, Command};
use ga4gh_sdk::reqwest;
use ga4gh_sdk::utils::auth::{AccessToken, BoxError, TokenProvider, TokenRequest};
use ga4gh_sdk::utils::configuration::Configuration;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use url::Url;

/// The key of a configuration section that holds the login settings.
pub(crate) const OIDC_KEY: &str = "oidc";

/// The scope requested when the `oidc` settings set none. `offline_access`
/// asks for a refresh token.
const DEFAULT_SCOPE: &str = "openid offline_access";

/// A token is refreshed this long before it expires.
const EXPIRY_MARGIN: Duration = Duration::from_secs(30);

/// The time limit of each request to the provider.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// How much longer to wait between polls after `slow_down` (RFC 8628, 3.5).
const SLOW_DOWN_STEP: Duration = Duration::from_secs(5);

/// The polling interval when the provider sets none (RFC 8628, 3.2).
const DEFAULT_INTERVAL: u64 = 5;

/// The shortest polling interval, so that a provider can't make the CLI poll
/// in a busy loop.
const MIN_INTERVAL: Duration = Duration::from_secs(1);

/// Lifetimes from the provider (of tokens and device codes) are capped at a
/// year, so that huge values can't overflow time arithmetic.
const MAX_LIFETIME: u64 = 365 * 24 * 60 * 60;

/// The grant type of the device access token request (RFC 8628, 3.4).
const DEVICE_CODE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// The `login` command.
pub(crate) fn login_command() -> Command {
    Command::new("login")
        .about("Log in with an OpenID Connect provider (device flow) and store the tokens")
        .long_about(
            "Log in with an OpenID Connect provider through the device authorization \
             grant and store the tokens. The section's 'oidc' settings name the \
             provider's issuer and client ID. Sections with the same settings share \
             the login.",
        )
        .arg(section_arg())
}

/// The `logout` command.
pub(crate) fn logout_command() -> Command {
    Command::new("logout")
        .about("Delete the stored tokens of a section's OpenID Connect login")
        .arg(section_arg())
}

fn section_arg() -> Arg {
    Arg::new("section")
        .value_name("SECTION")
        .required(true)
        .value_parser(clap::builder::NonEmptyStringValueParser::new())
        .help("Configuration section with 'oidc' settings, such as TES; built-in names may be lowercase")
}

/// The section name for `name`: the built-in services' command names stand
/// for their sections, so `tes` means `TES`. Other names are used as given.
fn section_name(name: &str) -> String {
    match name {
        "drs" | "tes" | "trs" | "wes" | "registry" => name.to_ascii_uppercase(),
        _ => name.to_owned(),
    }
}

/// The `oidc` settings of a configuration section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OidcSettings {
    issuer: String,
    client_id: String,
    scope: String,
}

impl OidcSettings {
    /// Reads the `oidc` settings of `config`, if it has them.
    ///
    /// # Errors
    ///
    /// Fails if `oidc` is not an object of the documented fields, or the
    /// issuer is not an HTTPS URL (HTTP only on a loopback address).
    pub(crate) fn from_config(config: &Configuration) -> anyhow::Result<Option<Self>> {
        match config.extra.get(OIDC_KEY) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Object(fields)) => Self::from_fields(fields).map(Some),
            Some(_) => anyhow::bail!("'{OIDC_KEY}' must be an object"),
        }
    }

    fn from_fields(fields: &Map<String, Value>) -> anyhow::Result<Self> {
        if let Some(key) = fields
            .keys()
            .find(|key| !matches!(key.as_str(), "issuer" | "client_id" | "scope"))
        {
            anyhow::bail!(
                "Unknown key '{}' in '{OIDC_KEY}'; expected issuer, client_id and scope",
                crate::printable(key)
            );
        }
        let string = |key: &str| -> anyhow::Result<Option<String>> {
            match fields.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
                Some(_) => anyhow::bail!("'{OIDC_KEY}.{key}' must be a non-empty string"),
            }
        };
        let issuer = string("issuer")?
            .ok_or_else(|| anyhow::anyhow!("'{OIDC_KEY}' must contain an 'issuer' string"))?;
        let client_id = string("client_id")?
            .ok_or_else(|| anyhow::anyhow!("'{OIDC_KEY}' must contain a 'client_id' string"))?;
        let scope = string("scope")?.unwrap_or_else(|| DEFAULT_SCOPE.to_owned());
        let url = Url::parse(&issuer)
            .map_err(|error| anyhow::anyhow!("Invalid '{OIDC_KEY}.issuer': {error}"))?;
        if url.query().is_some() || url.fragment().is_some() || !url.username().is_empty() {
            anyhow::bail!("'{OIDC_KEY}.issuer' must not have a query, fragment or user");
        }
        check_secure(&url, "issuer")?;
        Ok(Self {
            issuer,
            client_id,
            scope,
        })
    }

    /// The URL of the provider's discovery document.
    fn discovery_url(&self) -> anyhow::Result<Url> {
        let url = format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        );
        Ok(Url::parse(&url)?)
    }

    /// Whether a stored login belongs to these settings.
    fn matches(&self, login: &StoredLogin) -> bool {
        same_issuer(&login.issuer, &self.issuer) && login.client_id == self.client_id
    }
}

/// Compares issuers, ignoring one trailing slash.
fn same_issuer(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
}

/// Accepts HTTPS URLs, and HTTP URLs on a loopback address, so that tokens
/// and codes never cross the network unencrypted.
fn check_secure(url: &Url, what: &str) -> anyhow::Result<()> {
    let loopback = match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    };
    match url.scheme() {
        "https" => Ok(()),
        "http" if loopback => Ok(()),
        _ => anyhow::bail!("The OpenID Connect {what} must use HTTPS"),
    }
}

/// The tokens of one login, as stored in the token store.
#[derive(Clone, Serialize, Deserialize)]
struct StoredLogin {
    issuer: String,
    client_id: String,
    token_endpoint: String,
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    /// When the access token expires, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<u64>,
}

impl StoredLogin {
    /// Whether the access token is valid for at least [`EXPIRY_MARGIN`].
    fn is_fresh(&self, now: u64) -> bool {
        self.expires_at
            .is_none_or(|expires_at| now.saturating_add(EXPIRY_MARGIN.as_secs()) < expires_at)
    }

    /// The access token, reusable until [`EXPIRY_MARGIN`] before it expires.
    fn access_token(&self, now: u64) -> AccessToken {
        let token = AccessToken::new(self.access_token.clone());
        match self.expires_at {
            Some(expires_at) => {
                // Capped, so that a huge lifetime can't overflow `Instant`.
                let left = expires_at
                    .saturating_sub(now.saturating_add(EXPIRY_MARGIN.as_secs()))
                    .min(MAX_LIFETIME);
                token.reusable_until(Instant::now() + Duration::from_secs(left))
            }
            None => token,
        }
    }

    /// Takes the tokens of a token response, keeping the refresh token if the
    /// response has no new one.
    fn update(&mut self, response: TokenResponse, now: u64) {
        self.access_token = response.access_token;
        if response.refresh_token.is_some() {
            self.refresh_token = response.refresh_token;
        }
        self.expires_at = response
            .expires_in
            .map(|seconds| now.saturating_add(seconds.min(MAX_LIFETIME)));
    }
}

/// The token store: a JSON file with the stored logins.
#[derive(Default, Serialize, Deserialize)]
struct TokenStore {
    #[serde(default)]
    logins: Vec<StoredLogin>,
}

impl TokenStore {
    /// Reads the store at `path`; a missing file is an empty store.
    ///
    /// # Errors
    ///
    /// Fails if the file can't be read or parsed, or, on Unix, if others
    /// than its owner may read or write it.
    fn read(path: &Path) -> anyhow::Result<Self> {
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => anyhow::bail!("Cannot read token store {}: {error}", path.display()),
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path)?.permissions().mode();
            if mode & 0o077 != 0 {
                anyhow::bail!(
                    "Token store {} is accessible to other users; run 'chmod 600' on it",
                    path.display()
                );
            }
        }
        // The parser's error can quote the file, so it is left out.
        serde_json::from_str(&contents)
            .map_err(|_| anyhow::anyhow!("Token store {} is malformed", path.display()))
    }

    /// Writes the store to `path`, readable only by its owner, through a
    /// temporary file in the same directory, so that readers never see a
    /// partly written store.
    fn write(&self, path: &Path) -> anyhow::Result<()> {
        let dir = match path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        create_private_dir(dir)?;
        // `tempfile` creates the file with mode 0600 on Unix.
        let mut file = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer_pretty(&mut file, self)?;
        std::io::Write::write_all(&mut file, b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|error| {
            anyhow::anyhow!("Cannot write token store {}: {error}", path.display())
        })?;
        Ok(())
    }

    /// Replaces the login of the same issuer and client, or adds it.
    fn put(&mut self, login: StoredLogin) {
        self.logins.retain(|stored| {
            !(same_issuer(&stored.issuer, &login.issuer) && stored.client_id == login.client_id)
        });
        self.logins.push(login);
    }
}

/// An exclusive lock on the token store, held until it is dropped, so that
/// processes don't overwrite each other's changes. It locks a separate file,
/// `<store>.lock`, as the store itself is replaced on each write. Acquiring
/// it waits while another process holds it.
struct StoreLock {
    _file: std::fs::File,
}

impl StoreLock {
    fn acquire(store: &Path) -> anyhow::Result<Self> {
        let mut name = store.as_os_str().to_owned();
        name.push(".lock");
        let path = PathBuf::from(name);
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            create_private_dir(dir)?;
        }
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(false).write(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options
            .open(&path)
            .map_err(|error| anyhow::anyhow!("Cannot open {}: {error}", path.display()))?;
        file.lock()
            .map_err(|error| anyhow::anyhow!("Cannot lock {}: {error}", path.display()))?;
        Ok(Self { _file: file })
    }
}

/// Creates `dir` and its parents if missing; on Unix, new directories get
/// mode `0700`.
fn create_private_dir(dir: &Path) -> anyhow::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(dir)
        .map_err(|error| anyhow::anyhow!("Cannot create directory {}: {error}", dir.display()))
}

/// Seconds since the Unix epoch.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// The endpoints of the provider's discovery document that the device flow
/// needs.
struct Endpoints {
    device_authorization: Url,
    token: Url,
}

/// A successful token response (RFC 6749, 5.1).
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    refresh_token: Option<String>,
}

/// A device authorization response (RFC 8628, 3.2).
#[derive(Deserialize)]
struct DeviceAuthorization {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    verification_uri_complete: Option<String>,
    expires_in: u64,
    #[serde(default)]
    interval: Option<u64>,
}

/// An HTTP client for the provider: no redirects, so that codes and tokens
/// reach only the endpoints of the discovery document.
fn http_client() -> anyhow::Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(REQUEST_TIMEOUT)
        .build()?)
}

/// Reads the discovery document and checks its issuer and endpoints.
async fn discover(client: &reqwest::Client, settings: &OidcSettings) -> anyhow::Result<Endpoints> {
    let url = settings.discovery_url()?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("OpenID Connect discovery failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("OpenID Connect discovery failed with HTTP status {status}");
    }
    let document: Value = response
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("The OpenID Connect discovery document is not valid JSON"))?;
    let field = |key: &str| document.get(key).and_then(Value::as_str);
    // OpenID Connect Discovery 1.0, 4.3: the issuer must be the one asked for.
    match field("issuer") {
        Some(issuer) if same_issuer(issuer, &settings.issuer) => {}
        _ => anyhow::bail!(
            "The discovery document of {} is for another issuer",
            settings.issuer
        ),
    }
    let endpoint = |key: &str| -> anyhow::Result<Url> {
        let value = field(key).ok_or_else(|| {
            anyhow::anyhow!("The provider has no '{key}'; it may not support the device flow")
        })?;
        let url = Url::parse(value).map_err(|error| anyhow::anyhow!("Invalid '{key}': {error}"))?;
        check_secure(&url, key)?;
        Ok(url)
    };
    Ok(Endpoints {
        device_authorization: endpoint("device_authorization_endpoint")?,
        token: endpoint("token_endpoint")?,
    })
}

/// The outcome of one token request.
enum TokenOutcome {
    Tokens(TokenResponse),
    /// An OAuth error response with its `error` code (RFC 6749, 5.2).
    Error(String),
}

/// Sends a token request with `form` to `endpoint`.
///
/// Error responses are returned by their `error` code only, as other fields
/// are untrusted text.
async fn token_request(
    client: &reqwest::Client,
    endpoint: &Url,
    form: &[(&str, &str)],
) -> anyhow::Result<TokenOutcome> {
    let response = client
        .post(endpoint.clone())
        .form(form)
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("The token request failed: {error}"))?;
    let status = response.status();
    let body: Value = response.json().await.map_err(|_| {
        anyhow::anyhow!("The token response (HTTP status {status}) is not valid JSON")
    })?;
    if status.is_success() {
        let tokens: TokenResponse = serde_json::from_value(body)
            .map_err(|_| anyhow::anyhow!("The token response lacks an access token"))?;
        if !tokens.token_type.eq_ignore_ascii_case("bearer") {
            anyhow::bail!("The provider issued a token of a type other than Bearer");
        }
        if !valid_bearer(&tokens.access_token) {
            anyhow::bail!("The provider issued an access token that is not a valid bearer token");
        }
        return Ok(TokenOutcome::Tokens(tokens));
    }
    match body.get("error").and_then(Value::as_str) {
        Some(error) => Ok(TokenOutcome::Error(error.to_owned())),
        None => anyhow::bail!("The token request failed with HTTP status {status}"),
    }
}

/// Whether `token` can be sent as `Authorization: Bearer <token>`.
pub(crate) fn valid_bearer(token: &str) -> bool {
    !token.is_empty()
        && !token.bytes().any(|byte| byte.is_ascii_whitespace())
        && reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).is_ok()
}

/// How the device flow polls the token endpoint.
#[derive(Clone, Copy)]
struct Polling {
    /// The shortest interval, whatever the provider asks for.
    min_interval: Duration,
    /// How much longer to wait after `slow_down`.
    slow_down_step: Duration,
}

impl Polling {
    /// The polling of RFC 8628, with an interval of at least [`MIN_INTERVAL`].
    const STANDARD: Self = Self {
        min_interval: MIN_INTERVAL,
        slow_down_step: SLOW_DOWN_STEP,
    };
}

/// Runs the device flow: asks for a device code, writes the instructions to
/// `prompt`, and polls the token endpoint until the user has approved.
async fn device_flow(
    client: &reqwest::Client,
    settings: &OidcSettings,
    prompt: &mut dyn std::io::Write,
    polling: Polling,
) -> anyhow::Result<StoredLogin> {
    let endpoints = discover(client, settings).await?;
    let response = client
        .post(endpoints.device_authorization.clone())
        .form(&[
            ("client_id", settings.client_id.as_str()),
            ("scope", settings.scope.as_str()),
        ])
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("The device authorization request failed: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("The device authorization request failed with HTTP status {status}");
    }
    let device: DeviceAuthorization = response
        .json()
        .await
        .map_err(|_| anyhow::anyhow!("The device authorization response is malformed"))?;
    let mut message = format!(
        "To log in, open {} and enter the code {}\n",
        crate::printable(&device.verification_uri),
        crate::printable(&device.user_code)
    );
    if let Some(complete) = &device.verification_uri_complete {
        message.push_str(&format!("or open {}\n", crate::printable(complete)));
    }
    message.push_str("Waiting for approval...\n");
    prompt.write_all(message.as_bytes())?;
    prompt.flush()?;

    let deadline = Instant::now() + Duration::from_secs(device.expires_in.min(MAX_LIFETIME));
    let mut interval = Duration::from_secs(
        device
            .interval
            .unwrap_or(DEFAULT_INTERVAL)
            .min(MAX_LIFETIME),
    )
    .max(polling.min_interval);
    loop {
        tokio::time::sleep(interval).await;
        if Instant::now() >= deadline {
            anyhow::bail!("The login code has expired; run login again");
        }
        let outcome = token_request(
            client,
            &endpoints.token,
            &[
                ("grant_type", DEVICE_CODE_GRANT),
                ("device_code", device.device_code.as_str()),
                ("client_id", settings.client_id.as_str()),
            ],
        )
        .await?;
        match outcome {
            TokenOutcome::Tokens(tokens) => {
                let mut login = StoredLogin {
                    issuer: settings.issuer.clone(),
                    client_id: settings.client_id.clone(),
                    token_endpoint: endpoints.token.to_string(),
                    access_token: String::new(),
                    refresh_token: None,
                    expires_at: None,
                };
                login.update(tokens, now());
                return Ok(login);
            }
            TokenOutcome::Error(error) => match error.as_str() {
                "authorization_pending" => {}
                "slow_down" => interval += polling.slow_down_step,
                "access_denied" => anyhow::bail!("The login was denied"),
                "expired_token" => anyhow::bail!("The login code has expired; run login again"),
                other => anyhow::bail!(
                    "The provider rejected the login: {}",
                    crate::printable(other)
                ),
            },
        }
    }
}

/// The `oidc` settings of `section` in the context's selected profile.
fn section_settings(context: &CliContext, section: &str) -> anyhow::Result<OidcSettings> {
    let path = context.config_path()?;
    let profile = context.selected_profile()?;
    let config = Configuration::from_path_with_profile(&path, profile.as_deref(), section)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    OidcSettings::from_config(&config)?.ok_or_else(|| {
        anyhow::anyhow!(
            "Section '{}' has no '{OIDC_KEY}' settings",
            crate::printable(section)
        )
    })
}

/// Runs `login`, writing the instructions for the user to `prompt`.
pub(crate) async fn run_login(
    matches: &ArgMatches,
    context: &CliContext,
    prompt: &mut dyn std::io::Write,
) -> anyhow::Result<CommandOutput> {
    let section = section_name(matches.get_one::<String>("section").expect("required"));
    let settings = section_settings(context, &section)?;
    let path = context.token_store_path()?;
    // Fail on an unreadable store before the user approves the login.
    TokenStore::read(&path)?;
    let login = device_flow(&http_client()?, &settings, prompt, Polling::STANDARD).await?;
    let refreshable = login.refresh_token.is_some();
    // Read the store again: another process may have changed it meanwhile.
    let _lock = StoreLock::acquire(&path)?;
    let mut store = TokenStore::read(&path)?;
    store.put(login);
    store.write(&path)?;
    let mut message = format!(
        "Logged in to {} and stored the tokens in {}\n",
        crate::printable(&settings.issuer),
        crate::printable(&path.display().to_string())
    );
    if !refreshable {
        message.push_str(
            "The provider issued no refresh token, so you need to log in again when the token expires\n",
        );
    }
    Ok(stderr_output(message))
}

/// Successful output with `message` on standard error, as `login` and
/// `logout` print nothing to standard output.
fn stderr_output(message: String) -> CommandOutput {
    CommandOutput {
        stdout: String::new(),
        stderr: message,
        exit_code: std::process::ExitCode::SUCCESS,
    }
}

/// Runs `logout`.
pub(crate) fn run_logout(
    matches: &ArgMatches,
    context: &CliContext,
) -> anyhow::Result<CommandOutput> {
    let section = section_name(matches.get_one::<String>("section").expect("required"));
    let settings = section_settings(context, &section)?;
    let path = context.token_store_path()?;
    let _lock = StoreLock::acquire(&path)?;
    let mut store = TokenStore::read(&path)?;
    let before = store.logins.len();
    store.logins.retain(|login| !settings.matches(login));
    let message = if store.logins.len() == before {
        format!("Not logged in to {}\n", crate::printable(&settings.issuer))
    } else {
        store.write(&path)?;
        format!(
            "Deleted the stored tokens of {}; they stay valid at the provider until they expire\n",
            crate::printable(&settings.issuer)
        )
    };
    Ok(stderr_output(message))
}

/// A token provider for a section with `oidc` settings: it gives the stored
/// access token, refreshes it when it expires or the service rejects it, and
/// writes refreshed tokens back to the store.
pub(crate) struct StoredLoginProvider {
    /// The command that logs in again, such as `ga4gh-cli login TES`.
    login_command: String,
    path: PathBuf,
    settings: OidcSettings,
    login: tokio::sync::Mutex<StoredLogin>,
    client: reqwest::Client,
}

impl StoredLoginProvider {
    /// A provider with the stored login of `settings`, those of `section`.
    ///
    /// # Errors
    ///
    /// Fails if the store can't be read or has no login for `settings`.
    pub(crate) fn load(
        context: &CliContext,
        section: &str,
        settings: OidcSettings,
    ) -> anyhow::Result<Arc<Self>> {
        let login_command = format!("{} login {}", context.name(), crate::printable(section));
        let path = context.token_store_path()?;
        let store = TokenStore::read(&path)?;
        let login = store
            .logins
            .into_iter()
            .find(|login| settings.matches(login))
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Not logged in to {}; run '{login_command}'",
                    crate::printable(&settings.issuer)
                )
            })?;
        Ok(Arc::new(Self {
            login_command,
            path,
            settings,
            login: tokio::sync::Mutex::new(login),
            client: http_client()?,
        }))
    }

    /// Refreshes the access token if it has expired or expires soon.
    ///
    /// # Errors
    ///
    /// Fails if the refresh fails or the login can't be refreshed.
    pub(crate) async fn ensure_fresh(&self) -> anyhow::Result<()> {
        let mut login = self.login.lock().await;
        if login.is_fresh(now()) {
            return Ok(());
        }
        self.refresh(&mut login).await
    }

    async fn refresh(&self, login: &mut StoredLogin) -> anyhow::Result<()> {
        let relogin = || {
            anyhow::anyhow!(
                "The login to {} has expired; run '{}' again",
                crate::printable(&self.settings.issuer),
                self.login_command
            )
        };
        // Other processes refresh the same login: hold the store's lock, and
        // start from the stored tokens, as the provider may have rotated the
        // refresh token that this process read.
        let path = self.path.clone();
        // Waiting for the lock blocks, so it doesn't run on a runtime thread.
        let _lock = tokio::task::spawn_blocking(move || StoreLock::acquire(&path)).await??;
        let mut store = TokenStore::read(&self.path)?;
        let stored = store
            .logins
            .iter()
            .find(|stored| self.settings.matches(stored))
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Logged out of {}; run '{}' to log in again",
                    crate::printable(&self.settings.issuer),
                    self.login_command
                )
            })?;
        if stored.access_token != login.access_token && stored.is_fresh(now()) {
            *login = stored;
            return Ok(());
        }
        *login = stored;
        let refresh_token = login.refresh_token.clone().ok_or_else(relogin)?;
        let endpoint = Url::parse(&login.token_endpoint)?;
        check_secure(&endpoint, "token_endpoint")?;
        let outcome = token_request(
            &self.client,
            &endpoint,
            &[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token.as_str()),
                ("client_id", self.settings.client_id.as_str()),
            ],
        )
        .await?;
        let tokens = match outcome {
            TokenOutcome::Tokens(tokens) => tokens,
            // The refresh token is no longer valid (RFC 6749, 5.2).
            TokenOutcome::Error(error) if error == "invalid_grant" => return Err(relogin()),
            TokenOutcome::Error(error) => anyhow::bail!(
                "The provider refused to refresh the token: {}",
                crate::printable(&error)
            ),
        };
        login.update(tokens, now());
        store.put(login.clone());
        // The provider may have replaced the refresh token already, so the
        // new tokens are used even if they can't be stored.
        if let Err(error) = store.write(&self.path) {
            eprintln!(
                "Warning: {}; the next command may need a new login",
                crate::printable(&error.to_string())
            );
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl TokenProvider for StoredLoginProvider {
    async fn token(&self, request: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
        let mut login = self.login.lock().await;
        if request.after_rejection || !login.is_fresh(now()) {
            self.refresh(&mut login)
                .await
                .map_err(|error| -> BoxError { error.into() })?;
        }
        Ok(login.access_token(now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::mock;
    use serde_json::json;

    fn settings(issuer: &str) -> OidcSettings {
        OidcSettings::from_fields(
            json!({"issuer": issuer, "client_id": "cli"})
                .as_object()
                .unwrap(),
        )
        .unwrap()
    }

    fn discovery(prefix: &str) -> mockito::Mock {
        let issuer = format!("{}{prefix}", mockito::server_url());
        mock(
            "GET",
            format!("{prefix}/.well-known/openid-configuration").as_str(),
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

    fn device(prefix: &str) -> mockito::Mock {
        mock("POST", format!("{prefix}/device").as_str())
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("client_id".into(), "cli".into()),
                mockito::Matcher::UrlEncoded("scope".into(), DEFAULT_SCOPE.into()),
            ]))
            .with_body(
                json!({
                    "device_code": "device-code",
                    "user_code": "USER-CODE",
                    "verification_uri": "https://example.org/device",
                    "expires_in": 60,
                    "interval": 0,
                })
                .to_string(),
            )
            .create()
    }

    fn token_error(prefix: &str, error: &str) -> mockito::Mock {
        mock("POST", format!("{prefix}/token").as_str())
            .with_status(400)
            .with_body(json!({"error": error, "error_description": "\u{1b}[31m"}).to_string())
            .expect(1)
            .create()
    }

    #[test]
    fn settings_require_issuer_and_client_and_reject_plain_http() {
        let parse = |value: Value| OidcSettings::from_fields(value.as_object().unwrap());
        let ok = parse(json!({"issuer": "https://idp.example.org", "client_id": "c"})).unwrap();
        assert_eq!(ok.scope, DEFAULT_SCOPE);
        let scoped =
            parse(json!({"issuer": "http://127.0.0.1:1/", "client_id": "c", "scope": "openid"}))
                .unwrap();
        assert_eq!(scoped.scope, "openid");
        for (value, expected) in [
            (json!({"client_id": "c"}), "'issuer'"),
            (json!({"issuer": "https://idp.example.org"}), "'client_id'"),
            (
                json!({"issuer": "https://idp.example.org", "client_id": ""}),
                "client_id",
            ),
            (
                json!({"issuer": "http://idp.example.org", "client_id": "c"}),
                "HTTPS",
            ),
            (
                json!({"issuer": "https://idp.example.org?x=1", "client_id": "c"}),
                "query",
            ),
            (
                json!({"issuer": "https://idp.example.org", "client_id": "c", "client_secret": "s"}),
                "Unknown key",
            ),
            (json!({"issuer": "not a url", "client_id": "c"}), "Invalid"),
        ] {
            let error = parse(value).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn section_names_of_built_in_commands_are_uppercased() {
        assert_eq!(section_name("tes"), "TES");
        assert_eq!(section_name("registry"), "REGISTRY");
        assert_eq!(section_name("MY_service"), "MY_service");
    }

    #[tokio::test]
    async fn device_flow_polls_through_pending_and_slow_down() {
        let prefix = "/oidc-flow";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = discovery(prefix);
        let device = device(prefix);
        // mockito answers with the first matching mock that hasn't had its
        // expected calls yet, so the responses come in order of creation.
        let pending = token_error(prefix, "authorization_pending");
        let slow_down = token_error(prefix, "slow_down");
        let success = mock("POST", format!("{prefix}/token").as_str())
            .match_body(mockito::Matcher::AllOf(vec![
                mockito::Matcher::UrlEncoded("grant_type".into(), DEVICE_CODE_GRANT.into()),
                mockito::Matcher::UrlEncoded("device_code".into(), "device-code".into()),
                mockito::Matcher::UrlEncoded("client_id".into(), "cli".into()),
            ]))
            .with_body(
                json!({
                    "access_token": "access-1",
                    "token_type": "Bearer",
                    "expires_in": 300,
                    "refresh_token": "refresh-1",
                })
                .to_string(),
            )
            .expect(1)
            .create();
        let mut prompt = Vec::new();
        let started = Instant::now();
        let login = device_flow(
            &http_client().unwrap(),
            &settings(&issuer),
            &mut prompt,
            Polling {
                min_interval: Duration::ZERO,
                slow_down_step: Duration::from_millis(200),
            },
        )
        .await
        .unwrap();
        // `slow_down` made the last wait longer.
        assert!(started.elapsed() >= Duration::from_millis(200));
        let prompt = String::from_utf8(prompt).unwrap();
        assert!(prompt.contains("https://example.org/device"), "{prompt}");
        assert!(prompt.contains("USER-CODE"), "{prompt}");
        assert_eq!(login.access_token, "access-1");
        assert_eq!(login.refresh_token.as_deref(), Some("refresh-1"));
        assert_eq!(login.token_endpoint, format!("{issuer}/token"));
        assert!(login.expires_at.unwrap() > now() + 200);
        device.assert();
        pending.assert();
        slow_down.assert();
        success.assert();
    }

    #[tokio::test]
    async fn device_flow_reports_denial_without_the_description() {
        let prefix = "/oidc-denied";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = discovery(prefix);
        let _device = device(prefix);
        let _denied = token_error(prefix, "access_denied");
        let error = device_flow(
            &http_client().unwrap(),
            &settings(&issuer),
            &mut Vec::new(),
            FAST,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert_eq!(error, "The login was denied");
    }

    #[tokio::test]
    async fn device_flow_stops_when_the_code_expires() {
        let prefix = "/oidc-expired";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = discovery(prefix);
        let _device = device(prefix);
        let _expired = token_error(prefix, "expired_token");
        let error = device_flow(
            &http_client().unwrap(),
            &settings(&issuer),
            &mut Vec::new(),
            FAST,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("expired"), "{error}");
    }

    #[tokio::test]
    async fn discovery_rejects_another_issuer_and_missing_device_endpoint() {
        let prefix = "/oidc-other-issuer";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _other = mock(
            "GET",
            format!("{prefix}/.well-known/openid-configuration").as_str(),
        )
        .with_body(json!({"issuer": "https://elsewhere.example.org"}).to_string())
        .create();
        let error = discover(&http_client().unwrap(), &settings(&issuer))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("another issuer"), "{error}");

        let prefix = "/oidc-no-device";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _no_device = mock(
            "GET",
            format!("{prefix}/.well-known/openid-configuration").as_str(),
        )
        .with_body(
            json!({"issuer": issuer, "token_endpoint": format!("{issuer}/token")}).to_string(),
        )
        .create();
        let error = discover(&http_client().unwrap(), &settings(&issuer))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("device_authorization_endpoint"), "{error}");
    }

    #[test]
    fn stored_login_freshness_and_update() {
        let mut login = StoredLogin {
            issuer: "https://idp.example.org".into(),
            client_id: "c".into(),
            token_endpoint: "https://idp.example.org/token".into(),
            access_token: "a".into(),
            refresh_token: Some("r".into()),
            expires_at: Some(1_000),
        };
        assert!(login.is_fresh(900));
        assert!(!login.is_fresh(980));
        login.update(
            TokenResponse {
                access_token: "b".into(),
                token_type: "Bearer".into(),
                expires_in: None,
                refresh_token: None,
            },
            2_000,
        );
        assert_eq!(login.access_token, "b");
        assert_eq!(login.refresh_token.as_deref(), Some("r"));
        assert!(login.is_fresh(u64::MAX - 100));
    }

    #[cfg(unix)]
    #[test]
    fn token_store_is_private_and_rejects_wider_modes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("new-dir").join("tokens.json");
        let mut store = TokenStore::default();
        store.put(StoredLogin {
            issuer: "https://idp.example.org".into(),
            client_id: "c".into(),
            token_endpoint: "https://idp.example.org/token".into(),
            access_token: "a".into(),
            refresh_token: None,
            expires_at: None,
        });
        store.write(&path).unwrap();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(TokenStore::read(&path).unwrap().logins.len(), 1);

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let error = TokenStore::read(&path).err().unwrap().to_string();
        assert!(error.contains("chmod 600"), "{error}");
    }

    const FAST: Polling = Polling {
        min_interval: Duration::ZERO,
        slow_down_step: Duration::ZERO,
    };

    #[test]
    fn standard_polling_waits_at_least_a_second() {
        assert_eq!(Polling::STANDARD.min_interval, Duration::from_secs(1));
        assert_eq!(Polling::STANDARD.slow_down_step, Duration::from_secs(5));
    }

    #[tokio::test]
    async fn device_flow_stops_at_the_codes_lifetime_without_polling() {
        let prefix = "/oidc-lifetime";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = discovery(prefix);
        let _device = mock("POST", format!("{prefix}/device").as_str())
            .with_body(
                json!({
                    "device_code": "d",
                    "user_code": "U",
                    "verification_uri": "https://example.org/device",
                    "expires_in": 0,
                    "interval": 0,
                })
                .to_string(),
            )
            .create();
        let token = mock("POST", format!("{prefix}/token").as_str())
            .expect(0)
            .create();
        let error = device_flow(
            &http_client().unwrap(),
            &settings(&issuer),
            &mut Vec::new(),
            FAST,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("expired"), "{error}");
        token.assert();
    }

    #[tokio::test]
    async fn huge_lifetimes_from_the_provider_are_capped() {
        let prefix = "/oidc-huge";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = discovery(prefix);
        let _device = mock("POST", format!("{prefix}/device").as_str())
            .with_body(
                json!({
                    "device_code": "d",
                    "user_code": "U",
                    "verification_uri": "https://example.org/device",
                    "expires_in": u64::MAX,
                    "interval": 0,
                })
                .to_string(),
            )
            .create();
        let _token = mock("POST", format!("{prefix}/token").as_str())
            .with_body(
                json!({"access_token": "a", "token_type": "Bearer", "expires_in": u64::MAX})
                    .to_string(),
            )
            .create();
        let login = device_flow(
            &http_client().unwrap(),
            &settings(&issuer),
            &mut Vec::new(),
            FAST,
        )
        .await
        .unwrap();
        assert!(login.expires_at.unwrap() <= now() + MAX_LIFETIME);
        assert!(login.access_token(now()).reuse_deadline().is_some());
        assert!(login.refresh_token.is_none());
    }

    #[tokio::test]
    async fn tokens_of_another_type_are_rejected() {
        let prefix = "/oidc-dpop";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = discovery(prefix);
        let _device = device(prefix);
        let _token = mock("POST", format!("{prefix}/token").as_str())
            .with_body(json!({"access_token": "a", "token_type": "DPoP"}).to_string())
            .create();
        let error = device_flow(
            &http_client().unwrap(),
            &settings(&issuer),
            &mut Vec::new(),
            FAST,
        )
        .await
        .err()
        .unwrap()
        .to_string();
        assert!(error.contains("other than Bearer"), "{error}");
    }

    #[tokio::test]
    async fn discovery_rejects_plain_http_endpoints() {
        let prefix = "/oidc-plain-http";
        let issuer = format!("{}{prefix}", mockito::server_url());
        let _discovery = mock(
            "GET",
            format!("{prefix}/.well-known/openid-configuration").as_str(),
        )
        .with_body(
            json!({
                "issuer": issuer,
                "device_authorization_endpoint": "http://idp.example.org/device",
                "token_endpoint": format!("{issuer}/token"),
            })
            .to_string(),
        )
        .create();
        let error = discover(&http_client().unwrap(), &settings(&issuer))
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("must use HTTPS"), "{error}");
    }
}
