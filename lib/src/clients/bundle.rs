//! Verified CWL descriptor staging from TRS, with explicit WES submission.
use super::{
    trs::models::{tool_file::FileType, Checksum, FileWrapper, ToolFile},
    wes::{
        attachments::{validate_filename, StreamingAttachment},
        models::WesRunRequest,
        Run, WES,
    },
};
use crate::utils::{
    operation::Operation,
    transport::{read_bounded, urlencode, Transport},
};
use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    error::Error,
    sync::Arc,
};
use url::Url;

pub struct BundleRequest {
    pub tool_id: String,
    pub version_id: String,
    pub workflow_params: Value,
}
#[derive(Clone, Debug)]
pub struct PreparationOptions {
    pub max_files: usize,
    pub max_file_bytes: usize,
    pub max_total_bytes: usize,
    pub allow_unsupported_checksums: bool,
}
impl Default for PreparationOptions {
    fn default() -> Self {
        Self {
            max_files: 128,
            max_file_bytes: 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024,
            allow_unsupported_checksums: false,
        }
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct ChecksumProof {
    pub algorithm: String,
    pub expected: String,
    pub verified: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct FileDigest {
    pub path: String,
    pub sha256: String,
    pub advertised: Vec<ChecksumProof>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ExecutionRecord {
    pub api_root: String,
    pub run_id: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct BundleManifest {
    pub registry: String,
    pub tool_id: String,
    pub version_id: String,
    pub is_production: Option<bool>,
    pub meta_version: Option<String>,
    pub files: Vec<FileDigest>,
    pub bundle_sha256: String,
    pub execution: Option<ExecutionRecord>,
}

/// Fetching wrapper URLs is opt-in. Implementations own origin/credential policy.
#[async_trait(?Send)]
pub trait DescriptorFetcher {
    async fn fetch(
        &self,
        url: &Url,
        operation: &Operation,
        limit: usize,
    ) -> Result<Vec<u8>, Box<dyn Error>>;
}
/// Explicit anonymous allowlist. Never reuses a registry's authenticated client,
/// never follows redirects, and never stores signed URL queries in provenance.
pub struct AnonymousFetcher {
    origins: BTreeSet<String>,
    client: reqwest::Client,
}
impl AnonymousFetcher {
    pub fn new(origins: impl IntoIterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let origins = origins
            .into_iter()
            .map(|s| Url::parse(&s).map(|u| u.origin().ascii_serialization()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            origins,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
}
#[async_trait(?Send)]
impl DescriptorFetcher for AnonymousFetcher {
    async fn fetch(
        &self,
        url: &Url,
        operation: &Operation,
        limit: usize,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        if !matches!(url.scheme(), "http" | "https")
            || !self.origins.contains(&url.origin().ascii_serialization())
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("descriptor URL is outside the explicit anonymous fetch policy".into());
        }
        operation
            .run(async {
                let response = self.client.get(url.clone()).send().await?;
                if !response.status().is_success() {
                    return Err(format!("descriptor fetch HTTP {}", response.status()).into());
                }
                Ok(read_bounded(response, limit).await?.body)
            })
            .await?
    }
}

pub struct PreparedBundle {
    files: BTreeMap<String, Arc<[u8]>>,
    manifest: BundleManifest,
    request: WesRunRequest,
}
pub struct SubmittedBundle {
    pub run: Run,
    pub manifest: BundleManifest,
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn safe_root(transport: &Transport) -> Result<String, Box<dyn Error>> {
    let mut url = transport.endpoint("")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}
fn check_ids(request: &BundleRequest) -> Result<(), Box<dyn Error>> {
    if [&request.tool_id, &request.version_id]
        .iter()
        .any(|s| s.is_empty() || matches!(s.as_str(), "." | ".."))
    {
        return Err("TRS tool/version IDs must be nonempty opaque segments".into());
    }
    Ok(())
}
fn checksums(
    bytes: &[u8],
    advertised: Vec<Checksum>,
    allow_unsupported: bool,
) -> Result<Vec<ChecksumProof>, Box<dyn Error>> {
    let mut proofs = Vec::new();
    for checksum in advertised {
        let verified = match checksum.r#type.to_ascii_lowercase().as_str() {
            "sha-256" | "sha256" => {
                if !checksum.checksum.eq_ignore_ascii_case(&digest(bytes)) {
                    return Err("TRS descriptor checksum mismatch".into());
                }
                true
            }
            _ if allow_unsupported => false,
            _ => {
                return Err(format!(
                    "unsupported advertised checksum algorithm: {}",
                    checksum.r#type
                )
                .into())
            }
        };
        proofs.push(ChecksumProof {
            algorithm: checksum.r#type,
            expected: checksum.checksum,
            verified,
        });
    }
    Ok(proofs)
}
fn relative_reference(document: &str, reference: &str) -> Result<String, Box<dyn Error>> {
    if reference.is_empty() || reference.contains("$(") || reference.contains("${") {
        return Err("dynamic/empty CWL dependency is outside this MVP".into());
    }
    if reference.contains(['?', '#', '\\'])
        || reference.starts_with('/')
        || Url::parse(reference).is_ok()
    {
        return Err("external or fragment CWL dependency is outside this MVP".into());
    }
    let bytes = reference.as_bytes();
    for (i, byte) in bytes.iter().enumerate() {
        if *byte == b'%'
            && (!bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                || !bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
        {
            return Err("malformed percent encoding in CWL dependency".into());
        }
    }
    let decoded = percent_encoding::percent_decode_str(reference).decode_utf8()?;
    let mut parts = document
        .rsplit_once('/')
        .map(|(p, _)| p.split('/').map(str::to_owned).collect::<Vec<_>>())
        .unwrap_or_default();
    for part in decoded.split('/') {
        match part {
            "" | "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return Err("CWL dependency escapes the staged root".into());
                }
            }
            value => parts.push(value.into()),
        }
    }
    let path = parts.join("/");
    validate_filename(&path)?;
    Ok(path)
}
fn references(
    value: &serde_yaml::Value,
    document: &str,
    result: &mut Vec<(String, bool)>,
    step: bool,
) -> Result<(), Box<dyn Error>> {
    match value {
        serde_yaml::Value::Mapping(map) => {
            for (key, value) in map {
                let key = key.as_str();
                if matches!(key, Some("$import" | "$include")) || (step && key == Some("run")) {
                    if let Some(reference) = value.as_str() {
                        result.push((
                            relative_reference(document, reference)?,
                            key != Some("$include"),
                        ));
                    } else if key != Some("run") || !value.is_mapping() {
                        return Err("dynamic CWL dependency is outside this MVP".into());
                    }
                }
                if key == Some("steps")
                    && map
                        .get(serde_yaml::Value::String("class".into()))
                        .and_then(serde_yaml::Value::as_str)
                        == Some("Workflow")
                {
                    match value {
                        serde_yaml::Value::Mapping(steps) => {
                            for (_, value) in steps {
                                references(value, document, result, true)?;
                            }
                        }
                        serde_yaml::Value::Sequence(steps) => {
                            for value in steps {
                                references(value, document, result, true)?;
                            }
                        }
                        _ => return Err("unsupported CWL steps declaration".into()),
                    }
                } else {
                    references(value, document, result, false)?;
                }
            }
        }
        serde_yaml::Value::Sequence(values) => {
            for value in values {
                references(value, document, result, false)?;
            }
        }
        serde_yaml::Value::Tagged(_) => {
            return Err("tagged YAML is outside the CWL bundle MVP".into())
        }
        _ => (),
    }
    Ok(())
}

fn acyclic(graph: &BTreeMap<String, BTreeSet<String>>) -> Result<(), Box<dyn Error>> {
    fn visit(
        node: &str,
        graph: &BTreeMap<String, BTreeSet<String>>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<(), Box<dyn Error>> {
        if done.contains(node) {
            return Ok(());
        }
        if !active.insert(node.into()) {
            return Err("cyclic CWL descriptor dependency".into());
        }
        if let Some(dependencies) = graph.get(node) {
            for dependency in dependencies {
                visit(dependency, graph, active, done)?;
            }
        }
        active.remove(node);
        done.insert(node.into());
        Ok(())
    }
    let mut active = BTreeSet::new();
    let mut done = BTreeSet::new();
    for node in graph.keys() {
        visit(node, graph, &mut active, &mut done)?;
    }
    Ok(())
}
impl PreparedBundle {
    pub fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }
    pub fn request_plan(&self) -> WesRunRequest {
        self.request.clone()
    }
    pub fn staged_files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_ref()))
    }
    pub async fn prepare(
        registry: &Transport,
        request: BundleRequest,
        options: &PreparationOptions,
        operation: &Operation,
        fetcher: Option<&dyn DescriptorFetcher>,
    ) -> Result<Self, Box<dyn Error>> {
        check_ids(&request)?;
        if options.max_files == 0 || options.max_file_bytes == 0 || options.max_total_bytes == 0 {
            return Err("bundle bounds must be positive".into());
        }
        let base = format!(
            "tools/{}/versions/{}",
            urlencode(&request.tool_id),
            urlencode(&request.version_id)
        );
        let version: Value =
            serde_json::from_str(&operation.run(registry.get(&base, None)).await??)?;
        let listed: Vec<ToolFile> = serde_json::from_str(
            &operation
                .run(registry.get(&format!("{base}/CWL/files"), None))
                .await??,
        )?;
        if listed.len() > options.max_files {
            return Err("TRS file list exceeds bundle file bound".into());
        }
        let mut available = BTreeMap::new();
        let mut primaries = Vec::new();
        for file in listed {
            let path = file
                .path
                .as_deref()
                .ok_or("TRS file entry is missing its relative path")?;
            validate_filename(path)?;
            if file.file_type == Some(FileType::PrimaryDescriptor) {
                primaries.push(path.to_owned());
            }
            if available.insert(path.to_owned(), file).is_some() {
                return Err("TRS file path collision".into());
            }
        }
        if primaries.len() != 1 {
            return Err("CWL bundle requires exactly one declared primary descriptor".into());
        }
        let primary = primaries.remove(0);
        let mut queue = VecDeque::from([(primary.clone(), true)]);
        // Stage every declared descriptor, including imports not reached until parsing.
        for (path, file) in &available {
            if file.file_type == Some(FileType::SecondaryDescriptor) {
                queue.push_back((path.clone(), true));
            }
        }
        let mut files: BTreeMap<String, Arc<[u8]>> = BTreeMap::new();
        let mut parsed = BTreeSet::new();
        let mut digests = BTreeMap::new();
        let mut graph = BTreeMap::new();
        let mut total = 0usize;
        let mut cwl_version = None;
        while let Some((path, parse)) = queue.pop_front() {
            if !files.contains_key(&path) {
                let declared = available
                    .get(&path)
                    .ok_or("CWL dependency is missing from the TRS file listing")?;
                let endpoint = if path == primary {
                    format!("{base}/CWL/descriptor")
                } else {
                    format!("{base}/CWL/descriptor/{}", urlencode(&path))
                };
                let wrapper: FileWrapper =
                    serde_json::from_str(&operation.run(registry.get(&endpoint, None)).await??)?;
                let mut advertised = wrapper.checksum.unwrap_or_default();
                if let Some(checksum) = &declared.checksum {
                    advertised.push(*checksum.clone());
                }
                let remaining = options
                    .max_total_bytes
                    .saturating_sub(total)
                    .min(options.max_file_bytes);
                let bytes = if let Some(content) = wrapper.content {
                    content.into_bytes()
                } else if let Some(url) = wrapper.url {
                    let fetcher =
                        fetcher.ok_or("TRS URL descriptor requires an explicit fetch policy")?;
                    fetcher
                        .fetch(&Url::parse(&url)?, operation, remaining)
                        .await?
                } else {
                    return Err("TRS descriptor has neither content nor URL".into());
                };
                if bytes.len() > remaining {
                    return Err("TRS descriptor exceeds bundle byte bound".into());
                }
                total += bytes.len();
                let advertised =
                    checksums(&bytes, advertised, options.allow_unsupported_checksums)?;
                digests.insert(
                    path.clone(),
                    FileDigest {
                        path: path.clone(),
                        sha256: digest(&bytes),
                        advertised,
                    },
                );
                files.insert(path.clone(), bytes.into());
            }
            if parse && parsed.insert(path.clone()) {
                let document: serde_yaml::Value = serde_yaml::from_slice(files[&path].as_ref())?;
                if path == primary {
                    if document.get("class").and_then(serde_yaml::Value::as_str) != Some("Workflow")
                    {
                        return Err("primary CWL descriptor must be a Workflow in this MVP".into());
                    }
                    cwl_version = document
                        .get("cwlVersion")
                        .and_then(serde_yaml::Value::as_str)
                        .map(str::to_owned);
                }
                let mut dependencies = Vec::new();
                references(&document, &path, &mut dependencies, false)?;
                for (dependency, parse) in dependencies {
                    if !available.contains_key(&dependency) {
                        return Err("CWL dependency is missing from the TRS file listing".into());
                    }
                    if parse {
                        graph
                            .entry(path.clone())
                            .or_insert_with(BTreeSet::new)
                            .insert(dependency.clone());
                    }
                    queue.push_back((dependency, parse));
                }
            }
        }
        acyclic(&graph)?;
        let cwl_version = cwl_version
            .filter(|v| matches!(v.as_str(), "v1.0" | "v1.1" | "v1.2"))
            .ok_or("unsupported/missing CWL language version")?;
        let registry = safe_root(registry)?;
        let files_manifest = digests.into_values().collect::<Vec<_>>();
        let is_production = version.get("is_production").and_then(Value::as_bool);
        let meta_version = version
            .get("meta_version")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let canonical = json!({"registry":registry,"tool_id":request.tool_id,"version_id":request.version_id,"is_production":is_production,"meta_version":meta_version,"files":files_manifest});
        let manifest = BundleManifest {
            registry,
            tool_id: request.tool_id,
            version_id: request.version_id,
            is_production,
            meta_version,
            files: files_manifest,
            bundle_sha256: digest(&serde_json::to_vec(&canonical)?),
            execution: None,
        };
        let request = WesRunRequest {
            workflow_type: Some("CWL".into()),
            workflow_type_version: Some(cwl_version),
            workflow_url: Some(primary),
            workflow_params: Some(request.workflow_params),
            ..Default::default()
        };
        Ok(Self {
            files,
            manifest,
            request,
        })
    }
    /// Explicit submission uploads the same immutable Arc bytes that were hashed.
    pub async fn submit(
        &self,
        wes: &WES,
        operation: &Operation,
    ) -> Result<SubmittedBundle, Box<dyn Error>> {
        let attachments = self
            .files
            .iter()
            .map(|(path, bytes)| {
                StreamingAttachment::reader(
                    path.clone(),
                    std::io::Cursor::new(bytes.clone()),
                    Some(bytes.len() as u64),
                )
            })
            .collect();
        let run = wes
            .run_workflow_streaming(self.request.clone(), attachments, operation)
            .await?;
        let mut manifest = self.manifest.clone();
        manifest.execution = Some(ExecutionRecord {
            api_root: safe_root(&wes.transport)?,
            run_id: run.id.clone(),
        });
        Ok(SubmittedBundle { run, manifest })
    }
}
