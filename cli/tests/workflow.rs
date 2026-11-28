use ga4gh_cli::{CliContext, CommandOutput};
use ga4gh_sdk::clients::ServiceType;
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::extension_manager::ExtensionManager;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};
use url::Url;
fn manifest() -> PathBuf {
    std::env::var_os("GA4GH_WORKFLOW_MANIFEST")
        .expect("run make workflow-extension")
        .into()
}

async fn install(context: &CliContext) {
    let mut manager = ExtensionManager::open(
        context
            .config_path()
            .unwrap()
            .with_file_name("extensions.json"),
    )
    .unwrap();
    manager.add_extension(manifest()).await.unwrap();
    manager
        .enable_extension("ga4gh-workflow-extension")
        .unwrap();
}

fn install_binary(home: &Path) {
    for args in [
        vec!["extension", "add", manifest().to_str().unwrap()],
        vec!["extension", "enable", "ga4gh-workflow-extension"],
    ] {
        let output = run(&args, home);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

fn run(args: &[&str], home: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_ga4gh-cli"))
        .args(args)
        .env("HOME", home)
        .output()
        .unwrap()
}
#[test]
fn real_cli_preparation_rejects_bad_params_without_configuration_and_emits_reviewable_json() {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let invalid = root.join("examples/sequence-qc/invalid-request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let args = |file: &Path| {
        vec![
            "workflow".to_owned(),
            "prepare".into(),
            schema.to_str().unwrap().into(),
            file.to_str().unwrap().into(),
            "public-qc@1".into(),
            "--profile-revision".into(),
            pin.into(),
            "--vocabulary-revision".into(),
            pin.into(),
        ]
    };
    install_binary(temp.path());
    let values = args(&invalid);
    let args = values.iter().map(String::as_str).collect::<Vec<_>>();
    let output = run(&args, temp.path());
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("/sequences/0"));
    assert!(stderr.contains("pattern"));
    let values = vec![
        "workflow",
        "prepare",
        schema.to_str().unwrap(),
        request.to_str().unwrap(),
        "public-qc@1",
        "--profile-revision",
        pin,
        "--vocabulary-revision",
        pin,
    ];
    let output = run(&values, temp.path());
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let prepared: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        prepared["request"]["workflow_params"]["sequences"],
        json!(["ACGT", "AC", "ACGTAC"])
    );
    assert!(prepared["fingerprint"].as_str().unwrap().len() == 64);
    assert_eq!(prepared["pins"]["profile_revision"], pin);
}
#[test]
fn cli_output_projection_keeps_complete_with_missing_delivery() {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let log = temp.path().join("log.json");
    std::fs::write(&log,json!({"state":"COMPLETE","outputs":{"metrics":{"sequence_count":3,"total_length":12,"mean_length":4.0}}}).to_string()).unwrap();
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    install_binary(temp.path());
    let output = run(
        &[
            "workflow",
            "results",
            schema.to_str().unwrap(),
            log.to_str().unwrap(),
            "wes/run1",
            "--profile-revision",
            pin,
            "--vocabulary-revision",
            pin,
        ],
        temp.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let record: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(record["execution_state"], "COMPLETE");
    assert_eq!(record["outputs"]["report"]["contract_status"], "missing");
}

#[test]
fn explicit_cli_submission_uploads_the_reviewed_request_once() {
    let mut server = mockito::Server::new();
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join(".ga4gh-cli");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.json"),
        json!({"WES":{"base_path":format!("{}/cli-wes",server.url()),"oauth_access_token":"cli-fixture-token"}}).to_string(),
    )
    .unwrap();
    let info = json!({"workflow_type_versions":{"CWL":{"workflow_type_version":["v1.2"]}},"supported_wes_versions":["1.0.0"],"supported_filesystem_protocols":["file"],"workflow_engine_versions":{},"default_workflow_engine_parameters":[],"system_state_counts":{},"tags":{}});
    let service = server
        .mock("GET", "/cli-wes/service-info")
        .match_header("authorization", "Bearer cli-fixture-token")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = server
        .mock("POST", "/cli-wes/runs")
        .match_header("authorization", "Bearer cli-fixture-token")
        .match_body(mockito::Matcher::Regex("Sequence QC".into()))
        .with_body(r#"{"run_id":"cli-qc-run"}"#)
        .expect(1)
        .create();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let descriptor = root.join("examples/sequence-qc/main.cwl");
    let attachment = format!("main.cwl={}", descriptor.display());
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    install_binary(temp.path());
    let output = run(
        &[
            "workflow",
            "submit",
            schema.to_str().unwrap(),
            request.to_str().unwrap(),
            "public-qc@1",
            "--profile-revision",
            pin,
            "--vocabulary-revision",
            pin,
            "--attachment",
            &attachment,
        ],
        temp.path(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["run_id"], "cli-qc-run");
    assert_eq!(
        result["invocation"]["request"]["workflow_params"]["sequences"][0],
        "ACGT"
    );
    service.assert();
    submission.assert();
}

#[tokio::test]
async fn downstream_workflow_submission_uses_supplied_config() {
    let mut server = mockito::Server::new_async().await;
    let info = json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let service = server
        .mock("GET", "/injected-wes/service-info")
        .match_header("authorization", "Bearer injected-token")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = server
        .mock("POST", "/injected-wes/runs")
        .match_header("authorization", "Bearer injected-token")
        .with_body(r#"{"run_id":"injected-run"}"#)
        .expect(1)
        .create();
    let config = Configuration::new(Url::parse(&format!("{}/injected-wes", server.url())).unwrap())
        .with_oauth_access_token("injected-token".into());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let temp = tempfile::tempdir().unwrap();
    let context = CliContext::default().with_config_path(temp.path().join("missing.json"));
    install(&context).await;
    let matches = context
        .parse_from(
            context.command(),
            [
                "ga4gh-cli",
                "workflow",
                "submit",
                schema.to_str().unwrap(),
                request.to_str().unwrap(),
                "public-qc@1",
                "--profile-revision",
                pin,
                "--vocabulary-revision",
                pin,
            ],
        )
        .unwrap();
    let output = matches
        .dispatch_extension(Some(&config))
        .await
        .unwrap()
        .expect("installed workflow provider");
    assert!(output.stderr.is_empty(), "{}", output.stderr);
    let response: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(response["run_id"], "injected-run");
    service.assert();
    submission.assert();
}

#[tokio::test]
async fn workflow_submit_reports_failed_oidc_refresh_before_sending() {
    let mut server = mockito::Server::new_async().await;
    let temp = tempfile::tempdir().unwrap();
    let issuer = format!("{}/workflow-idp", server.url());
    let wes_root = format!("{}/workflow-wes", server.url());
    let config_path = temp.path().join("config.json");
    std::fs::write(
        &config_path,
        json!({"WES": {
            "base_path": wes_root,
            "oidc": {"issuer": issuer, "client_id": "workflow-cli"}
        }})
        .to_string(),
    )
    .unwrap();
    let token_store = temp.path().join("tokens.json");
    std::fs::write(
        &token_store,
        json!({"logins": [{
            "issuer": issuer,
            "client_id": "workflow-cli",
            "scope": "openid offline_access",
            "token_endpoint": format!("{issuer}/token"),
            "access_token": "expired-access",
            "refresh_token": "expired-refresh",
            "expires_at": 0
        }]})
        .to_string(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&token_store, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let refresh = server
        .mock("POST", "/workflow-idp/token")
        .match_body(mockito::Matcher::UrlEncoded(
            "refresh_token".into(),
            "expired-refresh".into(),
        ))
        .with_status(400)
        .with_body(r#"{"error":"invalid_grant"}"#)
        .expect(1)
        .create();
    let service = server
        .mock("GET", "/workflow-wes/service-info")
        .expect(0)
        .create();
    let submission = server.mock("POST", "/workflow-wes/runs").expect(0).create();

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    let context = CliContext::new("test-cli", "1.0.0").with_config_path(config_path);
    install(&context).await;
    let matches = context
        .parse_from(
            context.command(),
            [
                "test-cli",
                "workflow",
                "submit",
                schema.to_str().unwrap(),
                request.to_str().unwrap(),
                "public-qc@1",
                "--profile-revision",
                pin,
                "--vocabulary-revision",
                pin,
            ],
        )
        .unwrap();
    let output = matches.dispatch_extension(None).await.unwrap().unwrap();
    assert_eq!(output.exit_code, std::process::ExitCode::FAILURE);
    assert!(
        output.stderr.contains("run 'test-cli login WES' again"),
        "{}",
        output.stderr
    );
    assert!(!output.stderr.contains("expired-refresh"));
    refresh.assert();
    service.assert();
    submission.assert();
}

/// A pin that the sequence QC profile accepts for both revisions.
const PIN: &str = "45d41324353cb4900fdd999c307c2e36a3f2a410";

fn assert_failure(output: &CommandOutput, expected: &str) {
    assert_eq!(output.exit_code, ExitCode::FAILURE);
    assert!(output.stdout.is_empty(), "{}", output.stdout);
    assert!(output.stderr.contains(expected), "{}", output.stderr);
}

/// The path of `file` in the sequence QC example.
fn example(file: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("examples/sequence-qc")
        .join(file)
}

/// Runs the `workflow` command `args`, without a WES configuration.
async fn run_workflow(context: &CliContext, args: &[&str]) -> CommandOutput {
    let args = ["cli", "workflow"].into_iter().chain(args.iter().copied());
    context
        .parse_from(context.command(), args)
        .unwrap()
        .dispatch_extension(None)
        .await
        .unwrap()
        .unwrap()
}

/// Writes `value` as JSON to `name` in `directory` and returns the path.
fn write_json(directory: &Path, name: &str, value: &Value) -> String {
    let path = directory.join(name);
    std::fs::write(&path, value.to_string()).unwrap();
    path.to_str().unwrap().into()
}

/// The arguments that pin the revisions of the sequence QC profile.
const PINS: [&str; 4] = ["--profile-revision", PIN, "--vocabulary-revision", PIN];

#[tokio::test]
async fn workflow_verify_and_compare_read_result_records() {
    let directory = tempfile::tempdir().unwrap();
    let context = CliContext::default().with_config_path(directory.path().join("missing.json"));
    install(&context).await;
    let schema = example("profile.json");
    let log = write_json(
        directory.path(),
        "log.json",
        &json!({"state": "COMPLETE", "outputs": {"metrics": {
            "sequence_count": 3, "total_length": 12, "mean_length": 4.0
        }}}),
    );
    let mut args = vec!["results", schema.to_str().unwrap(), &log, "wes/run1"];
    args.extend(PINS);
    let output = run_workflow(&context, &args).await;
    assert_eq!(output.exit_code, ExitCode::SUCCESS, "{}", output.stderr);
    let record = directory.path().join("record.json");
    std::fs::write(&record, &output.stdout).unwrap();
    let record = record.to_str().unwrap();

    let bytes = directory.path().join("metrics.json");
    std::fs::write(&bytes, b"{}").unwrap();
    let bytes = bytes.to_str().unwrap();
    // SHA-256 of "{}"
    let digest = "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a";
    let output = run_workflow(
        &context,
        &[
            "verify",
            record,
            "/metrics",
            bytes,
            "--sha256",
            digest,
            "--evidence-source",
            "manifest@1",
        ],
    )
    .await;
    assert_eq!(output.exit_code, ExitCode::SUCCESS, "{}", output.stderr);
    let verified: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(
        verified["artifacts"]["/metrics"]["integrity_status"],
        "verified"
    );
    let output = run_workflow(
        &context,
        &[
            "verify",
            record,
            "/metrics",
            bytes,
            "--sha256",
            "abc",
            "--evidence-source",
            "manifest@1",
        ],
    )
    .await;
    assert_failure(&output, "needs a valid digest");

    let policy = example("policy.json");
    let policy = policy.to_str().unwrap();
    let output = run_workflow(&context, &["compare", policy, record, record]).await;
    assert_eq!(output.exit_code, ExitCode::SUCCESS, "{}", output.stderr);
    let report: Value = serde_json::from_str(&output.stdout).unwrap();
    assert_eq!(report["policy"]["id"], "sequence-qc/1");
    assert_eq!(report["left"], "wes/run1");
    assert_eq!(report["right"], "wes/run1");
    // A record compared with itself matches on every output, but the
    // record lacks the workflow and input digests the policy requires.
    assert_eq!(
        report["outputs"],
        json!({
            "/metrics/mean_length": "equivalent_under_policy",
            "/metrics/sequence_count": "equivalent_under_policy",
            "/metrics/total_length": "equivalent_under_policy",
        })
    );
    assert_eq!(
        report["missing_context"],
        json!(["workflow_digest", "input_digest"])
    );
    assert_eq!(report["outcome"], "inconclusive");
    let unsupported = write_json(
        directory.path(),
        "policy.json",
        &json!({"id": "p", "version": 2, "outputs": {}, "required_context": []}),
    );
    let output = run_workflow(&context, &["compare", &unsupported, record, record]).await;
    assert_failure(&output, "version 1");
}

#[tokio::test]
async fn workflow_commands_report_invalid_inputs() {
    let directory = tempfile::tempdir().unwrap();
    let context = CliContext::default().with_config_path(directory.path().join("missing.json"));
    install(&context).await;
    let request = example("request.json");
    let request = request.to_str().unwrap();
    let invalid = directory.path().join("invalid.json");
    std::fs::write(&invalid, "not json").unwrap();
    let invalid = invalid.to_str().unwrap();
    let mut args = vec!["prepare", invalid, request, "public-qc@1"];
    args.extend(PINS);
    assert_failure(&run_workflow(&context, &args).await, "expected ident");

    let log = write_json(
        directory.path(),
        "log.json",
        &json!({"state": "COMPLETE", "outputs": {}}),
    );
    let mut args = vec!["results", invalid, &log, "wes/run1"];
    args.extend(PINS);
    assert_failure(&run_workflow(&context, &args).await, "expected ident");
    // A profile without an output contract can't project results.
    let mut schema: Value =
        serde_json::from_str(&std::fs::read_to_string(example("profile.json")).unwrap()).unwrap();
    schema["properties"]
        .as_object_mut()
        .unwrap()
        .remove("outputs");
    let schema = write_json(directory.path(), "profile.json", &schema);
    let mut args = vec!["results", &schema, &log, "wes/run1"];
    args.extend(PINS);
    assert_failure(&run_workflow(&context, &args).await, "output contract");

    let large = directory.path().join("large.json");
    std::fs::write(&large, vec![b' '; 8 * 1024 * 1024 + 1]).unwrap();
    let large = large.to_str().unwrap();
    let output = run_workflow(&context, &["verify", large, "/metrics", large]).await;
    assert_failure(&output, "JSON document exceeds 8 MiB");
}

#[tokio::test]
async fn workflow_submit_reports_unreadable_attachments() {
    let mut server = mockito::Server::new_async().await;
    let info = server
        .mock("GET", "/service-info")
        .expect(0)
        .create_async()
        .await;
    // The attachment is read before the run is submitted.
    let submission = server.mock("POST", "/runs").expect(0).create_async().await;
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("main.cwl");
    let attachment = format!("main.cwl={}", missing.display());
    let schema = example("profile.json");
    let request = example("request.json");
    let mut args = vec![
        "cli",
        "workflow",
        "submit",
        schema.to_str().unwrap(),
        request.to_str().unwrap(),
        "public-qc@1",
        "--attachment",
        &attachment,
    ];
    args.extend(PINS);
    let context = CliContext::default().with_config_path(directory.path().join("config.json"));
    install(&context).await;
    let config = Configuration::new(Url::parse(&server.url()).unwrap());
    let output = context
        .parse_from(context.command(), args)
        .unwrap()
        .dispatch_extension(Some(&config))
        .await
        .unwrap()
        .unwrap();
    info.assert_async().await;
    // ENOENT on Unix and ERROR_FILE_NOT_FOUND on Windows are both code 2.
    assert_failure(&output, "(os error 2)");
    submission.assert_async().await;
}

#[tokio::test]
async fn only_validated_bytes_reach_the_injected_client_even_when_the_acknowledgement_is_ambiguous()
{
    use ga4gh_sdk::{
        clients::wes::WES,
        utils::{
            auth::{AccessToken, BoxError, TokenProvider, TokenRequest},
            transport::Transport,
        },
    };
    use sha2::{Digest, Sha256};

    struct ReplaceFile(PathBuf);
    #[async_trait::async_trait]
    impl TokenProvider for ReplaceFile {
        async fn token(&self, _: &TokenRequest<'_>) -> Result<AccessToken, BoxError> {
            std::fs::write(&self.0, "changed after preparation")?;
            Ok(AccessToken::new("provider-token"))
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let context = CliContext::default().with_config_path(dir.path().join("missing.json"));
    install(&context).await;
    let mut server = mockito::Server::new_async().await;
    let submission = server
        .mock("POST", "/wes/runs")
        .match_header("authorization", "Bearer provider-token")
        .match_header("x-caller", "custom-client")
        .match_body(mockito::Matcher::Regex("original verified bytes".into()))
        .with_body("{}")
        .expect(1)
        .create();
    let file = dir.path().join("main.cwl");
    std::fs::write(&file, "original verified bytes").unwrap();
    let config = Configuration::new(Url::parse(&format!("{}/wes", server.url())).unwrap())
        .with_token_provider(ReplaceFile(file.clone()));
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("x-caller", "custom-client".parse().unwrap());
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .default_headers(headers)
        .build()
        .unwrap();
    let wes = WES::without_discovery(
        Transport::with_client_and_configured_auth(&config, client),
        Default::default(),
    );
    let schema = example("profile.json");
    let request = example("request.json");
    let spec = format!("main.cwl={}", file.display());
    let parse = |digest: &str| {
        context
            .parse_from(
                context.command(),
                [
                    "cli",
                    "workflow",
                    "submit",
                    schema.to_str().unwrap(),
                    request.to_str().unwrap(),
                    "qc",
                    "--attachment",
                    &spec,
                    "--descriptor-digest",
                    digest,
                ]
                .into_iter()
                .chain(PINS),
            )
            .unwrap()
    };
    let output = parse("sha256:wrong")
        .dispatch_extension_with_wes(&wes)
        .await
        .unwrap()
        .unwrap();
    assert_failure(&output, "primary descriptor digest mismatch");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "original verified bytes"
    );
    let digest = Sha256::digest(b"original verified bytes")
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let output = parse(&format!("sha256:{digest}"))
        .dispatch_extension_with_wes(&wes)
        .await
        .unwrap()
        .unwrap();
    assert_failure(&output, "Reconcile before retrying");
    assert_eq!(
        std::fs::read_to_string(file).unwrap(),
        "changed after preparation"
    );
    submission.assert();
}

/// `workflow submit` is the one command that sends a configured token, so it
/// shows that the token variable reaches the service.
#[tokio::test]
async fn workflow_submission_uses_the_context_file_and_token_variable() {
    let mut server = mockito::Server::new_async().await;
    const VAR: &str = "GA4GH_CLI_TEST_CONTEXT_TOKEN_WORKFLOW";
    let info = json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let service = server
        .mock("GET", "/context-wes/service-info")
        .match_header("authorization", "Bearer token-from-variable")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = server
        .mock("POST", "/context-wes/runs")
        .match_header("authorization", "Bearer token-from-variable")
        .with_body(r#"{"run_id":"context-run"}"#)
        .expect(1)
        .create();
    let dir = tempfile::tempdir().unwrap();
    let base_path = format!("{}/context-wes", server.url());
    let path = write_config(dir.path(), json!({"WES": {"base_path": base_path}}));
    std::env::set_var(VAR, "token-from-variable");
    let context = CliContext::new("other-cli", "1")
        .with_config_path(&path)
        .with_token_env(VAR, [ServiceType::WES]);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    install(&context).await;
    let matches = context
        .parse_from(
            context.command(),
            [
                "other-cli",
                "workflow",
                "submit",
                schema.to_str().unwrap(),
                request.to_str().unwrap(),
                "public-qc@1",
                "--profile-revision",
                pin,
                "--vocabulary-revision",
                pin,
            ],
        )
        .unwrap();

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = matches.dispatch_to(&mut stdout, &mut stderr).await.unwrap();
    assert_eq!(
        exit_code,
        Some(ExitCode::SUCCESS),
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(response["run_id"], "context-run");
    service.assert();
    submission.assert();
}

/// `workflow submit` sends a configured token, so it shows that a profile's
/// section, with its token in a named variable, reaches the service.
#[tokio::test]
async fn workflow_submission_uses_the_profile_and_its_token_variable() {
    let mut server = mockito::Server::new_async().await;
    const TOKEN: &str = "GA4GH_CLI_TEST_PROFILE_WORKFLOW_TOKEN";
    let info = json!({
        "workflow_type_versions": {"CWL": {"workflow_type_version": ["v1.2"]}},
        "supported_wes_versions": ["1.0.0"],
        "supported_filesystem_protocols": ["file"],
        "workflow_engine_versions": {},
        "default_workflow_engine_parameters": [],
        "system_state_counts": {},
        "tags": {}
    });
    let service = server
        .mock("GET", "/profile-wes/service-info")
        .match_header("authorization", "Bearer token-of-the-profile")
        .with_body(info.to_string())
        .expect(1)
        .create();
    let submission = server
        .mock("POST", "/profile-wes/runs")
        .match_header("authorization", "Bearer token-of-the-profile")
        .with_body(r#"{"run_id":"profile-run"}"#)
        .expect(1)
        .create();
    let dir = tempfile::tempdir().unwrap();
    let base_path = format!("{}/profile-wes", server.url());
    let path = write_config(
        dir.path(),
        json!({
            "profiles": {
                "default": {"WES": {"base_path": "http://localhost:1/unused"}},
                "remote": {"WES": {"base_path": base_path, "oauth_access_token_env": TOKEN}}
            }
        }),
    );
    std::env::set_var(TOKEN, "token-of-the-profile");
    let context = CliContext::new("other-cli", "1")
        .with_config_path(&path)
        .with_profile_env("GA4GH_CLI_TEST_PROFILE_WORKFLOW");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let schema = root.join("examples/sequence-qc/profile.json");
    let request = root.join("examples/sequence-qc/request.json");
    let pin = "45d41324353cb4900fdd999c307c2e36a3f2a410";
    install(&context).await;
    let matches = context
        .parse_from(
            context.command(),
            [
                "other-cli",
                "--profile",
                "remote",
                "workflow",
                "submit",
                schema.to_str().unwrap(),
                request.to_str().unwrap(),
                "public-qc@1",
                "--profile-revision",
                pin,
                "--vocabulary-revision",
                pin,
            ],
        )
        .unwrap();

    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let exit_code = matches.dispatch_to(&mut stdout, &mut stderr).await.unwrap();
    assert_eq!(
        exit_code,
        Some(ExitCode::SUCCESS),
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&stdout).unwrap();
    assert_eq!(response["run_id"], "profile-run");
    service.assert();
    submission.assert();
}

fn write_config(dir: &Path, document: serde_json::Value) -> PathBuf {
    let path = dir.join("config.json");
    std::fs::write(&path, document.to_string()).unwrap();
    path
}
