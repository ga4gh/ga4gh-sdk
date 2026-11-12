use crate::clients::ServiceType;
use crate::utils::timeouts::Timeouts;
use serde_json::{Map, Value};
use std::fmt;
use std::path::Path;
use url::Url;

/// Printed in `Debug` output in place of a value that is set but not meant to be shown.
struct Redacted;

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

fn redact(value: &Option<String>) -> Option<Redacted> {
    value.as_ref().map(|_| Redacted)
}

/// Prints the keys of a map, each with a redacted value.
struct RedactedValues<'a>(&'a Map<String, Value>);

impl fmt::Debug for RedactedValues<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key, Redacted)))
            .finish()
    }
}

/// The keys of a configuration file section that `Configuration::from_path` reads itself.
const SECTION_KEYS: [&str; 8] = [
    "base_path",
    "basic_auth",
    "oauth_access_token",
    "oauth_access_token_env",
    "bearer_access_token",
    "bearer_access_token_env",
    "api_key",
    "timeouts",
];

/// Reads the `timeouts` object of a configuration file section: each key is a
/// limit of [`Timeouts`] in seconds; missing keys keep their defaults.
fn timeouts_from_value(value: &Value) -> Result<Timeouts, String> {
    let fields = value
        .as_object()
        .ok_or("timeouts must be an object of numbers of seconds")?;
    let mut timeouts = Timeouts::default();
    for (key, value) in fields {
        // A non-number is as invalid as a negative one; NaN gives the same message.
        timeouts
            .set_seconds(key, value.as_f64().unwrap_or(f64::NAN))
            .map_err(|error| format!("Invalid timeouts: {error}"))?;
    }
    Ok(timeouts)
}

/// Reads a token from the environment variable `name`.
fn token_from_env(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|error| var_error_message(name, &error))
}

/// Read one direct token or an explicitly named environment variable.
fn token_field(
    fields: &Map<String, Value>,
    direct: &'static str,
    env_field: &'static str,
    section: &str,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let token = match fields.get(direct) {
        None | Some(Value::Null) => None,
        Some(Value::String(token)) => Some(token.clone()),
        _ => return Err(format!("{direct} must be a string").into()),
    };
    match fields.get(env_field) {
        None | Some(Value::Null) => Ok(token),
        Some(Value::String(_)) if token.is_some() => {
            Err(format!("Set either {direct} or {env_field}, not both").into())
        }
        Some(Value::String(name)) => token_from_env(name)
            .map(Some)
            .map_err(|error| format!("{error}; it is named by {env_field} in '{section}'").into()),
        _ => Err(format!("{env_field} must be a string").into()),
    }
}

/// Describes why the environment variable `name` can't be read, without its value.
fn var_error_message(name: &str, error: &std::env::VarError) -> String {
    match error {
        std::env::VarError::NotPresent => format!("Environment variable '{name}' is not set"),
        std::env::VarError::NotUnicode(_) => {
            format!("Environment variable '{name}' is not valid Unicode")
        }
    }
}

/// Returns the URL without its user, password, query and fragment. The path stays.
///
/// Without an authority, as in `user:value@example.org` with no scheme, the parts can't be told
/// apart, so only the scheme stays.
fn display_url(url: &Url) -> String {
    if url.cannot_be_a_base() {
        return format!("{}:<redacted>", url.scheme());
    }
    let mut url = url.clone();
    // These fail only for URLs without a host, which hold neither value.
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    url.into()
}

/// A struct representing a configuration for the SDK.
///
/// The `Configuration` struct is responsible for specifying details of the Endpoint where the requests are made.
/// It provides methods for making constructing new configuration, changing the base url, and specifying a default configuration.
///
/// Build it with `new()`, `default()`, `from_file()` or `from_path()` and the `with_*`
/// methods rather than a struct literal, which breaks when a field is added.
#[derive(Clone)]
pub struct Configuration {
    /// The API root; endpoint paths are resolved below it, with or without a trailing slash.
    /// Query strings and fragments are not supported in the API root.
    pub base_path: Url,
    /// The user agent to be used in API requests.
    pub user_agent: Option<String>,
    /// The basic authentication credentials.
    pub basic_auth: Option<BasicAuth>,
    /// The OAuth access token for authentication.
    pub oauth_access_token: Option<String>,
    /// The bearer access token for authentication.
    pub bearer_access_token: Option<String>,
    /// The API key for authentication.
    pub api_key: Option<ApiKey>,
    /// Other keys of the configuration file section, with their JSON values, for
    /// settings that the SDK doesn't know, such as those of a downstream crate.
    /// They can hold credentials, so `Debug` output shows only their keys.
    pub extra: Map<String, Value>,
    /// The time limits of requests; see [`Timeouts`].
    pub timeouts: Timeouts,
}

/// Represents the basic authentication credentials.
#[derive(Clone, PartialEq)]
pub struct BasicAuth {
    /// The username for basic authentication.
    pub username: String,
    /// The password for basic authentication.
    pub password: Option<String>,
}

/// Represents the API key for authentication.
#[derive(Clone, PartialEq)]
pub struct ApiKey {
    /// The prefix for the API key.
    pub prefix: Option<String>,
    /// The key value of the API key.
    pub key: String,
}

/// An invalid or ambiguous configured credential. No credential value is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthError {
    /// More than one credential source was configured.
    Conflicting { sources: Vec<&'static str> },
    /// The SDK has no request-header contract for this source.
    Unsupported { source: &'static str },
    /// The bearer value cannot be sent as an HTTP bearer token.
    InvalidBearer { source: &'static str },
    /// A Basic username cannot contain the separator used by Basic auth.
    InvalidBasicUsername,
    /// A caller-owned client and configured credentials require explicit opt-in.
    CallerOwnedClient,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflicting { sources } => {
                write!(f, "Conflicting authentication sources: {}", sources.join(", "))
            }
            Self::Unsupported { source } => write!(f, "Unsupported authentication source: {source}"),
            Self::InvalidBearer { source } => write!(f, "Invalid bearer token in {source}"),
            Self::InvalidBasicUsername => f.write_str("Basic auth username must not contain ':'"),
            Self::CallerOwnedClient => f.write_str(
                "Configured credentials cannot be applied to a caller-owned HTTP client without explicit opt-in",
            ),
        }
    }
}

impl std::error::Error for AuthError {}

/// The one supported credential selected for a request.
pub(crate) enum ConfiguredAuth<'a> {
    None,
    Basic(&'a BasicAuth),
    Bearer(&'a str),
}

impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Listing all fields makes the compiler ask how to print any new one.
        let Configuration {
            base_path,
            user_agent,
            basic_auth,
            oauth_access_token,
            bearer_access_token,
            api_key,
            extra,
            timeouts,
        } = self;
        f.debug_struct("Configuration")
            .field("base_path", &display_url(base_path))
            .field("user_agent", user_agent)
            .field("basic_auth", basic_auth)
            .field("oauth_access_token", &redact(oauth_access_token))
            .field("bearer_access_token", &redact(bearer_access_token))
            .field("api_key", api_key)
            .field("extra", &RedactedValues(extra))
            .field("timeouts", timeouts)
            .finish()
    }
}

impl fmt::Debug for BasicAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let BasicAuth {
            username: _,
            password,
        } = self;
        f.debug_struct("BasicAuth")
            .field("username", &Redacted)
            .field("password", &redact(password))
            .finish()
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let ApiKey { prefix, key: _ } = self;
        f.debug_struct("ApiKey")
            .field("prefix", prefix)
            .field("key", &Redacted)
            .finish()
    }
}

impl Configuration {
    /// Creates a new instance of Configuration.
    ///
    /// # Arguments
    ///
    /// * `base_path` - The API root for requests, such as `https://host/ga4gh/tes/v1`,
    ///   without a query string or fragment.
    /// * `user_agent` - The user agent to be used in API requests.
    /// * `basic_auth` - The basic authentication credentials.
    /// * `oauth_access_token` - The OAuth access token for authentication.
    ///
    /// # Returns
    ///
    /// A new instance of Configuration.
    pub fn new(base_path: Url) -> Self {
        Configuration {
            base_path,
            user_agent: Some("GA4GH SDK".to_owned()),
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: None,
            api_key: None,
            extra: Map::new(),
            timeouts: Timeouts::default(),
        }
    }

    /// Sets the API root for requests.
    ///
    /// # Arguments
    ///
    /// * `base_path` - The API root, with or without a trailing slash.
    ///
    /// # Returns
    ///
    /// A mutable reference to the Configuration instance.
    pub fn set_base_path(&mut self, base_path: Url) -> &mut Self {
        self.base_path = base_path;
        self
    }

    /// Sets the user agent for API requests.
    ///
    /// # Arguments
    ///
    /// * `user_agent` - The user agent to be used in API requests.
    ///
    /// # Returns
    ///
    /// A new instance of Configuration.
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = Some(user_agent);
        self
    }

    /// Sets the basic authentication credentials for API requests.
    ///
    /// # Arguments
    ///
    /// * `basic_auth` - The basic authentication credentials.
    ///
    /// # Returns
    ///
    /// A new instance of Configuration.
    pub fn with_basic_auth(mut self, basic_auth: BasicAuth) -> Self {
        self.basic_auth = Some(basic_auth);
        self
    }

    /// Sets the OAuth access token for API requests.
    ///
    /// # Arguments
    ///
    /// * `oauth_access_token` - The OAuth access token for authentication.
    ///
    /// # Returns
    ///
    /// A new instance of Configuration.
    pub fn with_oauth_access_token(mut self, oauth_access_token: String) -> Self {
        self.oauth_access_token = Some(oauth_access_token);
        self
    }

    /// Select exactly one credential source. Validate programmatic changes on
    /// each request, because the configuration fields are public.
    pub(crate) fn configured_auth(&self) -> Result<ConfiguredAuth<'_>, AuthError> {
        let mut sources = Vec::new();
        if self.basic_auth.is_some() {
            sources.push("basic_auth");
        }
        if self.oauth_access_token.is_some() {
            sources.push("oauth_access_token");
        }
        if self.bearer_access_token.is_some() {
            sources.push("bearer_access_token");
        }
        if self.api_key.is_some() {
            sources.push("api_key");
        }
        if !self.base_path.username().is_empty() || self.base_path.password().is_some() {
            sources.push("base_path userinfo");
        }
        if sources.len() > 1 {
            return Err(AuthError::Conflicting { sources });
        }
        if self.api_key.is_some() {
            return Err(AuthError::Unsupported { source: "api_key" });
        }
        if sources.contains(&"base_path userinfo") {
            return Err(AuthError::Unsupported {
                source: "base_path userinfo",
            });
        }
        if let Some(basic) = &self.basic_auth {
            if basic.username.contains(':') {
                return Err(AuthError::InvalidBasicUsername);
            }
            return Ok(ConfiguredAuth::Basic(basic));
        }
        for (source, token) in [
            ("oauth_access_token", self.oauth_access_token.as_deref()),
            ("bearer_access_token", self.bearer_access_token.as_deref()),
        ] {
            if let Some(token) = token {
                if token.is_empty()
                    || token.bytes().any(|byte| byte.is_ascii_whitespace())
                    || reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).is_err()
                {
                    return Err(AuthError::InvalidBearer { source });
                }
                return Ok(ConfiguredAuth::Bearer(token));
            }
        }
        Ok(ConfiguredAuth::None)
    }

    /// Sets the OAuth access token from an environment variable that the caller names.
    ///
    /// # Arguments
    ///
    /// * `name` - The name of the environment variable that holds the token.
    ///
    /// # Errors
    ///
    /// Returns an error naming the variable if it isn't set or isn't valid Unicode.
    pub fn with_oauth_access_token_from_env(
        self,
        name: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(self.with_oauth_access_token(token_from_env(name)?))
    }

    /// Sets a generic bearer access token for API requests.
    pub fn with_bearer_access_token(mut self, token: String) -> Self {
        self.bearer_access_token = Some(token);
        self
    }

    /// Sets a generic bearer token from an explicitly named environment variable.
    pub fn with_bearer_access_token_from_env(
        self,
        name: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(self.with_bearer_access_token(token_from_env(name)?))
    }

    /// Sets the time limits of requests; see [`Timeouts`].
    ///
    /// # Arguments
    ///
    /// * `timeouts` - The time limits.
    ///
    /// # Returns
    ///
    /// A new instance of Configuration.
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Loads the configuration from a JSON file.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of service to load the configuration for; its
    ///   section name is `service_type.as_str()`, such as `TES`, or the name of a
    ///   `ServiceType::Custom` section.
    ///
    /// # Errors
    ///
    /// This function will return an error if the configuration file is missing or malformed.
    pub fn from_file(service_type: ServiceType) -> Result<Self, Box<dyn std::error::Error>> {
        let path = dirs::home_dir()
            .ok_or("Home directory not found")?
            .join(".ga4gh-cli/config.json");
        Self::from_path(path, service_type.as_str())
    }

    /// Load one named section from a JSON configuration file.
    ///
    /// Only `base_path` is required; authentication fields are optional.
    /// `oauth_access_token_env` and `bearer_access_token_env` name environment
    /// variables to read a token from. Configure one supported source only;
    /// `api_key` and URL userinfo are rejected. `timeouts` is an object with
    /// `connect`, `request`, `discovery` and `submission` in seconds; missing
    /// ones keep their defaults (see [`Timeouts`]). Other keys of the section go
    /// into `extra`. A missing file is an I/O error; malformed content is an
    /// input error.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use ga4gh_sdk::utils::configuration::Configuration;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let config = Configuration::from_path("config.json", "TES")?;
    /// # let _ = config;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_path(
        path: impl AsRef<Path>,
        section: &str,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.as_ref();
        let contents = std::fs::read_to_string(path).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("Cannot read configuration file {}: {error}", path.display()),
            )
        })?;
        let document: Value = serde_json::from_str(&contents).map_err(|error| {
            format!(
                "Invalid JSON in configuration file {}: {error}",
                path.display()
            )
        })?;
        let root = document
            .as_object()
            .ok_or("Configuration file must be a JSON object")?;
        let fields = root
            .get(section)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("Configuration file must contain a '{section}' object"))?;
        let base_path = fields
            .get("base_path")
            .and_then(Value::as_str)
            .ok_or("Configuration section must contain a 'base_path' string")?;
        let base_path = Url::parse(base_path)
            .map_err(|error| format!("Invalid base_path in '{section}' configuration: {error}"))?;
        let mut config = Configuration::new(base_path);

        match fields.get("basic_auth") {
            None | Some(Value::Null) => {}
            Some(Value::Object(auth)) => {
                let username = auth
                    .get("username")
                    .and_then(Value::as_str)
                    .ok_or("basic_auth.username must be a string")?;
                let password = match auth.get("password") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(password)) => Some(password.clone()),
                    _ => return Err("basic_auth.password must be a string".into()),
                };
                config.basic_auth = Some(BasicAuth {
                    username: username.to_owned(),
                    password,
                });
            }
            _ => return Err("basic_auth must be an object".into()),
        }
        config.oauth_access_token = token_field(
            fields,
            "oauth_access_token",
            "oauth_access_token_env",
            section,
        )?;
        config.bearer_access_token = token_field(
            fields,
            "bearer_access_token",
            "bearer_access_token_env",
            section,
        )?;
        if fields.get("api_key").is_some_and(|value| !value.is_null()) {
            return Err(AuthError::Unsupported { source: "api_key" }.into());
        }
        match fields.get("timeouts") {
            None | Some(Value::Null) => {}
            Some(value) => config.timeouts = timeouts_from_value(value)?,
        }
        config.extra = fields
            .iter()
            .filter(|(key, _)| !SECTION_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        config.configured_auth()?;
        Ok(config)
    }
}

impl Default for Configuration {
    /// Creates a default instance of Configuration.
    ///
    /// # Returns
    ///
    /// A default instance of Configuration.
    /// This is used to define a configuration for a server that is running on your localhost
    fn default() -> Self {
        Configuration {
            base_path: Url::parse("http://localhost").unwrap(),
            user_agent: Some("GA4GH SDK".to_owned()),
            basic_auth: None,
            oauth_access_token: None,
            bearer_access_token: None,
            api_key: None,
            extra: Map::new(),
            timeouts: Timeouts::default(),
        }
    }
}

/// A configuration with a marker value in everything that isn't meant to be shown.
#[cfg(test)]
pub(crate) fn config_with_markers() -> Configuration {
    let base_path = "https://SENTINEL_USER:SENTINEL_PASSWORD@api.example.com/v1/?key=SENTINEL_QUERY#SENTINEL_FRAGMENT";
    let mut config = Configuration::new(Url::parse(base_path).unwrap())
        .with_basic_auth(BasicAuth {
            username: "SENTINEL_USER".to_owned(),
            password: Some("SENTINEL_PASSWORD".to_owned()),
        })
        .with_oauth_access_token("SENTINEL_OAUTH".to_owned());
    config.bearer_access_token = Some("SENTINEL_BEARER".to_owned());
    config.api_key = Some(ApiKey {
        prefix: Some("Token".to_owned()),
        key: "SENTINEL_KEY".to_owned(),
    });
    config.extra.insert(
        "client_secret".to_owned(),
        Value::String("SENTINEL_EXTRA".to_owned()),
    );
    config
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;

    #[test]
    fn test_from_path_minimal_and_optional_auth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org/tes/v1"}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(config.base_path.as_str(), "https://example.org/tes/v1");
        assert!(config.basic_auth.is_none());
        assert!(config.oauth_access_token.is_none());

        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org/tes/v1","basic_auth":{"username":"alice"}}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(config.basic_auth.as_ref().unwrap().username, "alice");
        assert_eq!(config.basic_auth.as_ref().unwrap().password, None);
        assert!(config.oauth_access_token.is_none());
    }

    #[test]
    fn test_from_path_reads_service_sections_with_one_credential() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
                "TES": {
                    "base_path": "https://example.org/ga4gh/tes/v1",
                    "basic_auth": {"username": "alice", "password": "change-me"}
                },
                "WES": {"base_path": "https://example.org/ga4gh/wes/v1"}
            }"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, ServiceType::TES.as_str()).unwrap();
        assert_eq!(
            config.base_path.as_str(),
            "https://example.org/ga4gh/tes/v1"
        );
        assert_eq!(
            config.basic_auth,
            Some(BasicAuth {
                username: "alice".to_owned(),
                password: Some("change-me".to_owned()),
            })
        );
        assert!(config.oauth_access_token.is_none());
        assert!(config.extra.is_empty(), "{:?}", config.extra);

        let config = Configuration::from_path(&path, ServiceType::WES.as_str()).unwrap();
        assert_eq!(
            config.base_path.as_str(),
            "https://example.org/ga4gh/wes/v1"
        );
        assert!(config.extra.is_empty(), "{:?}", config.extra);
    }

    #[test]
    fn test_from_path_custom_section_keeps_extra_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
                "TES": {"base_path": "https://example.org/ga4gh/tes/v1"},
                "PROXY": {
                    "base_path": "https://example.org/proxy/v1",
                    "oauth_access_token": "token",
                    "timeout_seconds": 30,
                    "routing": {"region": "eu"}
                }
            }"#,
        )
        .unwrap();
        let service_type = ServiceType::Custom("PROXY".to_owned());
        let config = Configuration::from_path(&path, service_type.as_str()).unwrap();
        assert_eq!(config.base_path.as_str(), "https://example.org/proxy/v1");
        assert_eq!(config.oauth_access_token.as_deref(), Some("token"));
        let mut keys: Vec<&str> = config.extra.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["routing", "timeout_seconds"]);
        assert_eq!(config.extra["timeout_seconds"], serde_json::json!(30));
        assert_eq!(config.extra["routing"], serde_json::json!({"region": "eu"}));
    }

    #[test]
    fn test_from_path_reads_token_from_named_environment_variable() {
        // PATH is set wherever tests run; setting a variable here would race with
        // other test threads that read the environment.
        let expected = std::env::var("PATH").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org","oauth_access_token_env":"PATH"}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(config.oauth_access_token, Some(expected.clone()));
        assert!(config.extra.is_empty(), "{:?}", config.extra);

        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org","bearer_access_token_env":"PATH"}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(
            config.bearer_access_token,
            Some(std::env::var("PATH").unwrap())
        );
        assert!(config.extra.is_empty());

        // `null` means unset, for both keys.
        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org","oauth_access_token":null,"oauth_access_token_env":"PATH"}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(config.oauth_access_token, Some(expected));
        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org","oauth_access_token":"token","oauth_access_token_env":null}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(config.oauth_access_token.as_deref(), Some("token"));
        assert!(config.extra.is_empty(), "{:?}", config.extra);
    }

    #[test]
    fn test_var_error_message_hides_value() {
        let error = std::env::VarError::NotUnicode("SENTINEL_TOKEN".into());
        assert_eq!(
            var_error_message("MY_TOKEN", &error),
            "Environment variable 'MY_TOKEN' is not valid Unicode"
        );
    }

    #[test]
    fn test_with_oauth_access_token_from_env() {
        let config = Configuration::default()
            .with_oauth_access_token_from_env("PATH")
            .unwrap();
        assert_eq!(
            config.oauth_access_token,
            Some(std::env::var("PATH").unwrap())
        );

        let error = Configuration::default()
            .with_oauth_access_token_from_env("GA4GH_SDK_TEST_TOKEN_BUILDER_UNSET")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Environment variable 'GA4GH_SDK_TEST_TOKEN_BUILDER_UNSET' is not set"
        );
    }

    #[test]
    fn test_from_path_reads_timeouts() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{"custom":{"base_path":"https://example.org","timeouts":{"discovery":30,"submission":0.5}}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(
            config.timeouts,
            Timeouts {
                discovery: Duration::from_secs(30),
                submission: Duration::from_millis(500),
                ..Default::default()
            }
        );
        assert!(config.extra.is_empty(), "{:?}", config.extra);

        for document in [
            r#"{"custom":{"base_path":"https://example.org"}}"#,
            r#"{"custom":{"base_path":"https://example.org","timeouts":null}}"#,
            r#"{"custom":{"base_path":"https://example.org","timeouts":{}}}"#,
        ] {
            std::fs::write(&path, document).unwrap();
            let config = Configuration::from_path(&path, "custom").unwrap();
            assert_eq!(config.timeouts, Timeouts::default(), "{document}");
        }
    }

    #[test]
    fn test_with_timeouts() {
        let timeouts = Timeouts {
            connect: std::time::Duration::from_secs(1),
            ..Default::default()
        };
        assert_eq!(
            Configuration::default().with_timeouts(timeouts).timeouts,
            timeouts
        );
    }

    #[test]
    fn test_from_path_invalid_documents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let cases = [
            ("not JSON", "Invalid JSON"),
            ("[]", "JSON object"),
            ("{}", "'custom' object"),
            (r#"{"custom":{}}"#, "base_path"),
            (
                r#"{"custom":{"base_path":"not a url"}}"#,
                "Invalid base_path",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","basic_auth":{}}}"#,
                "basic_auth.username",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","oauth_access_token":3}}"#,
                "oauth_access_token",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","oauth_access_token_env":3}}"#,
                "oauth_access_token_env must be a string",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","oauth_access_token":"token","oauth_access_token_env":"GA4GH_SDK_TEST_TOKEN_BOTH"}}"#,
                "not both",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","basic_auth":{"username":"alice"},"oauth_access_token":"token"}}"#,
                "Conflicting authentication sources",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","bearer_access_token":3}}"#,
                "bearer_access_token must be a string",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","bearer_access_token":"one","bearer_access_token_env":"PATH"}}"#,
                "not both",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","api_key":{"key":"secret"}}}"#,
                "Unsupported authentication source: api_key",
            ),
            (
                r#"{"custom":{"base_path":"https://user:secret@example.org"}}"#,
                "Unsupported authentication source: base_path userinfo",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","timeouts":30}}"#,
                "timeouts must be an object",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","timeouts":{"read":30}}}"#,
                "Invalid timeouts: unknown timeout 'read'",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","timeouts":{"request":"30"}}}"#,
                "Invalid timeouts: request must be a positive number of seconds",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","timeouts":{"connect":0}}}"#,
                "Invalid timeouts: connect must be a positive number of seconds",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","timeouts":{"discovery":-1}}}"#,
                "Invalid timeouts: discovery must be a positive number of seconds",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","timeouts":{"submission":1e300}}}"#,
                "Invalid timeouts: submission must be a positive number of seconds",
            ),
            (
                r#"{"custom":{"base_path":"https://example.org","oauth_access_token_env":"GA4GH_SDK_TEST_TOKEN_UNSET"}}"#,
                "Environment variable 'GA4GH_SDK_TEST_TOKEN_UNSET' is not set; it is named by oauth_access_token_env in 'custom'",
            ),
        ];
        for (document, expected_message) in cases {
            std::fs::write(&path, document).unwrap();
            let error = Configuration::from_path(&path, "custom").unwrap_err();
            assert!(
                error.to_string().contains(expected_message),
                "{document}: {error}"
            );
        }

        let missing = dir.path().join("missing.json");
        let error = Configuration::from_path(&missing, "custom").unwrap_err();
        assert!(error.to_string().contains("missing.json"), "{error}");
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    #[test]
    fn test_debug_output_basic_auth() {
        let basic_auth = BasicAuth {
            username: "SENTINEL_USER".to_owned(),
            password: Some("SENTINEL_PASSWORD".to_owned()),
        };
        assert_eq!(
            format!("{:?}", basic_auth),
            "BasicAuth { username: <redacted>, password: Some(<redacted>) }"
        );

        let basic_auth = BasicAuth {
            username: "SENTINEL_USER".to_owned(),
            password: None,
        };
        assert_eq!(
            format!("{:?}", basic_auth),
            "BasicAuth { username: <redacted>, password: None }"
        );
    }

    #[test]
    fn test_debug_output_api_key() {
        let api_key = ApiKey {
            prefix: Some("Token".to_owned()),
            key: "SENTINEL_KEY".to_owned(),
        };
        assert_eq!(
            format!("{:?}", api_key),
            "ApiKey { prefix: Some(\"Token\"), key: <redacted> }"
        );
    }

    #[test]
    fn test_debug_output_configuration() {
        assert_eq!(
            format!("{:?}", config_with_markers()),
            "Configuration { \
             base_path: \"https://api.example.com/v1/\", \
             user_agent: Some(\"GA4GH SDK\"), \
             basic_auth: Some(BasicAuth { username: <redacted>, password: Some(<redacted>) }), \
             oauth_access_token: Some(<redacted>), \
             bearer_access_token: Some(<redacted>), \
             api_key: Some(ApiKey { prefix: Some(\"Token\"), key: <redacted> }), \
             extra: {\"client_secret\": <redacted>}, \
             timeouts: Timeouts { connect: 10s, request: 60s, discovery: 10s, submission: 300s } }"
        );
    }

    #[test]
    fn test_debug_output_configuration_with_defaults() {
        assert_eq!(
            format!("{:?}", Configuration::default()),
            "Configuration { \
             base_path: \"http://localhost/\", \
             user_agent: Some(\"GA4GH SDK\"), \
             basic_auth: None, \
             oauth_access_token: None, \
             bearer_access_token: None, \
             api_key: None, \
             extra: {}, \
             timeouts: Timeouts { connect: 10s, request: 60s, discovery: 10s, submission: 300s } }"
        );
    }

    #[test]
    fn test_debug_output_pretty() {
        let output = format!("{:#?}", config_with_markers());
        assert!(!output.contains("SENTINEL"), "{output}");
        assert!(
            output.contains("oauth_access_token: Some(\n        <redacted>,\n    )"),
            "{output}"
        );
    }

    #[test]
    fn test_debug_output_base_path_without_authority() {
        let config =
            Configuration::new(Url::parse("user:SENTINEL_PASSWORD@example.org/v1").unwrap());
        let output = format!("{:?}", config);
        assert!(!output.contains("SENTINEL"), "{output}");
        assert!(
            output.contains("base_path: \"user:<redacted>\""),
            "{output}"
        );
    }

    #[test]
    fn test_new_configuration() {
        let config = Configuration::new(Url::parse("https://api.example.com").unwrap())
            .with_user_agent("My User Agent".to_owned())
            .with_basic_auth(BasicAuth {
                username: "admin".to_owned(),
                password: Some("password".to_owned()),
            })
            .with_oauth_access_token("my_oauth_token".to_owned());

        assert_eq!(config.base_path.as_str(), "https://api.example.com/");
        assert_eq!(config.user_agent, Some("My User Agent".to_owned()));
        assert_eq!(
            config.basic_auth,
            Some(BasicAuth {
                username: "admin".to_owned(),
                password: Some("password".to_owned()),
            })
        );
        assert_eq!(config.oauth_access_token, Some("my_oauth_token".to_owned()));
        assert_eq!(config.bearer_access_token, None);
        assert_eq!(config.api_key, None);
    }

    #[test]
    fn test_set_base_path() {
        let mut config = Configuration::default();
        config.set_base_path(Url::parse("https://api.example.com").unwrap());
        assert_eq!(config.base_path.as_str(), "https://api.example.com/");
    }
}
