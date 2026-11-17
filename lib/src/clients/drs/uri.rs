//! Turns hostname-based `drs://` URIs into access plans; never downloads
//! object bytes.
//!
//! [`DrsUri`] parses hostname-based `drs://<host>/<id>` URIs.
//! [`DrsResolver`] reads the object at its host's DRS service, picks an
//! access method by [`AccessPreferences`], and gives an [`AccessPlan`]: the
//! URL and headers to fetch the bytes with. [`DrsResolver::with_service`]
//! sets the transport, such as one with credentials, for a host, and
//! [`DrsResolver::access_plan_once`] gives pending answers back instead of
//! waiting.
//!
//! ```no_run
//! use ga4gh_sdk::clients::drs::models::access_method::Type;
//! use ga4gh_sdk::clients::drs::uri::{AccessPreferences, DrsResolver};
//! use ga4gh_sdk::utils::operation::Operation;
//! use std::time::Duration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let resolver = DrsResolver::default();
//! // HTTPS access methods first, then S3 ones
//! let preferences = AccessPreferences {
//!     protocols: vec![Type::Https, Type::S3],
//!     ..Default::default()
//! };
//! let operation = Operation::new(Duration::from_secs(60));
//! let max_attempts = 3;
//! let plan = resolver
//!     .access_plan("drs://drs.example.org/object-1", &preferences, &operation, max_attempts)
//!     .await?;
//! println!("{:?} from {}", plan.method, plan.url.origin().ascii_serialization());
//! # Ok(())
//! # }
//! ```
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

/// Why a DRS URI can't be parsed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UriError {
    /// The URI doesn't start with `drs://`.
    UnsupportedScheme,
    /// The URI has a `:` after `drs://` that isn't part of a port or an
    /// IPv6 host in brackets: a compact identifier
    /// (`drs://<prefix>:<accession>`), which needs a resolver service. A
    /// `:` without a `/` after it always counts as one.
    UnsupportedCompactIdentifier,
    /// The URI isn't `drs://<host>/<id>` with a percent-encoded ID of one
    /// path segment, or its host has a port, which hostname-based DRS URIs
    /// don't allow. Digits after the last `:` before the first `/` count as
    /// a port, so a compact identifier such as `drs://ark:12148/x` also
    /// gives this error.
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
// What the part of a DRS URI before its first '/' looks like, judged by its
// text before URL parsing.
enum Authority {
    // An IPv6 address in brackets, without a port.
    Ipv6,
    // A host with a port: digits only after the last ':', or after the
    // brackets of an IPv6 address. `drs://<prefix>:<digits>/...` is
    // ambiguous with a compact identifier and counts as a port.
    WithPort,
    // An opening bracket without a closing one, or an IPv6 address
    // without the '/' that must follow a host.
    Invalid,
    // Anything else, including compact identifiers and the whole URI if it
    // has no '/', since a hostname-based URI needs one after its host.
    Other,
}
impl Authority {
    // Classifies the part of `rest`, the URI after `drs://`, before its
    // first '/'.
    fn classify(rest: &str) -> Self {
        let Some((authority, _)) = rest.split_once('/') else {
            return if rest.starts_with('[') {
                Self::Invalid
            } else {
                Self::Other
            };
        };
        let port = match authority.strip_prefix('[') {
            Some(inner) => match inner.split_once(']') {
                Some((_, "")) => return Self::Ipv6,
                Some((_, after)) => after.strip_prefix(':'),
                None => return Self::Invalid,
            },
            None => authority
                .rsplit_once(':')
                .map(|(_, port)| port)
                .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
        };
        if port.is_some() {
            Self::WithPort
        } else if authority.starts_with('[') {
            // Text after the brackets that isn't a port; URL parsing rejects it
            Self::Ipv6
        } else {
            Self::Other
        }
    }
}
/// A hostname-based DRS URI, `drs://<host>/<id>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DrsUri {
    /// The host as the URL parser normalizes it: lowercase, with
    /// internationalized names in punycode, IPv4 addresses in dotted
    /// decimal and IPv6 addresses compressed, in brackets.
    pub authority: String,
    /// The object ID, percent-decoded.
    pub object_id: String,
}
impl DrsUri {
    /// Parses `drs://<host>/<id>`; the scheme is matched ignoring case. The
    /// host can be a name, an IPv4 address or an IPv6 address in brackets.
    ///
    /// # Errors
    ///
    /// Fails with the [`UriError`] that says why the URI isn't supported.
    pub fn parse(uri: &str) -> Result<Self, UriError> {
        if !uri
            .get(..6)
            .is_some_and(|s| s.eq_ignore_ascii_case("drs://"))
        {
            return Err(UriError::UnsupportedScheme);
        }
        let rest = &uri[6..];
        // The pinned hostname form has no literal ':' except in an IPv6 host.
        // Compact accessions may contain slashes and colons, so recognize them
        // before interpreting URL authority, but tell a port or an IPv6 host
        // apart from them.
        match Authority::classify(rest) {
            Authority::Ipv6 => {}
            Authority::WithPort | Authority::Invalid => return Err(UriError::Malformed),
            Authority::Other if rest.contains(':') => {
                return Err(UriError::UnsupportedCompactIdentifier)
            }
            Authority::Other => {}
        }
        if rest.contains(['?', '#', '\\', '@']) || rest.chars().any(char::is_whitespace) {
            return Err(UriError::Malformed);
        }
        let (authority, encoded) = rest.split_once('/').ok_or(UriError::Malformed)?;
        if encoded.is_empty() || encoded.contains('/') || authority.is_empty() {
            return Err(UriError::Malformed);
        }
        let host = Url::parse(&format!("https://{authority}/")).map_err(|_| UriError::Malformed)?;
        if host.host_str().is_none() {
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
    /// The DRS API root that the DRS specification derives from the host:
    /// `https://<host>/ga4gh/drs/v1`.
    pub fn default_api_root(&self) -> Url {
        Url::parse(&format!("https://{}/ga4gh/drs/v1", self.authority))
            .expect("validated DRS authority")
    }
}

/// Which access method of an object to use.
///
/// Without `method_index`, the methods of the listed protocols and regions
/// are candidates (all of them for empty lists), ordered by the position of
/// their protocol, then their region, in the lists, then by the server's
/// order. The first available one is used.
#[derive(Clone, Debug, Default)]
pub struct AccessPreferences {
    /// The accepted protocols, most preferred first; empty accepts any.
    pub protocols: Vec<Type>,
    /// The accepted regions, most preferred first; empty accepts any,
    /// including methods without a region.
    pub regions: Vec<String>,
    /// The index of the method in the object's `access_methods` to use
    /// instead of choosing one.
    pub method_index: Option<usize>,
}
/// Why no access plan could be made from an object's access methods.
#[derive(Debug)]
pub enum AccessChoiceError {
    /// No method matches the preferences, or the chosen one has neither an
    /// access URL nor an access ID.
    UnsupportedMethods,
    /// The matching methods, or the one of `method_index`, are marked
    /// unavailable.
    Unavailable,
    /// `method_index` is out of range.
    InvalidOverride,
    /// The access URL isn't an absolute URL.
    InvalidAccessUrl,
    /// An access header isn't `<name>: <value>` with a valid name and value.
    InvalidAccessHeaders,
}
impl fmt::Display for AccessChoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DRS access selection: {self:?}")
    }
}
impl Error for AccessChoiceError {}
/// How to fetch the bytes of an object. The URL and headers may hold
/// credentials; `Debug` shows only the URL's origin.
pub struct AccessPlan {
    /// The DRS object ID.
    pub object_id: String,
    /// The protocol of the chosen access method.
    pub method: Type,
    /// The region of the chosen access method, if it has one.
    pub region: Option<String>,
    /// The URL to fetch the bytes from.
    pub url: Url,
    /// The headers to send with the request.
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
/// The result of [`DrsResolver::access_plan_once`].
#[derive(Debug)]
pub enum AccessResolution {
    /// The access plan.
    Ready(AccessPlan),
    /// The server is still preparing the object or access URL.
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
    /// Uses `transport` for the URIs of `authority`, a host without port,
    /// instead of an anonymous transport at the default API root.
    ///
    /// # Errors
    ///
    /// Fails with [`UriError::Malformed`] if `authority` has a port or isn't
    /// a host, and with [`UriError::UnsupportedCompactIdentifier`] if it
    /// has another `:` outside an IPv6 host.
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
    /// Plans access to the object of `uri` with at most one request per
    /// step, without waiting for pending answers.
    ///
    /// # Errors
    ///
    /// Fails if `uri` can't be parsed, a request fails, the operation ends,
    /// the server sends another object, or no access method fits (see
    /// [`AccessChoiceError`]).
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
    /// Like [`Self::access_plan_once`], but waits for pending answers, with up
    /// to `max_attempts` attempts per request, as [`DRS::resolve_object`]
    /// does.
    ///
    /// Opt-in waiting uses the same deferred-resolution state machine and budget.
    ///
    /// # Errors
    ///
    /// Fails if `uri` can't be parsed, the server sends another object, or no
    /// access method fits (see [`AccessChoiceError`]). Failures while reading
    /// the object or access URL, an operation that ends and a
    /// `max_attempts` outside 1 to 10 000 included, come as a
    /// [`ResolveError`](super::resolution::ResolveError).
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
