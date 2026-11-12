//! Client for the GA4GH Tool Registry Service (TRS) API, version 2.0.2.
//!
//! `Configuration.base_path` is the TRS API root, including the service's
//! path prefix, with or without a trailing slash, for example
//! `https://workflowhub.eu/ga4gh/trs/v2` or
//! `https://dockstore.org/api/ga4gh/trs/v2`. All requests are resolved below
//! that root. Tool IDs, version IDs and relative paths are sent as single,
//! percent-encoded path segments, so IDs such as Dockstore's
//! `#workflow/github.com/org/repo` work unchanged.
//!
//! # Examples
//!
//! List workflows and fetch the bare descriptor of a version:
//!
//! ```no_run
//! use ga4gh_sdk::clients::trs::models::{DescriptorType, DescriptorTypeWithPlain};
//! use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
//! use ga4gh_sdk::utils::configuration::Configuration;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = Configuration::new(url::Url::parse("https://workflowhub.eu/ga4gh/trs/v2")?);
//! let trs = TRS::new(&config).await?;
//!
//! let filter = ToolFilter {
//!     tool_class: Some("Workflow".to_string()),
//!     limit: Some(10),
//!     ..Default::default()
//! };
//! for tool in trs.list_tools(&filter).await? {
//!     println!("{} {}", tool.id, tool.name.unwrap_or_default());
//! }
//!
//! let descriptor = trs
//!     .get_descriptor("2", "1", DescriptorTypeWithPlain::PlainGalaxy)
//!     .await?;
//! println!("{}", descriptor.content.unwrap_or_default());
//!
//! // All files of a CWL version as one ZIP archive, at most 10 MiB.
//! let archive = trs
//!     .get_files_zip("2", "1", DescriptorType::Cwl, 10 * 1024 * 1024)
//!     .await?;
//! println!("{} bytes", archive.len());
//! # Ok(())
//! # }
//! ```

pub mod models;
pub mod responses;

use crate::clients::serviceinfo::models::Service;
use crate::clients::trs::models::{
    DescriptorType, DescriptorTypeWithPlain, FileWrapper, Tool, ToolClass, ToolFile, ToolVersion,
};
use crate::utils::configuration::Configuration;
use crate::utils::transport::{read_bounded, urlencode, Transport};
use serde::de::DeserializeOwned;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::error::Error;

// Encodes a path parameter as one URL path segment. Empty values and dot
// segments would change the path that the URL resolves to, so they are
// rejected before any request.
fn segment(name: &str, value: &str) -> Result<String, Box<dyn Error>> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(format!("TRS {name} must not be empty, '.' or '..'").into());
    }
    Ok(urlencode(value))
}

fn version_path(id: &str, version_id: &str) -> Result<String, Box<dyn Error>> {
    Ok(format!(
        "tools/{}/versions/{}",
        segment("tool ID", id)?,
        segment("version ID", version_id)?
    ))
}

// Checks that service info describes a TRS. Some registries (e.g. Dockstore)
// report the artifact as `TRS`, so the match ignores case.
fn is_trs(service: &Service) -> bool {
    service.r#type.group == "org.ga4gh" && service.r#type.artifact.eq_ignore_ascii_case("trs")
}

// Descriptor types that the spec doesn't define count as plain if they use the
// spec's `PLAIN_` prefix.
fn is_plain(descriptor_type: &DescriptorTypeWithPlain) -> bool {
    match descriptor_type {
        DescriptorTypeWithPlain::PlainCwl
        | DescriptorTypeWithPlain::PlainWdl
        | DescriptorTypeWithPlain::PlainNfl
        | DescriptorTypeWithPlain::PlainGalaxy
        | DescriptorTypeWithPlain::PlainSmk => true,
        DescriptorTypeWithPlain::Unrecognized(value) => value.starts_with("PLAIN_"),
        _ => false,
    }
}

// Plain descriptor types return the bare file, not a JSON `FileWrapper`.
fn plain_file(content: String) -> FileWrapper {
    FileWrapper {
        content: Some(content),
        ..FileWrapper::default()
    }
}

// The `PLAIN_*` tests endpoint may return a JSON array of `FileWrapper`s or
// the bare test file. Every field of `FileWrapper` is optional, so an array
// counts as wrappers only if each element has `content` or `url`.
fn plain_tests(body: String) -> Vec<FileWrapper> {
    match serde_json::from_str::<Vec<FileWrapper>>(&body) {
        Ok(files)
            if !files.is_empty()
                && files.iter().all(|f| f.content.is_some() || f.url.is_some()) =>
        {
            files
        }
        _ => vec![plain_file(body)],
    }
}

// `tags` is a query array with `explode: false`: one comma-separated value.
fn join_tags<S: Serializer>(tags: &[String], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&tags.join(","))
}

/// Query parameters for [`TRS::list_tools`], as defined by TRS 2.0.2.
///
/// Every field is optional; unset fields and an empty `tags` list are not
/// sent. `offset` is a string because registries may page by values other
/// than numbers. [`ToolFilter::query`] checks the tags and is the supported
/// way to turn a filter into query parameters, also for
/// [`responses::list_tools_page`]; serializing a filter directly skips the
/// check.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ToolFilter {
    /// A tool ID, scoped to the registry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// A tool alias, for registries that support aliases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// The name of the tool class, e.g. `Workflow` or `CommandLineTool`.
    #[serde(rename = "toolClass", skip_serializing_if = "Option::is_none")]
    pub tool_class: Option<String>,
    /// The descriptor type, e.g. CWL or WDL.
    #[serde(rename = "descriptorType", skip_serializing_if = "Option::is_none")]
    pub descriptor_type: Option<DescriptorType>,
    /// The image registry that contains the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub registry: Option<String>,
    /// The organization in the registry that published the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    /// The name of the image.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The name of the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub toolname: Option<String>,
    /// The description of the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The author of the tool.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Return only checker workflows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker: Option<bool>,
    /// Registry-specific tags (TRS 2.0.2). They are sent as one
    /// comma-separated value, so a tag must not be empty or contain a comma.
    #[serde(skip_serializing_if = "Vec::is_empty", serialize_with = "join_tags")]
    pub tags: Vec<String>,
    /// The start index of the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<String>,
    /// The number of records per page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
}

impl ToolFilter {
    /// Returns the filter as query parameters for
    /// [`Transport`](crate::utils::transport::Transport) requests, or `None`
    /// if no field is set.
    ///
    /// # Errors
    /// If a tag is empty or contains a comma, which `tags` can't send.
    pub fn query(&self) -> Result<Option<Value>, Box<dyn Error>> {
        if self
            .tags
            .iter()
            .any(|tag| tag.is_empty() || tag.contains(','))
        {
            return Err("TRS tags must not be empty or contain a comma".into());
        }
        let params = serde_json::to_value(self)?;
        Ok(match params.as_object() {
            Some(object) if !object.is_empty() => Some(params),
            _ => None,
        })
    }
}

/// The main struct for interacting with a TRS service.
#[derive(Debug)]
pub struct TRS {
    /// The configuration of this client; `base_path` is the TRS API root.
    pub config: Configuration,
    /// The service info fetched when the client was created.
    pub service: Result<Service, Box<dyn Error>>,
    /// The transport layer for sending HTTP requests.
    pub transport: Transport,
}

impl TRS {
    /// Creates a new `TRS` instance and checks that the endpoint is a TRS.
    ///
    /// # Arguments
    /// - `config`: The client configuration; `base_path` is the TRS API root,
    ///   and `timeouts` set the time limits (see
    ///   [`Timeouts`](crate::utils::timeouts::Timeouts)).
    ///
    /// # Returns
    /// - A new `TRS` instance, or an error if the service info can't be
    ///   fetched or doesn't describe a TRS (`org.ga4gh`, artifact `trs`).
    pub async fn new(config: &Configuration) -> Result<Self, Box<dyn Error>> {
        Self::with_transport(Transport::new(config)).await
    }

    /// Creates a new `TRS` instance that sends its requests with `transport`,
    /// for example one from [`Transport::with_client`], and checks that the
    /// endpoint is a TRS. The client's configuration is the transport's.
    pub async fn with_transport(transport: Transport) -> Result<Self, Box<dyn Error>> {
        let mut instance = TRS {
            config: transport.config.clone(),
            transport,
            // Replaced below; `new` never returns an instance without it.
            service: Ok(Service::default()),
        };
        let service = instance.service_info().await?;
        if !is_trs(&service) {
            return Err("The endpoint is not an instance of TRS".into());
        }
        instance.service = Ok(service);
        Ok(instance)
    }

    // `path` is relative to the API root; `Transport` resolves it below
    // `base_path`, with or without a trailing slash.
    async fn get_text(&self, path: &str, params: Option<Value>) -> Result<String, Box<dyn Error>> {
        self.transport.get(path, params).await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        params: Option<Value>,
    ) -> Result<T, Box<dyn Error>> {
        let body = self.get_text(path, params).await?;
        Ok(serde_json::from_str(&body)?)
    }

    /// Fetches the service info of the registry (`GET /service-info`), within
    /// the `discovery` limit of the transport's configuration.
    ///
    /// Dockstore sends `createdAt` and `updatedAt` as numbers, while
    /// service-info defines them as strings; such non-string values are
    /// dropped instead of failing the whole response.
    pub async fn service_info(&self) -> Result<Service, Box<dyn Error>> {
        let body = crate::clients::serviceinfo::fetch_raw(&self.transport, "service-info").await?;
        let mut value: Value = serde_json::from_str(&body)?;
        if let Some(object) = value.as_object_mut() {
            for key in ["createdAt", "updatedAt"] {
                if object.get(key).is_some_and(|v| !v.is_string()) {
                    log::debug!("Ignoring non-string TRS service-info field {key}");
                    object.remove(key);
                }
            }
        }
        Ok(serde_json::from_value(value)?)
    }

    /// Lists the tool classes of the registry (`GET /toolClasses`).
    pub async fn list_tool_classes(&self) -> Result<Vec<ToolClass>, Box<dyn Error>> {
        self.get_json("toolClasses", None).await
    }

    /// Lists tools, filtered by `filter` (`GET /tools`).
    ///
    /// Returns one page; use `filter.offset` and `filter.limit` to page.
    /// Fails before sending a request if a tag can't be sent (see
    /// [`ToolFilter::query`]).
    pub async fn list_tools(&self, filter: &ToolFilter) -> Result<Vec<Tool>, Box<dyn Error>> {
        self.get_json("tools", filter.query()?).await
    }

    /// Fetches one tool, including its versions (`GET /tools/{id}`).
    pub async fn get_tool(&self, id: &str) -> Result<Tool, Box<dyn Error>> {
        let path = format!("tools/{}", segment("tool ID", id)?);
        self.get_json(&path, None).await
    }

    /// Lists the versions of a tool (`GET /tools/{id}/versions`).
    pub async fn list_versions(&self, id: &str) -> Result<Vec<ToolVersion>, Box<dyn Error>> {
        let path = format!("tools/{}/versions", segment("tool ID", id)?);
        self.get_json(&path, None).await
    }

    /// Fetches one version of a tool (`GET /tools/{id}/versions/{version_id}`).
    pub async fn get_version(
        &self,
        id: &str,
        version_id: &str,
    ) -> Result<ToolVersion, Box<dyn Error>> {
        let path = version_path(id, version_id)?;
        self.get_json(&path, None).await
    }

    /// Fetches the primary descriptor of a tool version
    /// (`GET /tools/{id}/versions/{version_id}/{type}/descriptor`).
    ///
    /// For `PLAIN_*` types the registry returns the bare file; it is returned
    /// as a `FileWrapper` with only `content` set.
    pub async fn get_descriptor(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorTypeWithPlain,
    ) -> Result<FileWrapper, Box<dyn Error>> {
        let path = format!(
            "{}/{}/descriptor",
            version_path(id, version_id)?,
            segment("descriptor type", &descriptor_type.to_string())?
        );
        self.get_file(&path, descriptor_type).await
    }

    /// Fetches a descriptor or other file by its path relative to the primary
    /// descriptor
    /// (`GET /tools/{id}/versions/{version_id}/{type}/descriptor/{relative_path}`).
    ///
    /// `relative_path` (e.g. `tools/align.cwl`) is sent as one encoded path
    /// segment. `PLAIN_*` types are handled as in [`TRS::get_descriptor`].
    pub async fn get_descriptor_file(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorTypeWithPlain,
        relative_path: &str,
    ) -> Result<FileWrapper, Box<dyn Error>> {
        let path = format!(
            "{}/{}/descriptor/{}",
            version_path(id, version_id)?,
            segment("descriptor type", &descriptor_type.to_string())?,
            segment("relative path", relative_path)?
        );
        self.get_file(&path, descriptor_type).await
    }

    async fn get_file(
        &self,
        path: &str,
        descriptor_type: DescriptorTypeWithPlain,
    ) -> Result<FileWrapper, Box<dyn Error>> {
        if is_plain(&descriptor_type) {
            Ok(plain_file(self.get_text(path, None).await?))
        } else {
            self.get_json(path, None).await
        }
    }

    /// Lists the test parameter files of a tool version
    /// (`GET /tools/{id}/versions/{version_id}/{type}/tests`).
    ///
    /// For `PLAIN_*` types, registries such as Dockstore return the bare test
    /// file (possibly several joined into one body); it is returned as a
    /// single `FileWrapper` with only `content` set. A body that is a JSON
    /// array of file wrappers, as the spec also allows, is returned as such.
    pub async fn list_tests(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorTypeWithPlain,
    ) -> Result<Vec<FileWrapper>, Box<dyn Error>> {
        let path = format!(
            "{}/{}/tests",
            version_path(id, version_id)?,
            segment("descriptor type", &descriptor_type.to_string())?
        );
        if is_plain(&descriptor_type) {
            Ok(plain_tests(self.get_text(&path, None).await?))
        } else {
            self.get_json(&path, None).await
        }
    }

    /// Lists the files of a tool version with their paths and file types
    /// (`GET /tools/{id}/versions/{version_id}/{type}/files`).
    ///
    /// To download the files themselves, use [`TRS::get_files_zip`].
    pub async fn list_files(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorType,
    ) -> Result<Vec<ToolFile>, Box<dyn Error>> {
        let path = format!(
            "{}/{}/files",
            version_path(id, version_id)?,
            segment("descriptor type", &descriptor_type.to_string())?
        );
        self.get_json(&path, None).await
    }

    /// Downloads all files of a tool version as one ZIP archive
    /// (`GET /tools/{id}/versions/{version_id}/{type}/files?format=zip`).
    ///
    /// The archive is read into memory, at most `limit` bytes; a larger body
    /// fails with a [`BodyLimitError`](crate::utils::transport::BodyLimitError).
    /// A response without a ZIP file signature (for example the JSON file
    /// list of a registry that ignores `format`) is an error. This does not
    /// validate the archive's contents or extract it. To stream large
    /// archives instead, use
    /// [`responses::files_zip`] with `self.transport`.
    pub async fn get_files_zip(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorType,
        limit: usize,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        // Validate before building the request, with the same messages as
        // the other methods (`files_zip` checks again, more generally).
        version_path(id, version_id)?;
        let response =
            responses::files_zip(&self.transport, id, version_id, descriptor_type).await?;
        let body = read_bounded(response, limit).await?.body;
        // Local-file and empty-archive signatures. A bare `PK` prefix is not
        // enough to distinguish an archive from an arbitrary response body.
        if !body.starts_with(b"PK\x03\x04") && !body.starts_with(b"PK\x05\x06") {
            return Err("TRS response is not a ZIP archive".into());
        }
        Ok(body)
    }

    /// Fetches the container specifications, e.g. Dockerfiles, of a tool
    /// version (`GET /tools/{id}/versions/{version_id}/containerfile`).
    pub async fn get_containerfiles(
        &self,
        id: &str,
        version_id: &str,
    ) -> Result<Vec<FileWrapper>, Box<dyn Error>> {
        let path = format!("{}/containerfile", version_path(id, version_id)?);
        self.get_json(&path, None).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::trs::models::tool_file::FileType;
    use crate::utils::transport::BodyLimitError;
    use mockito::{mock, server_url, Matcher, Mock};
    use url::Url;

    // A Dockstore-style ID and a version ID that both need encoding.
    const TOOL_ID: &str = "#workflow/github.com/org/repo";
    const TOOL_ID_ENCODED: &str = "%23workflow%2Fgithub.com%2Forg%2Frepo";
    const VERSION_ID: &str = "v1/rc 1";
    const VERSION_ID_ENCODED: &str = "v1%2Frc%201";

    fn tool_json() -> Value {
        serde_json::json!({
            "url": "https://example.org/tools/1",
            "id": TOOL_ID,
            "organization": "org",
            "toolclass": { "id": "1", "name": "Workflow" },
            "versions": [version_json()]
        })
    }

    fn version_json() -> Value {
        serde_json::json!({
            "url": "https://example.org/tools/1/versions/1",
            "id": VERSION_ID,
            "descriptor_type": ["CWL"],
            "description": "First release"
        })
    }

    fn service_info_json(artifact: &str) -> Value {
        serde_json::json!({
            "id": "org.example.trs",
            "name": "Example TRS",
            "type": { "group": "org.ga4gh", "artifact": artifact, "version": "2.0.2" },
            "organization": { "name": "Example", "url": "https://example.org" },
            "version": "1.0.0"
        })
    }

    // Clients for the API root `<mock server>/<prefix>/ga4gh/trs/v2`, without
    // and with a trailing slash. Each test uses its own prefix, because
    // mockito's server is shared by all tests.
    fn clients(prefix: &str) -> Vec<TRS> {
        ["", "/"]
            .iter()
            .map(|slash| {
                let base = format!("{}/{prefix}/ga4gh/trs/v2{slash}", server_url());
                let config = Configuration::new(Url::parse(&base).unwrap());
                TRS {
                    transport: Transport::new(&config),
                    config,
                    service: Ok(Service::default()),
                }
            })
            .collect()
    }

    fn path(prefix: &str, api_path: &str) -> String {
        format!("/{prefix}/ga4gh/trs/v2/{api_path}")
    }

    // The API path of the test tool version, plus `rest` if not empty.
    fn tool_version(rest: &str) -> String {
        let base = format!("tools/{TOOL_ID_ENCODED}/versions/{VERSION_ID_ENCODED}");
        if rest.is_empty() {
            base
        } else {
            format!("{base}/{rest}")
        }
    }

    // Mocks a GET that must be called once by each client from `clients`.
    fn get_mock(prefix: &str, api_path: &str, body: &str) -> Mock {
        mock("GET", path(prefix, api_path).as_str())
            .with_status(200)
            .with_body(body)
            .expect(2)
            .create()
    }

    #[test]
    fn test_debug_output() {
        let config = crate::utils::configuration::config_with_markers();
        let trs = TRS {
            transport: Transport::new(&config),
            config,
            service: Ok(Service::default()),
        };
        let output = format!("{:?}", trs);
        assert!(
            output.starts_with("TRS { config: Configuration {"),
            "{output}"
        );
        assert!(!output.contains("SENTINEL"), "{output}");
    }

    #[test]
    fn test_url_preserves_encoded_base_path() {
        for slash in ["", "/"] {
            let base = Url::parse(&format!("https://example.org/a%20b/trs/v2{slash}")).unwrap();
            let config = Configuration::new(base);
            let trs = TRS {
                transport: Transport::new(&config),
                config,
                service: Ok(Service::default()),
            };
            assert_eq!(
                trs.transport.endpoint("tools/a%2Fb").unwrap().as_str(),
                "https://example.org/a%20b/trs/v2/tools/a%2Fb"
            );
        }
    }

    #[test]
    fn test_urlencode_encodes_path_separators() {
        assert_eq!(urlencode(TOOL_ID), TOOL_ID_ENCODED);
        assert_eq!(urlencode(VERSION_ID), VERSION_ID_ENCODED);
        assert_eq!(urlencode("a?b%c"), "a%3Fb%25c");
    }

    #[tokio::test]
    async fn test_new_checks_service_info_below_api_root() {
        let m = get_mock("new", "service-info", &service_info_json("trs").to_string());
        for client in clients("new") {
            let trs = TRS::new(&client.config).await.unwrap();
            assert_eq!(trs.service.unwrap().r#type.artifact, "trs");
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_old_trs_service_and_version_without_description() {
        let mut info = service_info_json("trs");
        info["type"]["version"] = serde_json::json!("2.0.1");
        let service = get_mock("trs-201", "service-info", &info.to_string());
        let mut version = version_json();
        version.as_object_mut().unwrap().remove("description");
        let response = get_mock("trs-201", &tool_version(""), &version.to_string());
        for client in clients("trs-201") {
            let trs = TRS::new(&client.config).await.unwrap();
            assert_eq!(trs.service.as_ref().unwrap().r#type.version, "2.0.1");
            assert_eq!(
                trs.get_version(TOOL_ID, VERSION_ID)
                    .await
                    .unwrap()
                    .description,
                None
            );
        }
        service.assert();
        response.assert();
    }

    #[tokio::test]
    async fn test_new_accepts_dockstore_service_info() {
        let mut info = service_info_json("TRS");
        info["createdAt"] = serde_json::json!(1593648000000u64);
        info["updatedAt"] = serde_json::json!("2026-11-09T00:00:00Z");
        let mut numeric_update = info.clone();
        numeric_update["updatedAt"] = serde_json::json!(1794200849592u64);
        let m = get_mock("dockstore", "service-info", &info.to_string());
        for client in clients("dockstore") {
            let service = TRS::new(&client.config).await.unwrap().service.unwrap();
            assert_eq!(service.created_at, None);
            assert_eq!(service.updated_at.as_deref(), Some("2026-11-09T00:00:00Z"));
        }
        m.assert();
        let m = get_mock("dockstore-2", "service-info", &numeric_update.to_string());
        for client in clients("dockstore-2") {
            let service = TRS::new(&client.config).await.unwrap().service.unwrap();
            assert_eq!((service.created_at, service.updated_at), (None, None));
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_new_rejects_other_services() {
        let m = get_mock(
            "not-trs",
            "service-info",
            &service_info_json("wes").to_string(),
        );
        for client in clients("not-trs") {
            let error = TRS::new(&client.config).await.unwrap_err();
            assert!(
                error.to_string().contains("not an instance of TRS"),
                "{error}"
            );
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_new_fails_without_service_info() {
        let _m = mock("GET", path("no-info", "service-info").as_str())
            .with_status(404)
            .create();
        for client in clients("no-info") {
            assert!(TRS::new(&client.config).await.is_err());
        }
    }

    #[tokio::test]
    async fn test_service_info() {
        let m = get_mock(
            "info",
            "service-info",
            &service_info_json("trs").to_string(),
        );
        for client in clients("info") {
            assert_eq!(client.service_info().await.unwrap().name, "Example TRS");
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_list_tool_classes() {
        let body = r#"[{"id":"1","name":"Workflow","description":"A workflow"}]"#;
        let m = get_mock("classes", "toolClasses", body);
        for client in clients("classes") {
            let classes = client.list_tool_classes().await.unwrap();
            assert_eq!(classes[0].name.as_deref(), Some("Workflow"));
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_list_tools_without_filter_sends_no_query() {
        let m = mock("GET", path("tools-all", "tools").as_str())
            .match_query(Matcher::Missing)
            .with_status(200)
            .with_body(serde_json::json!([tool_json()]).to_string())
            .expect(2)
            .create();
        for client in clients("tools-all") {
            let tools = client.list_tools(&ToolFilter::default()).await.unwrap();
            assert_eq!(tools[0].id, TOOL_ID);
            assert_eq!(tools[0].versions[0].id, VERSION_ID);
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_list_tools_sends_filters() {
        let m = mock("GET", path("tools-filter", "tools").as_str())
            .match_query(Matcher::AllOf(vec![
                Matcher::UrlEncoded("id".into(), "a/b".into()),
                Matcher::UrlEncoded("alias".into(), "x y".into()),
                Matcher::UrlEncoded("toolClass".into(), "Workflow".into()),
                Matcher::UrlEncoded("descriptorType".into(), "NFL".into()),
                Matcher::UrlEncoded("registry".into(), "quay.io".into()),
                Matcher::UrlEncoded("organization".into(), "org".into()),
                Matcher::UrlEncoded("name".into(), "image".into()),
                Matcher::UrlEncoded("toolname".into(), "tool".into()),
                Matcher::UrlEncoded("description".into(), "a&b".into()),
                Matcher::UrlEncoded("author".into(), "Ada".into()),
                Matcher::UrlEncoded("checker".into(), "false".into()),
                Matcher::UrlEncoded("offset".into(), "abc".into()),
                Matcher::UrlEncoded("limit".into(), "5".into()),
                // `explode: false`: one comma-separated value.
                Matcher::UrlEncoded("tags".into(), "a b,c&d".into()),
            ]))
            .with_status(200)
            .with_body("[]")
            .expect(2)
            .create();
        let filter = ToolFilter {
            id: Some("a/b".into()),
            alias: Some("x y".into()),
            tool_class: Some("Workflow".into()),
            descriptor_type: Some(DescriptorType::Nfl),
            registry: Some("quay.io".into()),
            organization: Some("org".into()),
            name: Some("image".into()),
            toolname: Some("tool".into()),
            description: Some("a&b".into()),
            author: Some("Ada".into()),
            checker: Some(false),
            tags: vec!["a b".into(), "c&d".into()],
            offset: Some("abc".into()),
            limit: Some(5),
        };
        for client in clients("tools-filter") {
            assert!(client.list_tools(&filter).await.unwrap().is_empty());
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_get_tool() {
        let m = get_mock(
            "tool",
            &format!("tools/{TOOL_ID_ENCODED}"),
            &tool_json().to_string(),
        );
        for client in clients("tool") {
            let tool = client.get_tool(TOOL_ID).await.unwrap();
            assert_eq!(tool.id, TOOL_ID);
            assert_eq!(tool.toolclass.name.as_deref(), Some("Workflow"));
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_get_tool_returns_http_errors() {
        let _m = mock("GET", path("tool-404", "tools/missing").as_str())
            .with_status(404)
            .with_body(r#"{"code":404,"message":"not found"}"#)
            .create();
        for client in clients("tool-404") {
            let error = client.get_tool("missing").await.unwrap_err();
            assert!(error.to_string().contains("404"), "{error}");
        }
    }

    #[tokio::test]
    async fn test_returns_errors_for_bodies_that_are_not_json() {
        let _invalid = mock("GET", path("bad-body", "toolClasses").as_str())
            .with_status(200)
            .with_body("[{\"id\":")
            .create();
        let _html = mock("GET", path("bad-body", "tools").as_str())
            .with_status(500)
            .with_header("content-type", "text/html")
            .with_body("<html>Internal Server Error</html>")
            .create();
        let _info = mock("GET", path("bad-body", "service-info").as_str())
            .with_status(200)
            .with_body("<html>Not a TRS</html>")
            .create();
        for client in clients("bad-body") {
            assert!(client.list_tool_classes().await.is_err());
            let error = client.list_tools(&ToolFilter::default()).await.unwrap_err();
            assert!(error.to_string().contains("500"), "{error}");
            assert!(TRS::new(&client.config).await.is_err());
        }
    }

    #[tokio::test]
    async fn test_rejects_ids_that_change_the_path() {
        // No mock: these must fail before any request is sent.
        for client in clients("bad-ids") {
            for id in ["", ".", ".."] {
                let error = client.get_tool(id).await.unwrap_err();
                assert!(error.to_string().contains("tool ID"), "{error}");
                let error = client.get_version("1", id).await.unwrap_err();
                assert!(error.to_string().contains("version ID"), "{error}");
                let error = client
                    .get_descriptor_file("1", "1", DescriptorTypeWithPlain::Cwl, id)
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains("relative path"), "{error}");
                let error = client
                    .get_descriptor("1", "1", DescriptorTypeWithPlain::Unrecognized(id.into()))
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains("descriptor type"), "{error}");
            }
        }
    }

    #[tokio::test]
    async fn test_unrecognized_descriptor_types_are_one_encoded_segment() {
        let json = get_mock(
            "unrecognized",
            &tool_version("JUPYTER%2F..%3Fx/descriptor"),
            r#"{"content":"notebook"}"#,
        );
        let plain = get_mock(
            "unrecognized",
            &tool_version("PLAIN_JUPYTER/descriptor"),
            "notebook",
        );
        for client in clients("unrecognized") {
            let file = client
                .get_descriptor(
                    TOOL_ID,
                    VERSION_ID,
                    DescriptorTypeWithPlain::Unrecognized("JUPYTER/..?x".into()),
                )
                .await
                .unwrap();
            assert_eq!(file.content.as_deref(), Some("notebook"));
            // The spec's `PLAIN_` prefix marks a bare file, also for types that
            // the spec doesn't define.
            let file = client
                .get_descriptor(
                    TOOL_ID,
                    VERSION_ID,
                    DescriptorTypeWithPlain::Unrecognized("PLAIN_JUPYTER".into()),
                )
                .await
                .unwrap();
            assert_eq!(file, plain_file("notebook".into()));
        }
        json.assert();
        plain.assert();
    }

    #[tokio::test]
    async fn test_list_versions() {
        let m = get_mock(
            "versions",
            &format!("tools/{TOOL_ID_ENCODED}/versions"),
            &serde_json::json!([version_json()]).to_string(),
        );
        for client in clients("versions") {
            let versions = client.list_versions(TOOL_ID).await.unwrap();
            assert_eq!(versions[0].id, VERSION_ID);
            assert_eq!(versions[0].descriptor_type, Some(vec![DescriptorType::Cwl]));
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_get_version() {
        let m = get_mock("version", &tool_version(""), &version_json().to_string());
        for client in clients("version") {
            let version = client.get_version(TOOL_ID, VERSION_ID).await.unwrap();
            assert_eq!(version.id, VERSION_ID);
            assert_eq!(version.description.as_deref(), Some("First release"));
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_get_version_rejects_malformed_description() {
        let mut version = version_json();
        version["description"] = serde_json::json!(42);
        let m = get_mock("bad-description", &tool_version(""), &version.to_string());
        for client in clients("bad-description") {
            let error = client.get_version(TOOL_ID, VERSION_ID).await.unwrap_err();
            assert!(error.to_string().contains("invalid type"), "{error}");
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_get_descriptor_per_type() {
        let cases = [
            (DescriptorTypeWithPlain::Cwl, "CWL"),
            (DescriptorTypeWithPlain::Wdl, "WDL"),
            (DescriptorTypeWithPlain::Nfl, "NFL"),
            (DescriptorTypeWithPlain::Galaxy, "GALAXY"),
            (DescriptorTypeWithPlain::Smk, "SMK"),
        ];
        for (descriptor_type, name) in cases {
            let body = serde_json::json!({
                "content": format!("{name} content"),
                "checksum": [{ "checksum": "abc", "type": "sha-256" }],
                "url": "https://example.org/main"
            });
            let m = get_mock(
                "descriptor",
                &tool_version(&format!("{name}/descriptor")),
                &body.to_string(),
            );
            for client in clients("descriptor") {
                let file = client
                    .get_descriptor(TOOL_ID, VERSION_ID, descriptor_type.clone())
                    .await
                    .unwrap();
                assert_eq!(file.content, Some(format!("{name} content")));
                assert_eq!(file.checksum.unwrap()[0].checksum, "abc");
            }
            m.assert();
        }
    }

    #[tokio::test]
    async fn test_get_plain_descriptor_per_type() {
        let cases = [
            (DescriptorTypeWithPlain::PlainCwl, "PLAIN_CWL"),
            (DescriptorTypeWithPlain::PlainWdl, "PLAIN_WDL"),
            (DescriptorTypeWithPlain::PlainNfl, "PLAIN_NFL"),
            (DescriptorTypeWithPlain::PlainGalaxy, "PLAIN_GALAXY"),
            (DescriptorTypeWithPlain::PlainSmk, "PLAIN_SMK"),
        ];
        for (descriptor_type, name) in cases {
            // A bare file that isn't JSON must not be parsed as JSON.
            let body = format!("#!/usr/bin/env runner\n{name}: [\n");
            let m = get_mock(
                "plain-descriptor",
                &tool_version(&format!("{name}/descriptor")),
                &body,
            );
            for client in clients("plain-descriptor") {
                let file = client
                    .get_descriptor(TOOL_ID, VERSION_ID, descriptor_type.clone())
                    .await
                    .unwrap();
                assert_eq!(file, plain_file(body.clone()));
            }
            m.assert();
        }
    }

    #[tokio::test]
    async fn test_get_descriptor_file() {
        let m = get_mock(
            "relative",
            &tool_version("WDL/descriptor/tasks%2Falign%20v2.wdl"),
            r#"{"content":"task align {}"}"#,
        );
        let plain = get_mock(
            "relative",
            &tool_version("PLAIN_WDL/descriptor/tasks%2Falign%20v2.wdl"),
            "task align {}",
        );
        for client in clients("relative") {
            for descriptor_type in [
                DescriptorTypeWithPlain::Wdl,
                DescriptorTypeWithPlain::PlainWdl,
            ] {
                let file = client
                    .get_descriptor_file(TOOL_ID, VERSION_ID, descriptor_type, "tasks/align v2.wdl")
                    .await
                    .unwrap();
                assert_eq!(file.content.as_deref(), Some("task align {}"));
            }
        }
        m.assert();
        plain.assert();
    }

    #[tokio::test]
    async fn test_list_tests() {
        let m = get_mock(
            "tests",
            &tool_version("CWL/tests"),
            r#"[{"content":"{\"input\":1}"},{"url":"https://example.org/t2.json"}]"#,
        );
        let plain = get_mock("tests", &tool_version("PLAIN_CWL/tests"), "{\"input\":1}");
        for client in clients("tests") {
            let tests = client
                .list_tests(TOOL_ID, VERSION_ID, DescriptorTypeWithPlain::Cwl)
                .await
                .unwrap();
            assert_eq!(tests.len(), 2);
            assert_eq!(tests[1].url.as_deref(), Some("https://example.org/t2.json"));
            let tests = client
                .list_tests(TOOL_ID, VERSION_ID, DescriptorTypeWithPlain::PlainCwl)
                .await
                .unwrap();
            assert_eq!(tests, vec![plain_file("{\"input\":1}".to_string())]);
        }
        m.assert();
        plain.assert();
    }

    #[test]
    fn test_plain_tests_accepts_wrapped_and_bare_bodies() {
        let wrapped = r#"[{"content":"{}"},{"url":"https://example.org/t.json"}]"#;
        assert_eq!(plain_tests(wrapped.to_string()).len(), 2);
        // A bare test file that is itself a JSON array stays one file.
        for bare in [r#"[{"input":1}]"#, "[]", "input: 1"] {
            assert_eq!(
                plain_tests(bare.to_string()),
                vec![plain_file(bare.to_string())]
            );
        }
    }

    #[tokio::test]
    async fn test_list_files() {
        let body = serde_json::json!([
            { "path": "main.nf", "file_type": "PRIMARY_DESCRIPTOR",
              "checksum": { "checksum": "abc", "type": "sha-256" } },
            { "path": "Dockerfile", "file_type": "CONTAINERFILE" }
        ]);
        let m = get_mock("files", &tool_version("NFL/files"), &body.to_string());
        for client in clients("files") {
            let files = client
                .list_files(TOOL_ID, VERSION_ID, DescriptorType::Nfl)
                .await
                .unwrap();
            assert_eq!(files[0].path.as_deref(), Some("main.nf"));
            assert_eq!(files[0].file_type, Some(FileType::PrimaryDescriptor));
            assert_eq!(files[1].file_type, Some(FileType::Containerfile));
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_list_tools_rejects_tags_that_cannot_be_sent() {
        // No mock: these must fail before any request is sent.
        for client in clients("bad-tags") {
            for tag in ["", "a,b"] {
                let filter = ToolFilter {
                    tags: vec!["ok".into(), tag.into()],
                    ..Default::default()
                };
                let error = client.list_tools(&filter).await.unwrap_err();
                assert!(error.to_string().contains("TRS tag"), "{error}");
            }
        }
    }

    // The shared query encoding percent-encodes the separating comma
    // (`tags=a%2Cb`); servers decode the value before splitting it.
    #[tokio::test]
    async fn test_list_tools_sends_tags_with_an_encoded_comma() {
        let m = mock("GET", path("tags-wire", "tools").as_str())
            .match_query(Matcher::Exact("tags=a%2Cb".into()))
            .with_status(200)
            .with_body("[]")
            .expect(2)
            .create();
        let filter = ToolFilter {
            tags: vec!["a".into(), "b".into()],
            ..Default::default()
        };
        for client in clients("tags-wire") {
            assert!(client.list_tools(&filter).await.unwrap().is_empty());
        }
        m.assert();
    }

    #[test]
    fn test_tool_filter_query() {
        assert_eq!(ToolFilter::default().query().unwrap(), None);
        let filter = ToolFilter {
            tags: vec!["x".into(), "y".into()],
            limit: Some(1),
            ..Default::default()
        };
        assert_eq!(
            filter.query().unwrap(),
            Some(serde_json::json!({ "tags": "x,y", "limit": 1 }))
        );
    }

    #[tokio::test]
    async fn test_get_files_zip() {
        let bytes = include_bytes!("../../../../tests/fixtures/trs/workflow.zip");
        let m = mock("GET", path("zip", &tool_version("CWL/files")).as_str())
            .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
            .with_status(200)
            .with_header("content-type", "application/zip")
            .with_body(bytes.as_slice())
            .expect(2)
            .create();
        for client in clients("zip") {
            let archive = client
                .get_files_zip(TOOL_ID, VERSION_ID, DescriptorType::Cwl, bytes.len())
                .await
                .unwrap();
            assert_eq!(archive, bytes);
        }
        m.assert();
    }

    #[tokio::test]
    async fn test_get_files_zip_checks_limit_and_body() {
        let too_large = mock(
            "GET",
            path("zip-limit", &tool_version("CWL/files")).as_str(),
        )
        .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
        .with_status(200)
        .with_body("PK\x03\x04 more than ten bytes")
        .expect(2)
        .create();
        let not_zip = mock("GET", path("zip-json", &tool_version("CWL/files")).as_str())
            .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
            .with_status(200)
            .with_body(r#"[{"path":"main.cwl"}]"#)
            .expect(2)
            .create();
        let fake_zip = mock("GET", path("zip-fake", &tool_version("CWL/files")).as_str())
            .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
            .with_status(200)
            .with_body("PK not an archive")
            .expect(2)
            .create();
        let missing = mock("GET", path("zip-404", &tool_version("CWL/files")).as_str())
            .match_query(Matcher::UrlEncoded("format".into(), "zip".into()))
            .with_status(404)
            .expect(2)
            .create();
        for client in clients("zip-limit") {
            let error = client
                .get_files_zip(TOOL_ID, VERSION_ID, DescriptorType::Cwl, 10)
                .await
                .unwrap_err();
            assert!(error.is::<BodyLimitError>(), "{error}");
        }
        for client in clients("zip-json") {
            let error = client
                .get_files_zip(TOOL_ID, VERSION_ID, DescriptorType::Cwl, 1024)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("not a ZIP archive"), "{error}");
        }
        for client in clients("zip-fake") {
            let error = client
                .get_files_zip(TOOL_ID, VERSION_ID, DescriptorType::Cwl, 1024)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("not a ZIP archive"), "{error}");
        }
        for client in clients("zip-404") {
            let error = client
                .get_files_zip(TOOL_ID, VERSION_ID, DescriptorType::Cwl, 1024)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("404"), "{error}");
        }
        for client in clients("zip-ids") {
            let error = client
                .get_files_zip("..", VERSION_ID, DescriptorType::Cwl, 1024)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("tool ID"), "{error}");
        }
        too_large.assert();
        not_zip.assert();
        fake_zip.assert();
        missing.assert();
    }

    #[tokio::test]
    async fn test_get_containerfiles() {
        let body = serde_json::json!([
            { "content": "FROM alpine", "image_type": "Docker" }
        ]);
        let m = get_mock(
            "containerfile",
            &tool_version("containerfile"),
            &body.to_string(),
        );
        for client in clients("containerfile") {
            let files = client
                .get_containerfiles(TOOL_ID, VERSION_ID)
                .await
                .unwrap();
            assert_eq!(files[0].content.as_deref(), Some("FROM alpine"));
        }
        m.assert();
    }
}
