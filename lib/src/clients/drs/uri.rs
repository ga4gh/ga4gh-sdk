//! Hostname DRS references -> explicit access plans. Never downloads object bytes.
use super::{
    models::{access_method::Type, AccessMethod, AccessUrl},
    resolution::{PendingResolution, Resolution},
    DRS,
};
use crate::{
    clients::serviceinfo::models::Service,
    utils::{operation::Operation, transport::Transport},
};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::{collections::HashMap, error::Error, fmt};
use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UriError {
    UnsupportedScheme,
    UnsupportedCompactIdentifier,
    Malformed,
}
impl fmt::Display for UriError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedScheme => "unsupported DRS URI scheme",
            Self::UnsupportedCompactIdentifier => {
                "compact DRS identifiers are not supported by this hostname resolver"
            }
            Self::Malformed => "malformed hostname-based DRS URI",
        })
    }
}
impl Error for UriError {}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrsUri {
    pub authority: String,
    pub object_id: String,
}
impl DrsUri {
    pub fn parse(uri: &str) -> Result<Self, UriError> {
        if !uri
            .get(..6)
            .is_some_and(|s| s.eq_ignore_ascii_case("drs://"))
        {
            return Err(UriError::UnsupportedScheme);
        }
        let rest = &uri[6..];
        // The pinned hostname form has no literal ':'. Compact accessions may contain
        // slashes and colons, so recognize them before interpreting URL authority.
        if rest.contains(':') {
            return Err(UriError::UnsupportedCompactIdentifier);
        }
        if rest.contains(['?', '#', '\\', '@']) || rest.chars().any(char::is_whitespace) {
            return Err(UriError::Malformed);
        }
        let (authority, encoded) = rest.split_once('/').ok_or(UriError::Malformed)?;
        if encoded.is_empty() || encoded.contains('/') || authority.is_empty() {
            return Err(UriError::Malformed);
        }
        let host = Url::parse(&format!("https://{authority}/")).map_err(|_| UriError::Malformed)?;
        if host.host_str().is_none() || host.port().is_some() {
            return Err(UriError::Malformed);
        }
        let mut bytes = Vec::new();
        let encoded = encoded.as_bytes();
        let mut index = 0;
        while index < encoded.len() {
            let byte = encoded[index];
            if byte == b'%' {
                let hex = encoded
                    .get(index + 1..index + 3)
                    .ok_or(UriError::Malformed)?;
                let text = std::str::from_utf8(hex).map_err(|_| UriError::Malformed)?;
                bytes.push(u8::from_str_radix(text, 16).map_err(|_| UriError::Malformed)?);
                index += 3;
            } else if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                bytes.push(byte);
                index += 1;
            } else {
                return Err(UriError::Malformed);
            }
        }
        let object_id = String::from_utf8(bytes).map_err(|_| UriError::Malformed)?;
        if object_id.is_empty() || matches!(object_id.as_str(), "." | "..") {
            return Err(UriError::Malformed);
        }
        Ok(Self {
            authority: host.host_str().unwrap().to_ascii_lowercase(),
            object_id,
        })
    }
    pub fn default_api_root(&self) -> Url {
        Url::parse(&format!("https://{}/ga4gh/drs/v1", self.authority))
            .expect("validated DRS authority")
    }
}

#[derive(Clone, Debug, Default)]
pub struct AccessPreferences {
    pub protocols: Vec<Type>,
    pub regions: Vec<String>,
    pub method_index: Option<usize>,
}
#[derive(Debug)]
pub enum AccessChoiceError {
    UnsupportedMethods,
    Unavailable,
    InvalidOverride,
    InvalidAccessUrl,
    InvalidAccessHeaders,
}
impl fmt::Display for AccessChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DRS access selection: {self:?}")
    }
}
impl Error for AccessChoiceError {}
pub struct AccessPlan {
    pub object_id: String,
    pub method: Type,
    pub region: Option<String>,
    pub url: Url,
    pub headers: HeaderMap,
}
impl fmt::Debug for AccessPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccessPlan")
            .field("object_id", &self.object_id)
            .field("method", &self.method)
            .field("region", &self.region)
            .field("url_origin", &self.url.origin().ascii_serialization())
            .field("headers", &"<redacted>")
            .finish()
    }
}
#[derive(Debug)]
pub enum AccessResolution {
    Ready(AccessPlan),
    Pending(PendingResolution),
}

fn select<'a>(
    methods: &'a [AccessMethod],
    preferences: &AccessPreferences,
) -> Result<&'a AccessMethod, AccessChoiceError> {
    if let Some(index) = preferences.method_index {
        let method = methods
            .get(index)
            .ok_or(AccessChoiceError::InvalidOverride)?;
        if method.available == Some(false) {
            return Err(AccessChoiceError::Unavailable);
        }
        return Ok(method);
    }
    let mut candidates = methods
        .iter()
        .filter(|m| {
            (preferences.protocols.is_empty() || preferences.protocols.contains(&m.r#type))
                && (preferences.regions.is_empty()
                    || m.region
                        .as_ref()
                        .is_some_and(|r| preferences.regions.contains(r)))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Err(AccessChoiceError::UnsupportedMethods);
    }
    candidates.retain(|m| m.available != Some(false));
    if candidates.is_empty() {
        return Err(AccessChoiceError::Unavailable);
    }
    candidates.sort_by_key(|m| {
        (
            preferences
                .protocols
                .iter()
                .position(|p| p == &m.r#type)
                .unwrap_or(0),
            preferences
                .regions
                .iter()
                .position(|r| Some(r) == m.region.as_ref())
                .unwrap_or(0),
        )
    });
    Ok(candidates[0])
}
fn plan(
    id: String,
    method: &AccessMethod,
    access: AccessUrl,
) -> Result<AccessPlan, AccessChoiceError> {
    let url = Url::parse(&access.url).map_err(|_| AccessChoiceError::InvalidAccessUrl)?;
    if url.cannot_be_a_base() || url.scheme().is_empty() {
        return Err(AccessChoiceError::InvalidAccessUrl);
    }
    let mut headers = HeaderMap::new();
    for header in access.headers.unwrap_or_default() {
        let (name, value) = header
            .split_once(':')
            .ok_or(AccessChoiceError::InvalidAccessHeaders)?;
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| AccessChoiceError::InvalidAccessHeaders)?;
        let value = HeaderValue::from_str(value.trim())
            .map_err(|_| AccessChoiceError::InvalidAccessHeaders)?;
        headers.append(name, value);
    }
    Ok(AccessPlan {
        object_id: id,
        method: method.r#type.clone(),
        region: method.region.clone(),
        url,
        headers,
    })
}

/// One transport per authority/credential context. Unmapped authorities use a
/// fresh anonymous HTTPS transport; configured credentials are never copied.
#[derive(Default)]
pub struct DrsResolver {
    services: HashMap<String, Transport>,
}
impl DrsResolver {
    pub fn with_service(mut self, authority: &str, transport: Transport) -> Result<Self, UriError> {
        let authority = DrsUri::parse(&format!("drs://{authority}/placeholder"))?.authority;
        self.services.insert(authority, transport);
        Ok(self)
    }
    fn client(&self, uri: &DrsUri) -> DRS {
        let transport = self
            .services
            .get(&uri.authority)
            .cloned()
            .unwrap_or_else(|| {
                Transport::new(&crate::utils::configuration::Configuration::new(
                    uri.default_api_root(),
                ))
            });
        // Routing is explicit; do not perform a second, unrelated discovery request.
        // Low-level object/access operations do not depend on the generic service view.
        DRS {
            config: transport.config.clone(),
            transport,
            service: Ok(Service::default()),
        }
    }
    pub async fn access_plan_once(
        &self,
        uri: &str,
        preferences: &AccessPreferences,
        operation: &Operation,
    ) -> Result<AccessResolution, Box<dyn Error>> {
        let uri = DrsUri::parse(uri)?;
        let client = self.client(&uri);
        let object = match operation
            .run(client.get_object_once(&uri.object_id))
            .await??
        {
            Resolution::Ready(object) => object,
            Resolution::Pending(p) => return Ok(AccessResolution::Pending(p)),
        };
        if object.id != uri.object_id {
            return Err("DRS object response identifies another object".into());
        }
        let method = select(
            object.access_methods.as_deref().unwrap_or_default(),
            preferences,
        )?;
        let access = if let Some(direct) = &method.access_url {
            AccessUrl {
                url: direct.url.clone(),
                headers: direct.headers.clone(),
                extra: direct.extra.clone(),
            }
        } else if let Some(access_id) = &method.access_id {
            match operation
                .run(client.get_access_url_once(&uri.object_id, access_id))
                .await??
            {
                Resolution::Ready(value) => value,
                Resolution::Pending(p) => return Ok(AccessResolution::Pending(p)),
            }
        } else {
            return Err(AccessChoiceError::UnsupportedMethods.into());
        };
        Ok(AccessResolution::Ready(plan(
            uri.object_id,
            method,
            access,
        )?))
    }
    /// Opt-in waiting uses the same deferred-resolution state machine and budget.
    pub async fn access_plan(
        &self,
        uri: &str,
        preferences: &AccessPreferences,
        operation: &Operation,
        max_attempts: usize,
    ) -> Result<AccessPlan, Box<dyn Error>> {
        let uri = DrsUri::parse(uri)?;
        let client = self.client(&uri);
        let object = client
            .resolve_object(&uri.object_id, operation, max_attempts)
            .await?;
        if object.id != uri.object_id {
            return Err("DRS object response identifies another object".into());
        }
        let method = select(
            object.access_methods.as_deref().unwrap_or_default(),
            preferences,
        )?;
        let access = if let Some(direct) = &method.access_url {
            AccessUrl {
                url: direct.url.clone(),
                headers: direct.headers.clone(),
                extra: direct.extra.clone(),
            }
        } else if let Some(access_id) = &method.access_id {
            client
                .resolve_access_url(&uri.object_id, access_id, operation, max_attempts)
                .await?
        } else {
            return Err(AccessChoiceError::UnsupportedMethods.into());
        };
        Ok(plan(uri.object_id, method, access)?)
    }
}
