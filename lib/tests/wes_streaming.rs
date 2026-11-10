use ga4gh_sdk::{
    clients::{
        serviceinfo::models::{Service, ServiceType},
        wes::{
            attachments::{AttachmentSource, StreamingAttachment},
            models::RunRequest,
            WES,
        },
    },
    utils::{configuration::Configuration, operation::Operation, transport::Transport},
};
use mockito::{mock, server_url, Matcher};
use std::{
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, ReadBuf};
fn wes(base: &str) -> WES {
    let config = Configuration::new(url::Url::parse(base).unwrap());
    WES {
        transport: Transport::new(&config),
        config,
        service: Ok(Service {
            r#type: Box::new(ServiceType {
                artifact: "wes".into(),
                ..Default::default()
            }),
            ..Default::default()
        }),
    }
}
fn request() -> RunRequest {
    RunRequest {
        workflow_type: "CWL".into(),
        workflow_type_version: "v1.2".into(),
        workflow_url: "main.cwl".into(),
        workflow_params: Some(serde_json::json!({})),
        ..Default::default()
    }
}
#[tokio::test]
async fn sources_preserve_nested_filenames_content_and_shared_header_policy() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("tool.cwl");
    std::fs::write(&file, "file contents").unwrap();
    let mut wes = wes(&server_url());
    let mut headers = ga4gh_sdk::reqwest::header::HeaderMap::new();
    headers.insert("authorization", "Bearer test-only".parse().unwrap());
    wes.transport = Transport::with_client(
        &wes.config,
        ga4gh_sdk::reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .unwrap(),
    );
    let json = mock("POST", "/json")
        .match_header("authorization", "Bearer test-only")
        .with_body("{}")
        .expect(1)
        .create();
    wes.transport.post("json", None).await.unwrap();
    let upload = mock("POST", "/runs")
        .match_header("authorization", "Bearer test-only")
        .match_body(Matcher::AllOf(vec![
            Matcher::Regex("filename=\"main.cwl\"".into()),
            Matcher::Regex("bytes contents".into()),
            Matcher::Regex("filename=\"tools/tool.cwl\"".into()),
            Matcher::Regex("file contents".into()),
            Matcher::Regex("reader contents".into()),
        ]))
        .with_body(r#"{"run_id":"opaque / % id"}"#)
        .expect(1)
        .create();
    let files = vec![
        StreamingAttachment {
            filename: "main.cwl".into(),
            source: AttachmentSource::Bytes(b"bytes contents".to_vec()),
        },
        StreamingAttachment {
            filename: "tools/tool.cwl".into(),
            source: AttachmentSource::File(file),
        },
        StreamingAttachment::reader(
            "data/input.txt".into(),
            std::io::Cursor::new(b"reader contents".to_vec()),
            None,
        ),
    ];
    assert_eq!(
        wes.run_workflow_streaming(request(), files, &Operation::new(Duration::from_secs(3)))
            .await
            .unwrap()
            .id,
        "opaque / % id"
    );
    json.assert();
    upload.assert();
}
struct Generated {
    read: Arc<AtomicUsize>,
    remaining: usize,
    fail: bool,
}
impl AsyncRead for Generated {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.fail && self.read.load(Ordering::Relaxed) > 0 {
            return Poll::Ready(Err(std::io::Error::other("injected reader failure")));
        }
        let count = 1024.min(buf.remaining()).min(self.remaining);
        buf.put_slice(&vec![b'x'; count]);
        self.remaining -= count;
        self.read.fetch_add(count, Ordering::Relaxed);
        Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn generated_large_upload_obeys_backpressure_and_cancellation() {
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted, ready) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(socket.read(&mut bytes).await.unwrap() > 0);
        accepted.send(()).unwrap();
        tokio::time::sleep(Duration::from_secs(5)).await;
    });
    let read = Arc::new(AtomicUsize::new(0));
    let total = 64 * 1024 * 1024;
    let source = Generated {
        read: read.clone(),
        remaining: total,
        fail: false,
    };
    let wes = wes(&format!("http://{address}"));
    let operation = Operation::new(Duration::from_secs(3));
    let token = operation.cancellation.clone();
    let upload = wes.run_workflow_streaming(
        request(),
        vec![StreamingAttachment::reader(
            "large.bin".into(),
            source,
            Some(total as u64),
        )],
        &operation,
    );
    let monitor = async {
        ready.await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let consumed = read.load(Ordering::Relaxed);
        assert!(
            consumed > 0 && consumed < total / 2,
            "eagerly consumed {consumed} of {total}"
        );
        token.cancel();
    };
    let start = std::time::Instant::now();
    let (result, ()) = tokio::join!(upload, monitor);
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("may have been accepted"));
    assert!(start.elapsed() < Duration::from_secs(2));
    server.abort();
}
#[tokio::test]
async fn reader_failure_and_invalid_paths_are_actionable_without_replay() {
    use tokio::{io::AsyncReadExt, net::TcpListener};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        while socket.read(&mut bytes).await.unwrap_or(0) > 0 {}
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let wes = wes(&format!("http://{address}"));
    let source = Generated {
        read: Arc::new(AtomicUsize::new(0)),
        remaining: 4096,
        fail: true,
    };
    let error = wes
        .run_workflow_streaming(
            request(),
            vec![StreamingAttachment::reader("data.bin".into(), source, None)],
            &Operation::new(Duration::from_secs(2)),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("injected reader failure"),
        "{error}"
    );
    server.await.unwrap();
    for filename in [
        "../escape",
        "/absolute",
        "tools/../escape",
        "tools\\escape",
        "C:escape",
        "a//b",
    ] {
        let error = wes
            .run_workflow_streaming(
                request(),
                vec![StreamingAttachment {
                    filename: filename.into(),
                    source: AttachmentSource::Bytes(vec![]),
                }],
                &Operation::new(Duration::from_secs(1)),
            )
            .await
            .unwrap_err();
        assert!(error.to_string().contains("safe relative path"));
    }
}

#[tokio::test]
async fn streamed_submission_validates_acknowledgement_once() {
    use ga4gh_sdk::utils::acknowledgement::AcknowledgementError;
    let wes = wes(&server_url());
    for body in [
        "{}",
        r#"{"run_id":null}"#,
        r#"{"run_id":4}"#,
        r#"{"run_id":""}"#,
    ] {
        let post = mock("POST", "/runs").with_body(body).expect(1).create();
        let error = wes
            .run_workflow_streaming(
                request(),
                Vec::new(),
                &Operation::new(Duration::from_secs(2)),
            )
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<AcknowledgementError>().is_some());
        post.assert();
    }
}
