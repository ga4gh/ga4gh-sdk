//! Cached service discovery and advisory submission diagnostics.
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
use std::{error::Error, ops::Deref, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub field: String,
    pub support: Support,
    pub detail: String,
    pub blocks_strict: bool,
}
#[derive(Clone, Debug)]
pub struct CapabilitySnapshot {
    pub raw: Value,
    pub fetched_at: Instant,
    pub max_age: Duration,
}
impl CapabilitySnapshot {
    pub fn from_raw(raw: Value, max_age: Duration) -> Self {
        Self {
            raw,
            fetched_at: Instant::now(),
            max_age,
        }
    }
    pub fn is_fresh(&self) -> bool {
        self.fetched_at.elapsed() <= self.max_age
    }
    pub fn typed_as<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.raw.clone())
    }
    pub fn tes_info(&self) -> Result<TesServiceInfo, serde_json::Error> {
        self.typed_as()
    }
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
#[derive(Debug)]
pub struct Discovered<T> {
    pub client: T,
    pub capabilities: CapabilitySnapshot,
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
) -> Result<(Service, CapabilitySnapshot), Box<dyn Error>> {
    let raw: Value = serde_json::from_str(&transport.get("service-info", None).await?)?;
    let service: Service = if artifact == "wes" {
        super::wes::service_from_info(raw.clone())?
    } else {
        serde_json::from_value(raw.clone())?
    };
    if service.r#type.group != "org.ga4gh"
        || !service.r#type.artifact.eq_ignore_ascii_case(artifact)
    {
        return Err(format!("expected {artifact} service-info").into());
    }
    Ok((service, CapabilitySnapshot::from_raw(raw, max_age)))
}
fn ensure_strict(diagnostics: &[Diagnostic], strict: bool) -> Result<(), Box<dyn Error>> {
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
    pub async fn discover(
        config: &Configuration,
        max_age: Duration,
    ) -> Result<Discovered<Self>, Box<dyn Error>> {
        let transport = Transport::new(config);
        let (service, capabilities) = discover(&transport, "tes", max_age).await?;
        Ok(Discovered {
            client: Self {
                config: config.clone(),
                transport,
                service: Ok(service),
            },
            capabilities,
        })
    }
}
impl WES {
    pub async fn discover(
        config: &Configuration,
        max_age: Duration,
    ) -> Result<Discovered<Self>, Box<dyn Error>> {
        let transport = Transport::new(config);
        let (service, capabilities) = discover(&transport, "wes", max_age).await?;
        Ok(Discovered {
            client: Self {
                config: config.clone(),
                transport,
                service: Ok(service),
            },
            capabilities,
        })
    }
}
impl Discovered<TES> {
    pub async fn refresh(&mut self) -> Result<(), Box<dyn Error>> {
        let (service, snapshot) =
            discover(&self.client.transport, "tes", self.capabilities.max_age).await?;
        self.client.service = Ok(service);
        self.capabilities = snapshot;
        Ok(())
    }
    pub async fn create_validated(
        &self,
        task: TesTask,
        strict: bool,
    ) -> Result<Task, Box<dyn Error>> {
        ensure_strict(&self.capabilities.validate_tes(&task), strict)?;
        self.client.create(task).await
    }
}
impl Discovered<WES> {
    pub async fn refresh(&mut self) -> Result<(), Box<dyn Error>> {
        let (service, snapshot) =
            discover(&self.client.transport, "wes", self.capabilities.max_age).await?;
        self.client.service = Ok(service);
        self.capabilities = snapshot;
        Ok(())
    }
    pub async fn run_validated(
        &self,
        request: RunRequest,
        strict: bool,
    ) -> Result<Run, Box<dyn Error>> {
        ensure_strict(&self.capabilities.validate_wes(&request), strict)?;
        self.client.run_workflow(request).await
    }
}
