//! Opt-in native WES profile. Configuration comes from SEQERA_PROFILE_* env vars.
//! Emits sanitized evidence only; the private ledger retains the accepted run ID.
use ga4gh_sdk::clients::wes::{models::RunRequest, models::State, Run, WorkflowAttachment, WES};
use ga4gh_sdk::utils::{configuration::Configuration, transport::Transport};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde_json::{json, Value};
use sha2::Digest;
use std::{collections::HashMap, env, path::Path, time::Duration};

type Error = Box<dyn std::error::Error>;
fn required(key: &str) -> Result<String, Error> {
    env::var(key).map_err(|_| format!("missing environment setting {key}").into())
}
fn check(id: &str, outcome: &str, detail: &str) -> Value {
    json!({"id":id, "outcome":outcome, "detail":detail})
}
fn terminal(state: State) -> bool {
    matches!(
        state,
        State::Complete
            | State::Canceled
            | State::ExecutorError
            | State::SystemError
            | State::Preempted
    )
}
fn ledger(path: &Path, run: &Run, state: &str) -> Result<(), Error> {
    // Never overwrite a previous unresolved submission ledger.
    let value = json!({"schema_version":1,"run_id":run.id,"state":state});
    std::fs::write(path, serde_json::to_vec(&value)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
async fn profile(checks: &mut Vec<Value>) -> Result<(), Error> {
    let config = Configuration::new(url::Url::parse(&required("SEQERA_PROFILE_API_ROOT")?)?);
    if config.base_path.username() != ""
        || config.base_path.password().is_some()
        || config.base_path.query().is_some()
        || config.base_path.fragment().is_some()
    {
        return Err("API root must not contain credentials, query or fragment".into());
    }
    let token = std::fs::read_to_string(required("SEQERA_PROFILE_TOKEN_FILE")?)?;
    let mut auth = HeaderValue::from_str(&format!("Bearer {}", token.trim()))?;
    auth.set_sensitive(true);
    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, auth);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    // Configuration bearer fields are not applied by Transport (#31). Inject one
    // authenticated reqwest client into the existing SDK transport instead.
    let transport = Transport::with_client(&config, client);
    let info: Value = serde_json::from_str(&transport.get("service-info", None).await?)?;
    let versions = info["supported_wes_versions"]
        .as_array()
        .ok_or("native WES discovery is missing versions")?;
    if !versions
        .iter()
        .any(|x| x == "1.0" || x == "1.0.0" || x == "1.0.1")
    {
        checks.push(check(
            "discovery",
            "version_mismatch",
            "Expected a native WES 1.0 service",
        ));
        return Ok(());
    }
    checks.push(json!({"id":"discovery", "outcome":"passed", "detail":"Authenticated native WES service-info", "observed":{"supported_wes_versions":versions,"workflow_type_versions":info["workflow_type_versions"],"workflow_engine_versions":info["workflow_engine_versions"]}}));
    if env::var("SEQERA_PROFILE_ACTION").as_deref() != Ok("launch-cancel") {
        checks.push(check(
            "launch_status_logs_cancel",
            "not_run",
            "Probe mode performs no launch",
        ));
        return Ok(());
    }
    if env::var("SEQERA_PROFILE_ALLOW_LAUNCH").as_deref() != Ok("one-bounded-run") {
        return Err("launch requires explicit one-bounded-run opt-in".into());
    }
    if !info["workflow_type_versions"]["NFL"].is_object() {
        checks.push(check(
            "descriptor_type",
            "unsupported",
            "Native discovery does not advertise NFL",
        ));
        return Ok(());
    }
    let descriptor = required("SEQERA_PROFILE_DESCRIPTOR_VERSION")?;
    let advertised = info["workflow_type_versions"]["NFL"]["workflow_type_version"].as_array();
    if !advertised.is_some_and(|v| v.iter().any(|x| x == &descriptor)) {
        checks.push(check("descriptor_version_advertised", "unsupported", "Descriptor version must be an operator override; native discovery does not advertise it"));
    }
    let public_hello = env::var("SEQERA_PROFILE_WORKFLOW_MODE").as_deref() == Ok("public-hello");
    let mut request = RunRequest::new(
        "NFL".into(),
        descriptor,
        if public_hello {
            "https://github.com/nextflow-io/hello"
        } else {
            "seqera.nf"
        }
        .into(),
    );
    request.workflow_params = Some(if public_hello {
        json!({})
    } else {
        json!({"delay":30})
    });
    request.workflow_engine_parameters = Some(HashMap::from([
        (
            "workspaceId".into(),
            required("SEQERA_PROFILE_WORKSPACE_ID")?,
        ),
        (
            "computeEnvId".into(),
            required("SEQERA_PROFILE_COMPUTE_ENV_ID")?,
        ),
    ]));
    let attachments = if public_hello {
        checks.push(check("workflow_revision_pin", "unsupported", "Public hello Git repository URL is selected; WES does not document a revision override, so observed revision must be verified separately"));
        checks.push(check("provider_resource_limit", "unsupported", "No documented native WES resource override used; operator-selected demonstration environment supplies resource policy"));
        Vec::new()
    } else {
        let content = std::fs::read(required("SEQERA_PROFILE_WORKFLOW")?)?;
        if format!("{:x}", sha2::Sha256::digest(&content))
            != "86ed3a8dd1ebfb1132468141b9f89f52e63b2dcfed6ce148657f2f86356a5db9"
        {
            return Err("workflow must match the fixed bounded public profile fixture".into());
        }
        vec![WorkflowAttachment {
            filename: "seqera.nf".into(),
            content,
        }]
    };
    // Native WES discovery is checked above. The public WES struct permits
    // supplying this transport without an unauthenticated constructor request.
    let mut service = ga4gh_sdk::clients::serviceinfo::models::Service::default();
    // This is local SDK discovery state, not a fabricated provider/image ID.
    // Only the checked protocol kind/version is projected from native WES info.
    service.r#type.artifact = "wes".into();
    service.r#type.group = "org.ga4gh".into();
    service.r#type.version = versions[0].as_str().unwrap_or("1.0").into();
    let wes = WES {
        config,
        transport,
        service: Ok(service),
    };
    let ledger_path = required("SEQERA_PROFILE_LEDGER")?;
    let ledger_path = Path::new(&ledger_path);
    // create_new prevents an accidental second launch during recovery.
    let intent = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(ledger_path)?;
    drop(intent);
    std::fs::write(
        ledger_path,
        b"{\"schema_version\":1,\"state\":\"SUBMISSION_OUTCOME_UNKNOWN\"}",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(ledger_path, std::fs::Permissions::from_mode(0o600))?;
    }

    let run = match wes
        .run_workflow_with_attachments(request, attachments)
        .await
    {
        Ok(run) => run,
        Err(error) => {
            let http_status = error
                .downcast_ref::<ga4gh_sdk::utils::transport::HttpStatusError>()
                .map(|error| error.status.as_u16());
            checks.push(json!({"id":"launch", "outcome":"observed_failure", "detail":"Submission not acknowledged; ledger remains SUBMISSION_OUTCOME_UNKNOWN; never retry automatically", "observed":{"http_status":http_status}}));
            return Ok(());
        }
    };
    // Retain ownership even if the first ledger update fails: cancellation is
    // attempted with the acknowledged ID before returning the I/O error.
    if let Err(error) = ledger(ledger_path, &run, "ACKNOWLEDGED") {
        let _ = run.cancel().await;
        return Err(error);
    }
    checks.push(check(
        "multipart_launch",
        "passed",
        "One SDK multipart request acknowledged with explicit workspace and compute environment",
    ));
    let budget = env::var("SEQERA_PROFILE_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(60)
        .clamp(1, 60);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(budget);
    let mut observed = None;
    while tokio::time::Instant::now() < deadline {
        match run.status().await {
            Ok(state) => {
                observed = Some(state);
                if state == State::Running || terminal(state) {
                    break;
                }
            }
            Err(_) => break,
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    checks.push(json!({"id":"status","outcome":if observed.is_some(){"passed"}else{"observed_failure"},"detail":"SDK status observation", "observed":{"state":observed.map(|s|s.to_string())}}));
    checks.push(json!({"id":"execution_observed", "outcome":if matches!(observed, Some(State::Running | State::Complete)) {"passed"} else {"not_run"}, "detail":"Native WES execution state is separate from acceptance/status/cancellation; this is not physical worker proof", "observed":{"state":observed.map(|s|s.to_string())}}));
    match run.log().await {
        Ok(log) => {
            let identity_matches = log.run_id.as_deref() == Some(run.id.as_str());
            let has_logs = log
                .run_log
                .as_deref()
                .is_some_and(|l| l.stdout.is_some() || l.stderr.is_some())
                || log.task_logs.as_ref().is_some_and(|logs| !logs.is_empty());
            checks.push(json!({"id":"run_details","outcome":if identity_matches {"passed"} else {"observed_failure"},"detail":"SDK decoded native run details with matching run identity", "observed":{"identity_matches":identity_matches}}));
            checks.push(json!({"id":"logs","outcome":if has_logs && identity_matches {"passed"} else {"unsupported"},"detail":"Native log content/reference availability; private text omitted and external signed log URLs not fetched", "observed":{"run_log_present":log.run_log.is_some(),"task_logs_present":log.task_logs.is_some()}}));
        }
        Err(_) => checks.push(check(
            "logs",
            "observed_failure",
            "SDK could not retrieve/decode native run details",
        )),
    }
    let cancel = run.cancel().await;
    checks.push(check(
        "cancel_request",
        if cancel.is_ok() {
            "passed"
        } else {
            "observed_failure"
        },
        "SDK cancellation acknowledgement",
    ));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(budget);
    let mut final_state = None;
    while tokio::time::Instant::now() < deadline {
        match run.status().await {
            Ok(state) => {
                final_state = Some(state);
                if terminal(state) {
                    break;
                }
            }
            Err(_) => break,
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    ledger(
        ledger_path,
        &run,
        &final_state
            .map(|s| s.to_string())
            .unwrap_or("OBSERVATION_UNKNOWN".into()),
    )?;
    checks.push(json!({"id":"cancel_terminal", "outcome":if final_state == Some(State::Canceled){"passed"}else{"observed_failure"},"detail":"CANCELED required; completion/cancellation race remains distinct", "observed":{"state":final_state.map(|s|s.to_string())}}));
    checks.push(check(
        "physical_worker_cleanup",
        "unsupported",
        "WES protocol does not expose worker termination; operator observation required",
    ));
    Ok(())
}
#[tokio::main]
async fn main() {
    let mut checks = Vec::new();
    if profile(&mut checks).await.is_err() {
        // Error bodies can contain private workspace/run IDs or signed URLs.
        checks.push(check(
            "harness",
            "harness_error",
            "Profile setup, I/O or transport failed; inspect private operator context",
        ));
    }
    println!("{}", json!({"schema_version":1,"checks":checks}));
}
