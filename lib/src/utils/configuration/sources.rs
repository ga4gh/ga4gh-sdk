//! Compatibility adapters and typed extraction for Figment sources.

use super::{profile_sections, BasicAuth, Configuration};
use crate::error::BoxError;
use figment::providers::Serialized;
use figment::{
    error::Kind, value::Dict, value::Map as FigmentMap, Figment, Metadata, Profile, Provider,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
pub(super) struct ServiceSettings {
    pub base_path: String,
    pub basic_auth: Option<BasicAuth>,
    pub oauth_access_token: Option<String>,
    pub oauth_access_token_env: Option<String>,
    pub bearer_access_token: Option<String>,
    pub bearer_access_token_env: Option<String>,
    pub api_key: Option<Value>,
    pub timeouts: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

struct ServiceFile {
    path: PathBuf,
    fields: Map<String, Value>,
}

impl Provider for ServiceFile {
    fn metadata(&self) -> Metadata {
        Metadata::named("JSON configuration file").source(self.path.as_path())
    }

    fn data(&self) -> Result<FigmentMap<Profile, Dict>, figment::Error> {
        Serialized::defaults(&self.fields).data()
    }
}

pub(super) fn file_source(
    path: &Path,
    profile: Option<&str>,
    section: &str,
) -> Result<Figment, BoxError> {
    let contents = std::fs::read_to_string(path).map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!("Cannot read configuration file {}: {error}", path.display()),
        )
    })?;
    let document: Value = serde_json::from_str(&contents).map_err(|error| {
        // A parser message may quote a credential or extension setting.
        format!(
            "Invalid JSON in configuration file {} at line {}, column {}",
            path.display(),
            error.line(),
            error.column()
        )
    })?;
    let root = document
        .as_object()
        .ok_or("Configuration file must be a JSON object")?;
    let (profile, sections) = profile_sections(root, profile)?;
    let fields = sections
        .get(section)
        .and_then(Value::as_object)
        .ok_or_else(|| match &profile {
            None => format!("Configuration file must contain a '{section}' object"),
            Some(name) => format!(
                "Profile '{}' must contain a '{section}' object",
                super::display_profile_name(name)
            ),
        })?;
    Ok(Figment::from(ServiceFile {
        path: path.to_owned(),
        fields: fields.clone(),
    }))
}

/// Omits values and custom provider/deserializer messages from diagnostics.
/// Retains field names, expected types and source names or file paths.
pub(super) fn extraction_error(error: figment::Error) -> BoxError {
    let mut path = error.path.clone();
    if let Kind::MissingField(field) = &error.kind {
        path.push(field.to_string());
    }
    let field = path.join(".");
    if field == "base_path" {
        return "Configuration section must contain a 'base_path' string".into();
    }
    let field = super::display_profile_name(&field);
    let message = match &error.kind {
        Kind::InvalidType(_, expected) if expected == "a string" => {
            format!("{field} must be a string")
        }
        Kind::InvalidType(_, expected) if expected.starts_with("struct ") => {
            format!("{field} must be an object")
        }
        Kind::MissingField(_) => format!("Missing configuration field '{field}'"),
        _ => format!("Invalid configuration source or field '{field}'"),
    };
    let source = error.metadata.as_ref().map(|metadata| {
        metadata
            .source
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| metadata.name.to_string())
    });
    match source {
        Some(source) => format!("{message} in {}", super::display_profile_name(&source)).into(),
        None => message.into(),
    }
}

impl Configuration {
    /// Returns the selected service section as a Figment source for layering.
    /// Reads the same JSON formats as [`Self::from_path_with_profile`], with
    /// exact, case-sensitive profile selection and no missing-profile fallback.
    /// The returned Figment contains service fields directly, without an
    /// enclosing service or profile key. Sources are read eagerly.
    ///
    /// Later Figment `merge` sources override earlier values; `join` supplies
    /// missing values. Dictionaries merge recursively, arrays replace, and
    /// explicit null replaces a previous value. An absent field keeps it.
    /// No environment variables are read unless a caller adds an environment
    /// provider or a setting explicitly names a token variable.
    ///
    /// # Errors
    ///
    /// Returns configuration errors for unreadable or malformed files and
    /// invalid or missing profiles or sections. Validation of service fields
    /// occurs in [`Self::from_provider`], after merging all sources.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use figment::providers::Env;
    /// use ga4gh_sdk::utils::configuration::Configuration;
    /// # fn main() -> ga4gh_sdk::Result<()> {
    /// let sources = Configuration::sources("config.json", Some("remote"), "TES")?
    ///     .merge(Env::prefixed("MY_APP_TES_").split("__"));
    /// let config = Configuration::from_provider(sources)?;
    /// # let _ = config;
    /// # Ok(())
    /// # }
    /// ```
    pub fn sources(
        path: impl AsRef<Path>,
        profile: Option<&str>,
        section: &str,
    ) -> crate::Result<Figment> {
        file_source(path.as_ref(), profile, section).map_err(crate::Error::Configuration)
    }

    /// Resolves and validates one service's fields from a Figment provider.
    ///
    /// Callers control source order and environment prefixes. The provider may
    /// combine file settings from [`Self::sources`], serialized defaults,
    /// environment values, and explicit overrides. Credential-source conflicts
    /// remain errors after merging. Optional null fields clear previous values;
    /// `base_path` must always be a string. Null timeouts restore their defaults.
    /// Extra settings, including nulls, are preserved in [`Self::extra`]. A
    /// selected non-default Figment profile must exist in the provided data.
    ///
    /// # Errors
    ///
    /// Returns configuration errors for invalid fields, URLs, timeouts or
    /// authentication. Default diagnostics omit provider values and messages
    /// from custom deserializers because they can contain credentials.
    pub fn from_provider(provider: impl Provider) -> crate::Result<Self> {
        Self::resolve_provider(provider).map_err(crate::Error::Configuration)
    }

    fn resolve_provider(provider: impl Provider) -> Result<Self, BoxError> {
        let sources = Figment::from(provider);
        if sources.profile() != Profile::Default
            && !sources
                .profiles()
                .any(|profile| profile == sources.profile())
        {
            return Err(format!(
                "Profile '{}' not found in configuration sources",
                super::display_profile_name(&sources.profile().to_string())
            )
            .into());
        }
        let fields: ServiceSettings = sources.extract().map_err(extraction_error)?;
        let base_path = url::Url::parse(&fields.base_path)
            .map_err(|error| format!("Invalid base_path in service configuration: {error}"))?;
        let mut config = Self::new(base_path);
        config.basic_auth = fields.basic_auth;
        config.oauth_access_token = super::token_field(
            fields.oauth_access_token,
            fields.oauth_access_token_env,
            "oauth_access_token",
            "oauth_access_token_env",
        )?;
        config.bearer_access_token = super::token_field(
            fields.bearer_access_token,
            fields.bearer_access_token_env,
            "bearer_access_token",
            "bearer_access_token_env",
        )?;
        if fields.api_key.is_some() {
            return Err(super::AuthError::Unsupported { source: "api_key" }.into());
        }
        if let Some(value) = fields.timeouts {
            config.timeouts = super::timeouts_from_value(&value)?;
        }
        config.extra = fields.extra;
        config.configured_auth()?;
        Ok(config)
    }
}
