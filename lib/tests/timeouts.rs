//! Configurable connection, request, discovery and submission timeouts (#173).
use ga4gh_sdk::{
    clients::{
        drs::DRS,
        tes::{models::TesTask, TES},
        trs::TRS,
        wes::{models::RunRequest, WES},
    },
    utils::{
        configuration::Configuration,
        operation::{AmbiguousSubmissionError, Operation},
        timeouts::Timeouts,
        transport::Transport,
    },
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

const SHORT: Duration = Duration::from_millis(150);
const DELAY: Duration = Duration::from_millis(600);
const LONG: Duration = Duration::from_secs(5);

/// How the test server answers one request.
#[derive(Clone, Copy)]
enum Reply {
    /// Answers with this body after a delay.
    After(Duration, &'static str),
    /// Answers with the service info of this artifact, such as `tes`, after a delay.
    ServiceInfo(Duration, &'static str),
    /// Reads the request, then keeps the connection open without answering.
    Never,
}

/// A loopback HTTP server that answers each request by its path, after a delay.
struct Server {
    url: String,
    requests: Arc<AtomicUsize>,
}

fn service_info(artifact: &str) -> String {
    serde_json::json!({
        "id": format!("org.example.{artifact}"),
        "name": "Example",
        "type": {"group": "org.ga4gh", "artifact": artifact, "version": "1.0.0"},
        "organization": {"name": "Example", "url": "https://example.org"},
        "version": "1.0.0"
    })
    .to_string()
}

// Reads one request: its headers, then a `Content-Length` or chunked body.
async fn read_request(socket: &mut TcpStream) -> Option<String> {
    let mut data = Vec::new();
    let mut buffer = [0; 8192];
    let header_end = loop {
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).to_lowercase();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok());
    let chunked = head.contains("transfer-encoding: chunked");
    loop {
        let body = &data[header_end..];
        let complete = match length {
            Some(length) => body.len() >= length,
            None if chunked => body.ends_with(b"0\r\n\r\n"),
            None => true,
        };
        if complete {
            break;
        }
        let read = socket.read(&mut buffer).await.ok()?;
        if read == 0 {
            return None;
        }
        data.extend_from_slice(&buffer[..read]);
    }
    let line = head.lines().next()?.to_owned();
    Some(line.split_whitespace().nth(1)?.to_owned())
}

async fn serve(routes: Vec<(&'static str, Reply)>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counter = requests.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let routes = routes.clone();
            let counter = counter.clone();
            tokio::spawn(async move {
                let Some(path) = read_request(&mut socket).await else {
                    return;
                };
                counter.fetch_add(1, Ordering::SeqCst);
                let reply = routes
                    .iter()
                    .find(|(route, _)| path.starts_with(route))
                    .map(|(_, reply)| *reply);
                let answer = match reply {
                    Some(Reply::After(delay, body)) => Some((delay, body.to_owned())),
                    Some(Reply::ServiceInfo(delay, artifact)) => {
                        Some((delay, service_info(artifact)))
                    }
                    _ => None,
                };
                match (reply, answer) {
                    (_, Some((delay, body))) => {
                        tokio::time::sleep(delay).await;
                        let response = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                             content-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                    }
                    (Some(Reply::Never), _) => tokio::time::sleep(Duration::from_secs(30)).await,
                    _ => {
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\
                                  connection: close\r\n\r\n",
                            )
                            .await;
                    }
                }
            });
        }
    });
    Server { url, requests }
}

fn config(url: &str, timeouts: Timeouts) -> Configuration {
    Configuration::new(url::Url::parse(url).unwrap()).with_timeouts(timeouts)
}

fn slow_service_info(artifact: &'static str) -> Vec<(&'static str, Reply)> {
    vec![("/service-info", Reply::ServiceInfo(DELAY, artifact))]
}

// Creates the client of `artifact` with `new`, and for TES and WES also with
// `discover`; gives the result of each, named.
async fn connect(artifact: &str, config: &Configuration) -> Vec<(String, Result<(), String>)> {
    let name = artifact.to_uppercase();
    let mut results = vec![(
        format!("{name}::new"),
        match artifact {
            "tes" => TES::new(config).await.map(drop),
            "wes" => WES::new(config).await.map(drop),
            "drs" => DRS::new(config).await.map(drop),
            _ => TRS::new(config).await.map(drop),
        }
        .map_err(|error| error.to_string()),
    )];
    let discovered = match artifact {
        "tes" => Some(TES::discover(config, LONG).await.map(drop)),
        "wes" => Some(WES::discover(config, LONG).await.map(drop)),
        _ => None,
    };
    if let Some(result) = discovered {
        results.push((
            format!("{name}::discover"),
            result.map_err(|error| error.to_string()),
        ));
    }
    results
}

#[test]
fn defaults_keep_the_previous_limits() {
    let timeouts = Timeouts::default();
    assert_eq!(timeouts.connect, Duration::from_secs(10));
    assert_eq!(timeouts.request, Duration::from_secs(60));
    assert_eq!(timeouts.discovery, Duration::from_secs(10));
    assert_eq!(timeouts.submission, Duration::from_secs(60));
    let config = Configuration::new(url::Url::parse("https://example.org").unwrap());
    assert_eq!(config.timeouts, timeouts);
    assert_eq!(Configuration::default().timeouts, timeouts);
}

#[test]
fn transport_with_timeouts_records_them_in_its_configuration() {
    let config = Configuration::new(url::Url::parse("https://example.org").unwrap());
    let transport = Transport::with_timeouts(&config, SHORT, LONG).unwrap();
    assert_eq!(transport.config.timeouts.connect, SHORT);
    assert_eq!(transport.config.timeouts.request, LONG);
    assert_eq!(transport.config.timeouts.discovery, Duration::from_secs(10));
}

#[test]
fn an_operation_with_a_huge_budget_does_not_overflow() {
    let operation = Operation::new(Duration::MAX);
    assert!(operation.deadline > tokio::time::Instant::now() + Duration::from_secs(86_400));
}

#[tokio::test]
async fn request_timeout_applies_to_ordinary_requests() {
    let server = serve(vec![("/tasks", Reply::After(DELAY, "{}"))]).await;
    let timeouts = Timeouts {
        request: SHORT,
        ..Default::default()
    };
    let transport = Transport::new(&config(&server.url, timeouts));
    assert!(transport.get("tasks", None).await.is_err());

    let transport = Transport::new(&config(&server.url, Timeouts::default()));
    assert_eq!(transport.get("tasks", None).await.unwrap(), "{}");
}

#[tokio::test]
async fn slow_service_info_fails_after_the_discovery_timeout() {
    let timeouts = Timeouts {
        discovery: SHORT,
        ..Default::default()
    };
    for artifact in ["tes", "wes", "drs", "trs"] {
        let server = serve(slow_service_info(artifact)).await;
        for (name, result) in connect(artifact, &config(&server.url, timeouts)).await {
            assert!(result.is_err(), "{name} should time out");
        }
    }
}

#[tokio::test]
async fn a_longer_discovery_timeout_overrides_a_shorter_request_timeout() {
    // The request timeout is shorter than the delay; the discovery timeout
    // alone limits service-info, in `new` and `discover` alike.
    let timeouts = Timeouts {
        request: SHORT,
        discovery: LONG,
        ..Default::default()
    };
    for artifact in ["tes", "wes", "drs", "trs"] {
        let server = serve(slow_service_info(artifact)).await;
        for (name, result) in connect(artifact, &config(&server.url, timeouts)).await {
            result.unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }
}

#[tokio::test]
async fn clients_accept_a_ready_transport() {
    for artifact in ["tes", "wes", "drs", "trs"] {
        let server = serve(vec![(
            "/service-info",
            Reply::ServiceInfo(Duration::ZERO, artifact),
        )])
        .await;
        let config = Configuration::new(url::Url::parse(&server.url).unwrap())
            .with_user_agent("custom agent".to_owned());
        let transport = Transport::with_client(&config, ga4gh_sdk::reqwest::Client::new());
        match artifact {
            "tes" => {
                let tes = TES::with_transport(transport.clone()).await.unwrap();
                assert_eq!(tes.config.user_agent.as_deref(), Some("custom agent"));
                TES::discover_with_transport(transport, LONG).await.unwrap();
            }
            "wes" => {
                let wes = WES::with_transport(transport.clone()).await.unwrap();
                assert_eq!(wes.config.user_agent.as_deref(), Some("custom agent"));
                WES::discover_with_transport(transport, LONG).await.unwrap();
            }
            "drs" => {
                let drs = DRS::with_transport(transport).await.unwrap();
                assert_eq!(drs.config.user_agent.as_deref(), Some("custom agent"));
            }
            _ => {
                let trs = TRS::with_transport(transport).await.unwrap();
                assert_eq!(trs.config.user_agent.as_deref(), Some("custom agent"));
            }
        }
    }
}

fn run_request() -> RunRequest {
    RunRequest {
        workflow_params: Some(serde_json::json!({})),
        ..RunRequest::new(
            "CWL".to_owned(),
            "v1.2".to_owned(),
            "https://example.org/workflow.cwl".to_owned(),
        )
    }
}

#[tokio::test]
async fn a_longer_submission_timeout_overrides_a_shorter_request_timeout() {
    let timeouts = Timeouts {
        request: SHORT,
        submission: LONG,
        ..Default::default()
    };
    let server = serve(vec![
        ("/service-info", Reply::ServiceInfo(Duration::ZERO, "tes")),
        ("/tasks", Reply::After(DELAY, r#"{"id":"task-1"}"#)),
    ])
    .await;
    let tes = TES::new(&config(&server.url, timeouts)).await.unwrap();
    assert_eq!(tes.create(TesTask::default()).await.unwrap().id, "task-1");

    let server = serve(vec![
        ("/service-info", Reply::ServiceInfo(Duration::ZERO, "wes")),
        ("/runs", Reply::After(DELAY, r#"{"run_id":"run-1"}"#)),
    ])
    .await;
    let wes = WES::new(&config(&server.url, timeouts)).await.unwrap();
    assert_eq!(wes.run_workflow(run_request()).await.unwrap().id, "run-1");
}

#[tokio::test]
async fn a_submission_that_times_out_after_sending_is_ambiguous_and_sent_once() {
    let timeouts = Timeouts {
        submission: SHORT,
        ..Default::default()
    };
    let server = serve(vec![
        ("/service-info", Reply::ServiceInfo(Duration::ZERO, "tes")),
        ("/tasks", Reply::Never),
    ])
    .await;
    let tes = TES::new(&config(&server.url, timeouts)).await.unwrap();
    let started = tokio::time::Instant::now();
    let error = tes.create(TesTask::default()).await.unwrap_err();
    assert!(started.elapsed() < LONG, "{:?}", started.elapsed());
    assert!(
        error.downcast_ref::<AmbiguousSubmissionError>().is_some(),
        "{error}"
    );
    tokio::time::sleep(SHORT).await;
    // One service-info request and one POST: the submission is not repeated.
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);

    let server = serve(vec![
        ("/service-info", Reply::ServiceInfo(Duration::ZERO, "wes")),
        ("/runs", Reply::Never),
    ])
    .await;
    let wes = WES::new(&config(&server.url, timeouts)).await.unwrap();
    let error = wes.run_workflow(run_request()).await.unwrap_err();
    assert!(
        error.downcast_ref::<AmbiguousSubmissionError>().is_some(),
        "{error}"
    );
    tokio::time::sleep(SHORT).await;
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
}
