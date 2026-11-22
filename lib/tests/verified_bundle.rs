use ga4gh_sdk::{
    clients::{
        bundle::{
            AnonymousFetcher, BundleRequest, DescriptorFetcher, PreparationOptions, PreparedBundle,
        },
        serviceinfo::models::{Service, ServiceType},
        wait::{WaitOptions, WaitOutcome},
        wes::WES,
    },
    utils::{configuration::Configuration, operation::Operation, transport::Transport},
};
use mockito::Mock;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, time::Duration};
const MAIN: &str = include_str!("../../tests/fixtures/trs-cwl/main.cwl");
const TOOL: &str = include_str!("../../tests/fixtures/trs-cwl/tools/echo.cwl");
const INLINE_STEPS: &str =
    "steps:\n  echo:\n    run: tools/echo.cwl\n    in:\n      message: message\n    out: [out]";
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn operation() -> Operation {
    Operation::new(Duration::from_secs(20))
}
fn request() -> BundleRequest {
    BundleRequest {
        tool_id: "workflow".into(),
        version_id: "1".into(),
        workflow_params: json!({"message":"verified hello","secret":"PARAM_SECRET"}),
    }
}
fn source(
    server: &mut mockito::ServerGuard,
    prefix: &str,
    main: &str,
    tool: &str,
    listing: Option<Value>,
    primary: Option<Value>,
) -> (Transport, Vec<Mock>) {
    let path = format!("{prefix}/tools/workflow/versions/1");
    let list=listing.unwrap_or_else(||json!([
  {"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR","checksum":{"type":"sha-256","checksum":hash(main.as_bytes())}},
  {"path":"tools/echo.cwl","file_type":"SECONDARY_DESCRIPTOR","checksum":{"type":"sha-256","checksum":hash(tool.as_bytes())}}
 ]));
    let mocks = vec![
        server
            .mock("GET", path.as_str())
            .with_body(r#"{"id":"1","is_production":false,"meta_version":"rev-A"}"#)
            .create(),
        server
            .mock("GET", format!("{path}/CWL/files").as_str())
            .with_body(list.to_string())
            .create(),
        server
            .mock("GET", format!("{path}/CWL/descriptor").as_str())
            .with_body(
                primary
                    .unwrap_or_else(|| json!({"content":main}))
                    .to_string(),
            )
            .create(),
        server
            .mock(
                "GET",
                format!("{path}/CWL/descriptor/tools%2Fecho.cwl").as_str(),
            )
            .with_body(json!({"content":tool}).to_string())
            .create(),
    ];
    (
        Transport::new(&Configuration::new(
            url::Url::parse(&format!("{}{prefix}", server.url())).unwrap(),
        )),
        mocks,
    )
}
fn wes(base: &str) -> WES {
    let config = Configuration::new(url::Url::parse(base).unwrap());
    WES::without_discovery(
        Transport::new(&config),
        Service {
            r#type: Box::new(ServiceType {
                artifact: "wes".into(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
}

#[tokio::test]
async fn verified_staged_bytes_arrive_at_wes_with_relative_paths_and_safe_provenance() {
    let mut mock_server = mockito::Server::new_async().await;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let (registry, _mocks) = source(&mut mock_server, "/verified", MAIN, TOOL, None, None);
    let prepared = PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(prepared.manifest().is_production, Some(false));
    assert_eq!(prepared.manifest().meta_version.as_deref(), Some("rev-A"));
    let expected = prepared
        .staged_files()
        .map(|(p, b)| (p.to_owned(), b.to_vec()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(expected["main.cwl"], MAIN.as_bytes());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let header_end = loop {
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buffer[..n]);
            if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
        };
        let headers = String::from_utf8_lossy(&data[..header_end]);
        let length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|v| v.trim().parse().unwrap())
            })
            .unwrap();
        let boundary = headers
            .lines()
            .find_map(|line| {
                line.split_once("boundary=")
                    .map(|(_, boundary)| boundary.trim().to_owned())
            })
            .unwrap();
        while data.len() < header_end + length {
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            data.extend_from_slice(&buffer[..n]);
        }
        let body = &data[header_end..header_end + length];
        let marker = format!("\r\n--{boundary}");
        let mut found = BTreeMap::new();
        {
            let part = body; // Scan binary multipart without decoding file payloads.
            let mut cursor = 0;
            while let Some(start) = part[cursor..].windows(10).position(|w| w == b"filename=\"") {
                let name_start = cursor + start + 10;
                let name_end =
                    name_start + part[name_start..].iter().position(|b| *b == b'"').unwrap();
                let filename = String::from_utf8(part[name_start..name_end].to_vec()).unwrap();
                let content_start = name_end
                    + part[name_end..]
                        .windows(4)
                        .position(|w| w == b"\r\n\r\n")
                        .unwrap()
                    + 4;
                let content_end = content_start
                    + part[content_start..]
                        .windows(marker.len())
                        .position(|w| w == marker.as_bytes())
                        .unwrap();
                found.insert(filename, part[content_start..content_end].to_vec());
                cursor = content_end + marker.len();
            }
        }
        let response = r#"{"run_id":"verified-run"}"#;
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{response}",
                    response.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        found
    });
    let submitted = prepared
        .submit(&wes(&format!("http://{address}")), &operation())
        .await
        .unwrap();
    assert_eq!(server.await.unwrap(), expected);
    assert_eq!(submitted.run.id, "verified-run");
    let manifest = serde_json::to_string(&submitted.manifest).unwrap();
    assert!(!manifest.contains("PARAM_SECRET"));
    assert!(manifest.contains("verified-run"));
    assert!(manifest.contains(&hash(MAIN.as_bytes())));
}

#[tokio::test]
async fn unsafe_missing_external_dynamic_and_checksum_failures_submit_nothing() {
    let mut server = mockito::Server::new_async().await;
    let submission = server.mock("POST", "/runs").expect(0).create();
    let cases = vec![
        (
            MAIN.replace("tools/echo.cwl", "tools/missing.cwl"),
            None,
            None,
        ),
        (
            MAIN.replace("tools/echo.cwl", "https://foreign.example/tool.cwl"),
            None,
            None,
        ),
        (
            MAIN.replace("tools/echo.cwl", "'$(inputs.tool)'"),
            None,
            None,
        ),
        (
            MAIN.into(),
            Some(json!([{"path":"../main.cwl","file_type":"PRIMARY_DESCRIPTOR"}])),
            None,
        ),
        (
            MAIN.into(),
            Some(
                json!([{"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR"},{"path":"main.cwl","file_type":"SECONDARY_DESCRIPTOR"}]),
            ),
            None,
        ),
        (
            MAIN.into(),
            None,
            Some(json!({"content":MAIN,"checksum":[{"type":"sha-256","checksum":"0".repeat(64)}]})),
        ),
        (
            MAIN.into(),
            None,
            Some(json!({"content":MAIN,"checksum":[{"type":"unsupported","checksum":"opaque"}]})),
        ),
    ];
    for (index, (main, list, wrapper)) in cases.into_iter().enumerate() {
        let (registry, _mocks) = source(
            &mut server,
            &format!("/bad{index}"),
            &main,
            TOOL,
            list,
            wrapper,
        );
        assert!(
            PreparedBundle::prepare(
                &registry,
                request(),
                &PreparationOptions::default(),
                &operation(),
                None
            )
            .await
            .is_err(),
            "case {index}"
        );
    }
    submission.assert();
}

#[tokio::test]
async fn mutable_version_changes_manifest_while_prepared_bytes_stay_immutable() {
    let mut server = mockito::Server::new_async().await;
    let (registry, mocks) = source(&mut server, "/mutable", MAIN, TOOL, None, None);
    let first = PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .unwrap();
    mocks.iter().for_each(mockito::Mock::remove);
    let changed = format!("{TOOL}\n# changed behind the same version\n");
    let (registry, _mocks) = source(&mut server, "/mutable", MAIN, &changed, None, None);
    let second = PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .unwrap();
    assert_ne!(
        first.manifest().bundle_sha256,
        second.manifest().bundle_sha256
    );
    assert_eq!(
        first
            .staged_files()
            .find(|(path, _)| *path == "tools/echo.cwl")
            .unwrap()
            .1,
        TOOL.as_bytes()
    );
    assert_eq!(
        second
            .staged_files()
            .find(|(path, _)| *path == "tools/echo.cwl")
            .unwrap()
            .1,
        changed.as_bytes()
    );
}

#[tokio::test]
async fn import_include_graph_and_explicit_url_fetch_policy() {
    let mut server = mockito::Server::new_async().await;
    let tool=TOOL.replace("baseCommand: echo","baseCommand: echo\ndoc:\n  $include: ../docs/help.txt\nlabel:\n  $import: ../labels/title.yaml");
    let listing = json!([{"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR"},{"path":"tools/echo.cwl","file_type":"SECONDARY_DESCRIPTOR"},{"path":"docs/help.txt","file_type":"OTHER"},{"path":"labels/title.yaml","file_type":"OTHER"}]);
    let (registry, _mocks) = source(&mut server, "/graph", MAIN, &tool, Some(listing), None);
    let help = server
        .mock(
            "GET",
            "/graph/tools/workflow/versions/1/CWL/descriptor/docs%2Fhelp.txt",
        )
        .with_body(
            json!({"url":format!("{}/content/help?signature=SIGNED_SECRET",server.url())})
                .to_string(),
        )
        .expect(2)
        .create();
    let label = server
        .mock(
            "GET",
            "/graph/tools/workflow/versions/1/CWL/descriptor/labels%2Ftitle.yaml",
        )
        .with_body(r#"{"content":"A title"}"#)
        .create();
    let download = server
        .mock("GET", "/content/help?signature=SIGNED_SECRET")
        .with_body("included help")
        .expect(1)
        .create();
    assert!(PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None
    )
    .await
    .is_err());
    let fetcher = AnonymousFetcher::new([server.url()]).unwrap();
    let prepared = PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        Some(&fetcher),
    )
    .await
    .unwrap();
    assert_eq!(prepared.staged_files().count(), 4);
    assert!(!serde_json::to_string(prepared.manifest())
        .unwrap()
        .contains("SIGNED_SECRET"));
    help.assert();
    label.assert();
    download.assert();
}

#[tokio::test]
async fn imported_workflow_steps_cannot_hide_external_run_dependencies() {
    let mut server = mockito::Server::new_async().await;
    let main = MAIN.replace(INLINE_STEPS, "steps:\n  - $import: steps/echo.yml");
    assert_ne!(main, MAIN);
    let listing = json!([
        {"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR"},
        {"path":"steps/echo.yml","file_type":"OTHER"}
    ]);
    let (registry, _mocks) = source(
        &mut server,
        "/imported-step",
        &main,
        TOOL,
        Some(listing),
        None,
    );
    let step = server.mock(
        "GET",
        "/imported-step/tools/workflow/versions/1/CWL/descriptor/steps%2Fecho.yml",
    )
    .with_body(json!({"content":"id: echo\nrun: https://foreign.example/tool.cwl\nin:\n  message: message\nout: [out]\n"}).to_string())
    .create();
    assert!(PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .is_err());
    step.assert();
}

#[tokio::test]
async fn imported_workflow_step_stages_its_local_run() {
    let mut server = mockito::Server::new_async().await;
    let main = MAIN.replace(INLINE_STEPS, "steps:\n  - $import: steps/echo.yml");
    assert_ne!(main, MAIN);
    let listing = json!([
        {"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR"},
        {"path":"steps/echo.yml","file_type":"SECONDARY_DESCRIPTOR"},
        {"path":"tools/echo.cwl","file_type":"OTHER"}
    ]);
    let (registry, _mocks) = source(&mut server, "/local-step", &main, TOOL, Some(listing), None);
    let step = server.mock(
        "GET",
        "/local-step/tools/workflow/versions/1/CWL/descriptor/steps%2Fecho.yml",
    )
    .with_body(json!({"content":"id: echo\nrun: ../tools/echo.cwl\nin:\n  message: message\nout: [out]\n"}).to_string())
    .create();
    let prepared = PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(prepared.staged_files().count(), 3);
    assert_eq!(
        prepared
            .staged_files()
            .find(|(path, _)| *path == "tools/echo.cwl")
            .unwrap()
            .1,
        TOOL.as_bytes()
    );
    step.assert();
}

#[tokio::test]
async fn imported_steps_collection_cannot_hide_external_runs() {
    let mut server = mockito::Server::new_async().await;
    let main = MAIN.replace(INLINE_STEPS, "steps:\n  $import: steps/all.yml");
    assert_ne!(main, MAIN);
    let listing = json!([
        {"path":"main.cwl","file_type":"PRIMARY_DESCRIPTOR"},
        {"path":"steps/all.yml","file_type":"OTHER"}
    ]);
    let (registry, _mocks) = source(&mut server, "/step-list", &main, TOOL, Some(listing), None);
    let steps = server.mock(
        "GET",
        "/step-list/tools/workflow/versions/1/CWL/descriptor/steps%2Fall.yml",
    )
    .with_body(json!({"content":"echo:\n  run: https://foreign.example/tool.cwl\n  in:\n    message: message\n  out: [out]\n"}).to_string())
    .create();
    assert!(PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .is_err());
    steps.assert();
}

#[test]
fn anonymous_fetch_allowlist_does_not_widen_paths_or_discard_credentials() {
    for input in [
        "https://example.org/restricted",
        "https://example.org/?token=secret",
        "https://user:pass@example.org",
        "file:///tmp/descriptor.cwl",
    ] {
        assert!(
            AnonymousFetcher::new([input.to_owned()]).is_err(),
            "{input}"
        );
    }
    assert!(AnonymousFetcher::new(["https://example.org/".to_owned()]).is_ok());
}

#[tokio::test]
async fn anonymous_fetch_errors_do_not_expose_signed_url_queries() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        drop(socket);
    });
    let fetcher = AnonymousFetcher::new([format!("http://{address}")]).unwrap();
    let url = url::Url::parse(&format!(
        "http://{address}/descriptor?signature=SIGNED_SECRET"
    ))
    .unwrap();
    let error = fetcher.fetch(&url, &operation(), 1024).await.unwrap_err();
    server.await.unwrap();
    assert!(!format!("{error}").contains("SIGNED_SECRET"));
    assert!(!format!("{error:?}").contains("SIGNED_SECRET"));
}

#[tokio::test]
#[ignore = "requires a named running Toil WES deployment via GA4GH_TOIL_WES_URL"]
async fn real_toil_executes_verified_nested_cwl_bundle() {
    let mut server = mockito::Server::new_async().await;
    let url = std::env::var("GA4GH_TOIL_WES_URL").expect("set GA4GH_TOIL_WES_URL");
    let (registry, _mocks) = source(&mut server, "/toil-trs", MAIN, TOOL, None, None);
    let prepared = PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .unwrap();
    let wes = wes(&url);
    let service: Value =
        serde_json::from_str(&wes.transport().get("service-info", None).await.unwrap()).unwrap();
    assert_eq!(service["version"], "9.3.0");
    let submitted = prepared.submit(&wes, &operation()).await.unwrap();
    let mut wait = WaitOptions::new(Duration::from_secs(180));
    wait.fetch_failure_logs = true;
    let result = submitted.run.wait(&wait).await;
    assert_eq!(result.outcome, WaitOutcome::Complete, "{result:?}");
    let log = submitted.run.log().await.unwrap();
    let raw = wes
        .transport()
        .get(&format!("runs/{}", submitted.run.id), None)
        .await
        .unwrap();
    let raw_log: Value = serde_json::from_str(&raw).unwrap();
    let output = raw_log["outputs"]["greeting"]["location"]
        .as_str()
        .expect("CWL File output location");
    let path = url::Url::parse(output).unwrap().to_file_path().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"verified hello\n");
    let evidence = json!({"backend":"Toil WES 9.3.0","run_id":submitted.run.id,"manifest":submitted.manifest,"run_log":raw_log,"output_verified":"verified hello\\n"});
    if let Ok(path) = std::env::var("GA4GH_BUNDLE_EVIDENCE") {
        std::fs::write(path, serde_json::to_vec_pretty(&evidence).unwrap()).unwrap();
    }
    assert!(log.outputs.is_some());
}

#[tokio::test]
async fn unsupported_checksums_are_explicitly_unverified_only_when_opted_in() {
    let mut server = mockito::Server::new_async().await;
    let (registry, _mocks) = source(
        &mut server,
        "/unsupported-opt-in",
        MAIN,
        TOOL,
        None,
        Some(json!({"content":MAIN,"checksum":[{"type":"unsupported","checksum":"opaque"}]})),
    );
    let options = PreparationOptions {
        allow_unsupported_checksums: true,
        ..Default::default()
    };
    let prepared = PreparedBundle::prepare(&registry, request(), &options, &operation(), None)
        .await
        .unwrap();
    let proof = prepared
        .manifest()
        .files
        .iter()
        .find(|f| f.path == "main.cwl")
        .unwrap()
        .advertised
        .iter()
        .find(|c| c.algorithm == "unsupported")
        .unwrap();
    assert!(!proof.verified);
}

/// Serves `main` and `tool` with [`source`] under `prefix`, prepares the
/// bundle with `options` and returns the error message; panics if
/// preparation succeeds.
async fn prepare_error(
    server: &mut mockito::ServerGuard,
    prefix: &str,
    (main, tool): (&str, &str),
    listing: Option<Value>,
    primary: Option<Value>,
    options: &PreparationOptions,
) -> String {
    let (registry, _mocks) = source(server, prefix, main, tool, listing, primary);
    PreparedBundle::prepare(&registry, request(), options, &operation(), None)
        .await
        .err()
        .unwrap()
        .to_string()
}

/// Serves `main` with [`source`] under `prefix` and prepares the bundle
/// with the default options; panics if preparation fails.
async fn prepare_ok(server: &mut mockito::ServerGuard, prefix: &str, main: &str) -> PreparedBundle {
    let (registry, _mocks) = source(server, prefix, main, TOOL, None, None);
    PreparedBundle::prepare(
        &registry,
        request(),
        &PreparationOptions::default(),
        &operation(),
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn malformed_cyclic_and_unsupported_cwl_dependencies_are_rejected() {
    let mut server = mockito::Server::new_async().await;
    let defaults = PreparationOptions::default();
    let run = |reference: &str| MAIN.replace("tools/echo.cwl", reference);
    let cases = [
        (
            run("tools/echo%zz.cwl"),
            TOOL.into(),
            "malformed percent encoding",
        ),
        (
            run("tools/echo%4.cwl"),
            TOOL.into(),
            "malformed percent encoding",
        ),
        (
            run("../../echo.cwl"),
            TOOL.into(),
            "escapes the staged root",
        ),
        (
            MAIN.replace(INLINE_STEPS, "steps:\n  $include: steps.txt"),
            TOOL.into(),
            "unsupported CWL steps declaration",
        ),
        (
            MAIN.replace(INLINE_STEPS, "steps: 5"),
            TOOL.into(),
            "unsupported CWL steps declaration",
        ),
        (
            format!("{MAIN}\ndoc:\n  $import:\n    nested: value\n"),
            TOOL.into(),
            "dynamic CWL dependency",
        ),
        (
            format!("{MAIN}\nlabel: !custom value\n"),
            TOOL.into(),
            "tagged YAML",
        ),
        (
            MAIN.into(),
            format!("{TOOL}\ndoc:\n  $import: ../main.cwl\n"),
            "cyclic CWL descriptor dependency",
        ),
        (TOOL.into(), TOOL.into(), "must be a Workflow"),
    ];
    for (index, (main, tool, message)) in cases.iter().enumerate() {
        let error = prepare_error(
            &mut server,
            &format!("/dependency{index}"),
            (main, tool),
            None,
            None,
            &defaults,
        )
        .await;
        assert!(error.contains(message), "case {index}: {error}");
    }
}

#[tokio::test]
async fn listings_descriptors_and_bounds_are_checked_before_staging() {
    let mut server = mockito::Server::new_async().await;
    let defaults = PreparationOptions::default();
    let secondary = json!([
        {"path":"main.cwl","file_type":"SECONDARY_DESCRIPTOR"},
        {"path":"tools/echo.cwl","file_type":"SECONDARY_DESCRIPTOR"}
    ]);
    let cases = [
        (
            Some(secondary),
            None,
            defaults.clone(),
            "exactly one declared primary descriptor",
        ),
        (
            Some(json!("not a file list")),
            None,
            defaults.clone(),
            "invalid type",
        ),
        (
            None,
            Some(json!({})),
            defaults.clone(),
            "neither content nor URL",
        ),
        (
            None,
            None,
            PreparationOptions {
                max_files: 1,
                ..defaults.clone()
            },
            "exceeds bundle file bound",
        ),
        (
            None,
            None,
            PreparationOptions {
                max_file_bytes: 10,
                ..defaults.clone()
            },
            "exceeds bundle byte bound",
        ),
        // Each file fits the per-file bound; together they exceed the total.
        (
            None,
            None,
            PreparationOptions {
                max_total_bytes: MAIN.len() + TOOL.len() - 1,
                ..defaults.clone()
            },
            "exceeds bundle byte bound",
        ),
    ];
    for (index, (listing, primary, options, message)) in cases.into_iter().enumerate() {
        let error = prepare_error(
            &mut server,
            &format!("/listing{index}"),
            (MAIN, TOOL),
            listing,
            primary,
            &options,
        )
        .await;
        assert!(error.contains(message), "case {index}: {error}");
    }
    let registry = Transport::new(&Configuration::new(url::Url::parse(&server.url()).unwrap()));
    let blank_tool = BundleRequest {
        tool_id: String::new(),
        ..request()
    };
    let no_files = PreparationOptions {
        max_files: 0,
        ..defaults.clone()
    };
    for (request, options, message) in [
        (blank_tool, &defaults, "nonempty opaque segments"),
        (request(), &no_files, "bounds must be positive"),
    ] {
        let error = PreparedBundle::prepare(&registry, request, options, &operation(), None)
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains(message), "{error}");
    }
}

#[tokio::test]
async fn equivalent_relative_paths_stage_the_listed_descriptor() {
    let mut server = mockito::Server::new_async().await;
    for (index, reference) in ["./tools/echo.cwl", "tools//echo.cwl"].iter().enumerate() {
        let main = MAIN.replace("tools/echo.cwl", reference);
        let prepared = prepare_ok(&mut server, &format!("/relative{index}"), &main).await;
        let plan = prepared.request_plan();
        assert_eq!(plan.workflow_url, "main.cwl");
        assert_eq!(plan.workflow_type, "CWL");
        assert_eq!(plan.workflow_type_version, "v1.2");
        assert_eq!(plan.workflow_params, Some(request().workflow_params));
        assert_eq!(
            prepared.staged_files().map(|(p, _)| p).collect::<Vec<_>>(),
            ["main.cwl", "tools/echo.cwl"]
        );
    }
}

#[tokio::test]
async fn several_steps_and_inline_tools_stage_only_listed_references() {
    let mut server = mockito::Server::new_async().await;
    let inline_tool = TOOL.replace('\n', "\n      ");
    let main = MAIN.replace(
        INLINE_STEPS,
        &format!(
            "{INLINE_STEPS}\n  again:\n    run: tools/echo.cwl\n    in:\n      message: message\n    out: [out]\n  inline:\n    run:\n      {inline_tool}\n    in:\n      message: message\n    out: [out]"
        ),
    );
    assert_ne!(main, MAIN);
    let prepared = prepare_ok(&mut server, "/several", &main).await;
    assert_eq!(
        prepared.staged_files().map(|(p, _)| p).collect::<Vec<_>>(),
        ["main.cwl", "tools/echo.cwl"]
    );
}

#[tokio::test]
async fn anonymous_fetch_rejects_error_statuses_and_oversized_bodies() {
    let mut server = mockito::Server::new_async().await;
    let missing = server
        .mock("GET", "/missing")
        .with_status(404)
        .expect(1)
        .create();
    let large = server
        .mock("GET", "/large")
        .with_body("0123456789")
        .expect(1)
        .create();
    let fetcher = AnonymousFetcher::new([server.url()]).unwrap();
    let url = |path: &str| url::Url::parse(&format!("{}{path}", server.url())).unwrap();
    let error = fetcher
        .fetch(&url("/missing"), &operation(), 1024)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("descriptor fetch HTTP 404"),
        "{error}"
    );
    let error = fetcher
        .fetch(&url("/large"), &operation(), 4)
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("response exceeds body limit of 4 bytes"),
        "{error}"
    );
    missing.assert();
    large.assert();
}

#[tokio::test]
async fn anonymous_fetch_truncated_bodies_do_not_expose_signed_url_queries() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\ntruncated")
            .await
            .unwrap();
    });
    let fetcher = AnonymousFetcher::new([format!("http://{address}")]).unwrap();
    let url = url::Url::parse(&format!(
        "http://{address}/descriptor?signature=SIGNED_SECRET"
    ))
    .unwrap();
    let error = tokio::time::timeout(
        Duration::from_secs(10),
        fetcher.fetch(&url, &operation(), 1024),
    )
    .await
    .unwrap()
    .unwrap_err();
    // Bounded, so a fetch that fails before connecting can't leave the test
    // waiting on accept forever.
    tokio::time::timeout(Duration::from_secs(10), server)
        .await
        .expect("the server answers the fetch")
        .unwrap();
    assert!(matches!(error, ga4gh_sdk::Error::Transport(_)), "{error:?}");
    assert!(!format!("{error}").contains("SIGNED_SECRET"));
    assert!(!format!("{error:?}").contains("SIGNED_SECRET"));
}
