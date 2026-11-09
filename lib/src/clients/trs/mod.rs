//! Client for the GA4GH Tool Registry Service (TRS) API, version 2.0.1.
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
//! use ga4gh_sdk::clients::trs::models::DescriptorTypeWithPlain;
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
//! # Ok(())
//! # }
//! ```

pub mod models;

use crate::clients::serviceinfo::models::Service;
use crate::clients::trs::models::{
    DescriptorType, DescriptorTypeWithPlain, FileWrapper, Tool, ToolClass, ToolFile, ToolVersion,
};
use crate::utils::configuration::Configuration;
use crate::utils::transport::Transport;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::error::Error;

// Percent-encodes one URL path segment, including `/`, `#` and spaces.
// TODO: use `crate::utils::transport::urlencode` once it lands there (#109).
fn urlencode(value: &str) -> String {
    value
        .as_bytes()
        .iter()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                (*byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

// Encodes a path parameter. Empty values and dot segments would change the
// path that the URL resolves to, so they are rejected before any request.
fn segment(name: &str, value: &str) -> Result<String, Box<dyn Error>> {
    if value.is_empty() || value == "." || value == ".." {
        return Err(format!("TRS {name} must not be empty, '.' or '..'").into());
    }
    Ok(urlencode(value))
}

fn is_plain(descriptor_type: DescriptorTypeWithPlain) -> bool {
    matches!(
        descriptor_type,
        DescriptorTypeWithPlain::PlainCwl
            | DescriptorTypeWithPlain::PlainWdl
            | DescriptorTypeWithPlain::PlainNfl
            | DescriptorTypeWithPlain::PlainGalaxy
            | DescriptorTypeWithPlain::PlainSmk
    )
}

// Plain descriptor types return the bare file, not a JSON `FileWrapper`.
fn plain_file(content: String) -> FileWrapper {
    FileWrapper {
        content: Some(content),
        ..FileWrapper::default()
    }
}

/// Query parameters for [`TRS::list_tools`], as defined by TRS 2.0.1.
///
/// Every field is optional; unset fields are not sent. `offset` is a string
/// because registries may page by values other than numbers.
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
    /// The start index of the page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<String>,
    /// The number of records per page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i32>,
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
    /// - `config`: The client configuration; `base_path` is the TRS API root.
    ///
    /// # Returns
    /// - A new `TRS` instance, or an error if the service info can't be
    ///   fetched or doesn't describe a TRS (`org.ga4gh`, artifact `trs`).
    pub async fn new(config: &Configuration) -> Result<Self, Box<dyn Error>> {
        let transport = Transport::new(config);
        let mut instance = TRS {
            config: config.clone(),
            transport,
            service: Err("service info not fetched yet".into()),
        };
        let service =
            tokio::time::timeout(std::time::Duration::from_secs(10), instance.service_info())
                .await??;
        instance.service = Ok(service);
        instance.check()?;
        Ok(instance)
    }

    // Checks that the service info describes a TRS. Some registries (e.g.
    // Dockstore) report the artifact as `TRS`, so the match ignores case.
    fn check(&self) -> Result<(), String> {
        match self.service.as_ref() {
            Ok(service)
                if service.r#type.group == "org.ga4gh"
                    && service.r#type.artifact.eq_ignore_ascii_case("trs") =>
            {
                Ok(())
            }
            Ok(_) => Err("The endpoint is not an instance of TRS".into()),
            Err(e) => Err(format!("Error accessing TRS service: {e}")),
        }
    }

    // Resolves `path` below the API root, with or without a trailing slash.
    // TODO: use `Transport::endpoint` with API-relative paths once #109 lands.
    fn url(&self, path: &str) -> Result<String, Box<dyn Error>> {
        let mut base = self.config.base_path.clone();
        if !base.path().ends_with('/') {
            base.set_path(&format!("{}/", base.path()));
        }
        Ok(base.join(path)?.to_string())
    }

    async fn get_text(&self, path: &str, params: Option<Value>) -> Result<String, Box<dyn Error>> {
        self.transport.get(&self.url(path)?, params).await
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        params: Option<Value>,
    ) -> Result<T, Box<dyn Error>> {
        let body = self.get_text(path, params).await?;
        Ok(serde_json::from_str(&body)?)
    }

    /// Fetches the service info of the registry (`GET /service-info`).
    ///
    /// Dockstore sends `createdAt` and `updatedAt` as numbers, while
    /// service-info defines them as strings; such non-string values are
    /// dropped instead of failing the whole response.
    pub async fn service_info(&self) -> Result<Service, Box<dyn Error>> {
        let mut value: Value = self.get_json("service-info", None).await?;
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
    pub async fn list_tools(&self, filter: &ToolFilter) -> Result<Vec<Tool>, Box<dyn Error>> {
        let params = serde_json::to_value(filter)?;
        let params = match params.as_object() {
            Some(object) if !object.is_empty() => Some(params),
            _ => None,
        };
        self.get_json("tools", params).await
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
            descriptor_type
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
            descriptor_type,
            segment("relative path", relative_path)?
        );
        self.get_file(&path, descriptor_type).await
    }

    async fn get_file(
        &self,
        path: &str,
        descriptor_type: DescriptorTypeWithPlain,
    ) -> Result<FileWrapper, Box<dyn Error>> {
        if is_plain(descriptor_type) {
            Ok(plain_file(self.get_text(path, None).await?))
        } else {
            self.get_json(path, None).await
        }
    }

    /// Lists the test parameter files of a tool version
    /// (`GET /tools/{id}/versions/{version_id}/{type}/tests`).
    ///
    /// For `PLAIN_*` types the registry returns the bare test file; it is
    /// returned as a single `FileWrapper` with only `content` set.
    pub async fn list_tests(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorTypeWithPlain,
    ) -> Result<Vec<FileWrapper>, Box<dyn Error>> {
        let path = format!(
            "{}/{}/tests",
            version_path(id, version_id)?,
            descriptor_type
        );
        if is_plain(descriptor_type) {
            Ok(vec![plain_file(self.get_text(&path, None).await?)])
        } else {
            self.get_json(&path, None).await
        }
    }

    /// Lists the files of a tool version with their paths and file types
    /// (`GET /tools/{id}/versions/{version_id}/{type}/files`).
    ///
    /// The `format=zip` variant is not supported.
    pub async fn list_files(
        &self,
        id: &str,
        version_id: &str,
        descriptor_type: DescriptorType,
    ) -> Result<Vec<ToolFile>, Box<dyn Error>> {
        let path = format!(
            "{}/{}/files",
            version_path(id, version_id)?,
            descriptor_type
        );
        self.get_json(&path, None).await
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

fn version_path(id: &str, version_id: &str) -> Result<String, Box<dyn Error>> {
    Ok(format!(
        "tools/{}/versions/{}",
        segment("tool ID", id)?,
        segment("version ID", version_id)?
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::trs::models::tool_file::FileType;
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
            "descriptor_type": ["CWL"]
        })
    }

    fn service_info_json(artifact: &str) -> Value {
        serde_json::json!({
            "id": "org.example.trs",
            "name": "Example TRS",
            "type": { "group": "org.ga4gh", "artifact": artifact, "version": "2.0.1" },
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
                trs.url("tools/a%2Fb").unwrap(),
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
    async fn test_new_accepts_dockstore_service_info() {
        let mut info = service_info_json("TRS");
        info["createdAt"] = serde_json::json!(1593648000000u64);
        info["updatedAt"] = serde_json::json!("2026-11-09T00:00:00Z");
        let m = get_mock("dockstore", "service-info", &info.to_string());
        for client in clients("dockstore") {
            let service = TRS::new(&client.config).await.unwrap().service.unwrap();
            assert_eq!(service.created_at, None);
            assert_eq!(service.updated_at.as_deref(), Some("2026-11-09T00:00:00Z"));
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
            }
        }
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
                    .get_descriptor(TOOL_ID, VERSION_ID, descriptor_type)
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
                    .get_descriptor(TOOL_ID, VERSION_ID, descriptor_type)
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
