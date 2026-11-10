use crate::clients::ServiceType;
use serde_json::Value;
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
        } = self;
        f.debug_struct("Configuration")
            .field("base_path", &display_url(base_path))
            .field("user_agent", user_agent)
            .field("basic_auth", basic_auth)
            .field("oauth_access_token", &redact(oauth_access_token))
            .field("bearer_access_token", &redact(bearer_access_token))
            .field("api_key", api_key)
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

    /// Loads the configuration from a JSON file.
    ///
    /// # Arguments
    ///
    /// * `service_type` - The type of service to load the configuration for.
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
    /// A missing file is an I/O error; malformed content is an input error.
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
        match fields.get("oauth_access_token") {
            None | Some(Value::Null) => {}
            Some(Value::String(token)) => config.oauth_access_token = Some(token.clone()),
            _ => return Err("oauth_access_token must be a string".into()),
        }
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
            r#"{"custom":{"base_path":"https://example.org/tes/v1","basic_auth":{"username":"alice"},"oauth_access_token":"token"}}"#,
        )
        .unwrap();
        let config = Configuration::from_path(&path, "custom").unwrap();
        assert_eq!(config.basic_auth.as_ref().unwrap().username, "alice");
        assert_eq!(config.basic_auth.as_ref().unwrap().password, None);
        assert_eq!(config.oauth_access_token.as_deref(), Some("token"));
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
             api_key: Some(ApiKey { prefix: Some(\"Token\"), key: <redacted> }) }"
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
             api_key: None }"
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
