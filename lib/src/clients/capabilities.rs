//! Cached service discovery and advisory submission diagnostics.
//!
//! [`TES::discover`] and [`WES::discover`] build a client from one
//! service-info request and keep the response as a [`CapabilitySnapshot`].
//! The snapshot checks a task or run request against what the service
//! advertises and gives a [`Diagnostic`] per checked field. The checks are
//! advisory: services may advertise too little, and a stale snapshot only
//! gives [`Support::Unknown`].
//!
//! ```no_run
//! use ga4gh_sdk::clients::wes::{models::RunRequest, WES};
//! use ga4gh_sdk::utils::configuration::Configuration;
//! use std::time::Duration;
//!
//! # async fn example() -> ga4gh_sdk::Result<()> {
//! let config = Configuration::new(url::Url::parse("https://example.org/ga4gh/wes/v1")?);
//! let wes = WES::discover(&config, Duration::from_secs(300)).await?;
//! let request = RunRequest {
//!     workflow_type: "CWL".into(),
//!     workflow_type_version: "v1.2".into(),
//!     workflow_url: "https://example.org/workflow.cwl".into(),
//!     ..Default::default()
//! };
//! for diagnostic in wes.capabilities().validate_wes(&request) {
//!     println!("{}: {:?}", diagnostic.field, diagnostic.support);
//! }
//! // Strict: refuse the request if a diagnostic blocks it
//! let run = wes.run_validated(request, true).await?;
//! println!("{}", run.id);
//! # Ok(())
//! # }
//! ```
use super::{
    serviceinfo::models::Service,
    tes::{
        models::{TesServiceInfo, TesTask},
        Task, TES,
    },
    wes::{
        models::{RunRequest, ServiceInfo},
        Run, WES,
    },
};
use crate::utils::{configuration::Configuration, transport::Transport};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use std::{ops::Deref, time::Duration};
use tokio::time::Instant;

/// Whether the service advertises a requested value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Support {
    /// The service advertises the value.
    Supported,
    /// The service advertises a list of values without it.
    Unsupported,
    /// The service advertises nothing about it, or the snapshot is stale.
    Unknown,
}
/// The result of checking one field of a request against a snapshot.
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    /// The request field, such as `workflow_type` or
    /// `resources.backend_parameters.<key>`.
    pub field: String,
    /// Whether the service advertises the field's value.
    pub support: Support,
    /// What was checked, or that the snapshot is stale.
    pub detail: String,
    /// Whether a strict submission, such as
    /// [`Discovered::<WES>::run_validated`] with `strict`, refuses the
    /// request because of this field.
    pub blocks_strict: bool,
}
/// A service-info response and when it was fetched.
#[derive(Clone, Debug)]
pub struct CapabilitySnapshot {
    /// The service-info response as JSON.
    pub raw: Value,
    /// When the response was fetched.
    pub fetched_at: Instant,
    /// How long the snapshot stays fresh.
    pub max_age: Duration,
}
impl CapabilitySnapshot {
    /// Wraps a service-info response fetched now, fresh for `max_age`.
    pub fn from_raw(raw: Value, max_age: Duration) -> Self {
        Self {
            raw,
            fetched_at: Instant::now(),
            max_age,
        }
    }
    /// Whether the snapshot is at most `max_age` old.
    pub fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() <= self.max_age
    }
    /// Parses the response as `T`, such as a service info model.
    ///
    /// # Errors
    ///
    /// Fails if the response doesn't match `T`.
    pub fn typed_as<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.raw.clone())
    }
    /// Parses the response as a TES service info.
    ///
    /// # Errors
    ///
    /// Fails if the response isn't a TES service info.
    pub fn tes_info(&self) -> Result<TesServiceInfo, serde_json::Error> {
        self.typed_as()
    }
    /// Parses the response as a WES service info.
    ///
    /// # Errors
    ///
    /// Fails if the response isn't a WES service info.
    pub fn wes_info(&self) -> Result<ServiceInfo, serde_json::Error> {
        self.typed_as()
    }
    fn diagnostic(
        &self,
        field: &str,
        support: Support,
        detail: impl Into<String>,
        blocks: bool,
    ) -> Diagnostic {
        Diagnostic {
            field: field.into(),
            support: if self.is_fresh() {
                support
            } else {
                Support::Unknown
            },
            detail: if self.is_fresh() {
                detail.into()
            } else {
                "cached service metadata is stale".into()
            },
            blocks_strict: blocks && self.is_fresh(),
        }
    }
    /// Checks a run request against the workflow types and engines that the
    /// service advertises.
    ///
    /// The request's `workflow_type` and `workflow_type_version` are checked
    /// against the advertised `workflow_type_versions`. Its `workflow_engine`,
    /// when set, is checked against `workflow_engine_versions`, and with it
    /// its `workflow_engine_version`, when set. Unsupported values block
    /// strict submissions.
    ///
    /// Missing metadata remains unknown. An explicit nonempty language map is
    /// authoritative; version/engine constraints are compared only when supplied.
    pub fn validate_wes(&self, request: &RunRequest) -> Vec<Diagnostic> {
        let mut result = Vec::new();
        let request = serde_json::to_value(request).unwrap_or_default();
        let language = request.get("workflow_type").and_then(Value::as_str);
        let map = self
            .raw
            .get("workflow_type_versions")
            .and_then(Value::as_object);
        let language_info = language.and_then(|language| map.and_then(|m| m.get(language)));
        let support = match (language, map) {
            (Some(language), Some(map)) if !map.is_empty() => {
                if map.contains_key(language) {
                    Support::Supported
                } else {
                    Support::Unsupported
                }
            }
            _ => Support::Unknown,
        };
        result.push(self.diagnostic(
            "workflow_type",
            support,
            format!(
                "requested workflow language {}",
                language.unwrap_or("<unspecified>")
            ),
            support == Support::Unsupported,
        ));
        if let Some(version) = request.get("workflow_type_version").and_then(Value::as_str) {
            let versions = language_info
                .and_then(|v| v.get("workflow_type_version"))
                .and_then(Value::as_array);
            let support = match versions {
                Some(v) if !v.is_empty() => {
                    if v.iter().any(|v| v.as_str() == Some(version)) {
                        Support::Supported
                    } else {
                        Support::Unsupported
                    }
                }
                _ => Support::Unknown,
            };
            result.push(self.diagnostic(
                "workflow_type_version",
                support,
                format!("requested version {version}"),
                support == Support::Unsupported,
            ));
        }
        if let Some(engine) = request.get("workflow_engine").and_then(Value::as_str) {
            let map = self
                .raw
                .get("workflow_engine_versions")
                .and_then(Value::as_object);
            let support = match map {
                Some(m) if !m.is_empty() => {
                    if m.contains_key(engine) {
                        Support::Supported
                    } else {
                        Support::Unsupported
                    }
                }
                _ => Support::Unknown,
            };
            result.push(self.diagnostic(
                "workflow_engine",
                support,
                format!("requested engine {engine}"),
                support == Support::Unsupported,
            ));
            if let Some(version) = request
                .get("workflow_engine_version")
                .and_then(Value::as_str)
            {
                let advertised = map.and_then(|m| m.get(engine));
                let versions = advertised
                    .and_then(|v| v.get("workflow_engine_version"))
                    .and_then(Value::as_array);
                // WES 1.0 advertised a single string; WES 1.1 uses a version list.
                let support = match (versions, advertised.and_then(Value::as_str)) {
                    (Some(v), _) if !v.is_empty() => {
                        if v.iter().any(|v| v.as_str() == Some(version)) {
                            Support::Supported
                        } else {
                            Support::Unsupported
                        }
                    }
                    (_, Some(v)) => {
                        if v == version {
                            Support::Supported
                        } else {
                            Support::Unsupported
                        }
                    }
                    _ => Support::Unknown,
                };
                result.push(self.diagnostic(
                    "workflow_engine_version",
                    support,
                    format!("requested engine version {version}"),
                    support == Support::Unsupported,
                ));
            }
        }
        result
    }
    /// Checks each key of the task's `resources.backend_parameters` against
    /// the advertised `tesResources_backend_parameters`, ignoring case. An
    /// unsupported key blocks strict submissions only if the task sets
    /// `backend_parameters_strict`. Other fields are not checked.
    pub fn validate_tes(&self, task: &TesTask) -> Vec<Diagnostic> {
        let mut result = Vec::new();
        if let Some(resources) = &task.resources {
            if let Some(parameters) = &resources.backend_parameters {
                let known = self
                    .raw
                    .get("tesResources_backend_parameters")
                    .and_then(Value::as_array);
                for key in parameters.keys() {
                    let support = match known {
                        Some(keys) => {
                            if keys
                                .iter()
                                .filter_map(Value::as_str)
                                .any(|v| v.eq_ignore_ascii_case(key))
                            {
                                Support::Supported
                            } else {
                                Support::Unsupported
                            }
                        }
                        None => Support::Unknown,
                    };
                    result.push(self.diagnostic(&format!("resources.backend_parameters.{key}"),support,
                        "backend parameter names are case insensitive; non-strict unsupported keys may be ignored",
                        support==Support::Unsupported&&resources.backend_parameters_strict==Some(true)));
                }
            }
        }
        // TES storage is explicitly nonexhaustive. Do not reject input locations.
        result
    }
}

/// Opt-in discovery wrapper: one request populates the client and raw/typed
/// capabilities. Accessors never fetch; refresh is explicit and atomic on success.
///
/// Read the parts with [`client`](Self::client) (or `Deref`) and
/// [`capabilities`](Self::capabilities), or split the wrapper with
/// [`into_parts`](Self::into_parts). The fields are private: an assignment
/// to one of them would separate the client from the snapshot of its
/// service. The
/// [client state ownership record](https://github.com/ga4gh/ga4gh-sdk/blob/main/docs/state_ownership.md#migration)
/// lists the replacements of the former public fields.
///
/// Assigning a field fails to compile:
///
/// ```compile_fail,E0616
/// use ga4gh_sdk::clients::{capabilities::Discovered, tes::TES};
/// fn swap(discovered: &mut Discovered<TES>, client: TES) {
///     discovered.client = client;
/// }
/// ```
///
/// ```compile_fail,E0616
/// use ga4gh_sdk::clients::{
///     capabilities::{CapabilitySnapshot, Discovered},
///     tes::TES,
/// };
/// fn swap_snapshot(discovered: &mut Discovered<TES>, snapshot: CapabilitySnapshot) {
///     discovered.capabilities = snapshot;
/// }
/// ```
#[derive(Debug)]
pub struct Discovered<T> {
    /// The client, also reached through `Deref` and [`Self::client`].
    client: T,
    /// The service-info response that the client was built from; returned
    /// by [`Self::capabilities`].
    capabilities: CapabilitySnapshot,
}
impl<T> Discovered<T> {
    /// The client, also reached through `Deref`.
    pub fn client(&self) -> &T {
        &self.client
    }

    /// The capability snapshot from the service-info response that the
    /// client's discovered service came from.
    pub fn capabilities(&self) -> &CapabilitySnapshot {
        &self.capabilities
    }

    /// Splits the wrapper into the client and its capability snapshot, for
    /// example to keep only the client.
    pub fn into_parts(self) -> (T, CapabilitySnapshot) {
        (self.client, self.capabilities)
    }

    /// Builds the wrapper from a client and the snapshot of the same
    /// service-info response.
    fn assemble(client: T, capabilities: CapabilitySnapshot) -> Self {
        Self {
            client,
            capabilities,
        }
    }
}
impl<T> Deref for Discovered<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.client
    }
}
async fn discover(
    transport: &Transport,
    artifact: &str,
    max_age: Duration,
) -> crate::Result<(Service, CapabilitySnapshot)> {
    let raw: Value =
        serde_json::from_str(&super::serviceinfo::fetch_raw(transport, "service-info").await?)?;
    let service: Service = if artifact == "wes" {
        super::wes::service_from_info(raw.clone())?
    } else {
        serde_json::from_value(raw.clone())?
    };
    if service.r#type.group != "org.ga4gh"
        || !service.r#type.artifact.eq_ignore_ascii_case(artifact)
    {
        return Err(crate::Error::UnexpectedService(format!(
            "expected {artifact} service-info"
        )));
    }
    Ok((service, CapabilitySnapshot::from_raw(raw, max_age)))
}
fn ensure_strict(diagnostics: &[Diagnostic], strict: bool) -> crate::Result<()> {
    if strict {
        if let Some(d) = diagnostics
            .iter()
            .find(|d| d.blocks_strict && d.support == Support::Unsupported)
        {
            return Err(format!("submission preflight: {}: {}", d.field, d.detail).into());
        }
    }
    Ok(())
}
impl TES {
    /// Creates a client and a capability snapshot from one service-info
    /// request, within the `discovery` limit of `config.timeouts`.
    ///
    /// # Errors
    ///
    /// Fails if the request fails or times out, or the response isn't a TES
    /// service info.
    pub async fn discover(
        config: &Configuration,
        max_age: Duration,
    ) -> crate::Result<Discovered<Self>> {
        Self::discover_with_transport(Transport::new(config), max_age).await
    }

    /// Like [`Self::discover`], with a ready `transport`, for example one from
    /// [`Transport::with_client`]. The client's configuration is the transport's.
    ///
    /// # Errors
    ///
    /// Fails if the request fails or times out, or the response isn't a TES
    /// service info.
    pub async fn discover_with_transport(
        transport: Transport,
        max_age: Duration,
    ) -> crate::Result<Discovered<Self>> {
        let (service, capabilities) = discover(&transport, "tes", max_age).await?;
        Ok(Discovered::assemble(
            Self::without_discovery(transport, service),
            capabilities,
        ))
    }
}
impl WES {
    /// Creates a client and a capability snapshot from one service-info
    /// request, within the `discovery` limit of `config.timeouts`.
    ///
    /// # Errors
    ///
    /// Fails if the request fails or times out, or the response isn't a WES
    /// service info.
    pub async fn discover(
        config: &Configuration,
        max_age: Duration,
    ) -> crate::Result<Discovered<Self>> {
        Self::discover_with_transport(Transport::new(config), max_age).await
    }

    /// Like [`Self::discover`], with a ready `transport`, for example one from
    /// [`Transport::with_client`]. The client's configuration is the transport's.
    ///
    /// # Errors
    ///
    /// Fails if the request fails or times out, or the response isn't a WES
    /// service info.
    pub async fn discover_with_transport(
        transport: Transport,
        max_age: Duration,
    ) -> crate::Result<Discovered<Self>> {
        let (service, capabilities) = discover(&transport, "wes", max_age).await?;
        Ok(Discovered::assemble(
            Self::without_discovery(transport, service),
            capabilities,
        ))
    }
}
impl Discovered<TES> {
    /// Fetches the service info again and replaces the client's service and
    /// the snapshot; on failure, both stay as they were.
    ///
    /// # Errors
    ///
    /// Fails if the request fails or the response isn't a TES service info.
    pub async fn refresh(&mut self) -> crate::Result<()> {
        let transport = self.client.transport();
        let (service, snapshot) = discover(transport, "tes", self.capabilities.max_age).await?;
        self.client = TES::without_discovery(transport.clone(), service);
        self.capabilities = snapshot;
        Ok(())
    }
    /// Checks `task` with [`CapabilitySnapshot::validate_tes`], then creates
    /// it like [`TES::create`].
    ///
    /// # Errors
    ///
    /// With `strict`, fails before any request if a diagnostic blocks strict
    /// submissions; otherwise fails as [`TES::create`] does.
    pub async fn create_validated(&self, task: TesTask, strict: bool) -> crate::Result<Task> {
        ensure_strict(&self.capabilities().validate_tes(&task), strict)?;
        self.client().create(task).await
    }
}
impl Discovered<WES> {
    /// Fetches the service info again and replaces the client's service and
    /// the snapshot; on failure, both stay as they were.
    ///
    /// # Errors
    ///
    /// Fails if the request fails or the response isn't a WES service info.
    pub async fn refresh(&mut self) -> crate::Result<()> {
        let transport = self.client.transport();
        let (service, snapshot) = discover(transport, "wes", self.capabilities.max_age).await?;
        self.client = WES::without_discovery(transport.clone(), service);
        self.capabilities = snapshot;
        Ok(())
    }
    /// Checks `request` with [`CapabilitySnapshot::validate_wes`], then
    /// starts the run like [`WES::run_workflow`].
    ///
    /// # Errors
    ///
    /// With `strict`, fails before any request if a diagnostic blocks strict
    /// submissions; otherwise fails as [`WES::run_workflow`] does.
    pub async fn run_validated(&self, request: RunRequest, strict: bool) -> crate::Result<Run> {
        ensure_strict(&self.capabilities().validate_wes(&request), strict)?;
        self.client().run_workflow(request).await
    }
}
