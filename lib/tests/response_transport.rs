use ga4gh_sdk::{
    reqwest::Method,
    utils::{
        configuration::Configuration,
        transport::{read_bounded, BodyLimitError, BodyLimits, Transport},
    },
};
use mockito::{mock, server_url};

fn transport() -> Transport {
    Transport::new(&Configuration::new(url::Url::parse(&server_url()).unwrap()))
}

#[tokio::test]
async fn binary_and_metadata_survive_before_parsing() {
    let bytes = vec![0x50, 0x4b, 3, 4, 0xff, 0xfe, 0, 0x80];
    let request = mock("GET", "/zip")
        .with_status(202)
        .with_header("retry-after", "1")
        .with_header("content-type", "application/zip")
        .with_header("next_page", "opaque+token/%")
        .with_body(bytes.clone())
        .create();
    let response = transport()
        .request_response(Method::GET, "zip", None, None)
        .await
        .unwrap();
    assert_eq!(response.status(), 202);
    assert_eq!(response.headers()["retry-after"], "1");
    assert_eq!(response.headers()["next_page"], "opaque+token/%");
    let buffered = read_bounded(response, 32).await.unwrap();
    assert_eq!(buffered.body, bytes);
    assert_eq!(buffered.headers["content-type"], "application/zip");
    request.assert();
}

#[tokio::test]
async fn success_and_error_bounds_preserve_status() {
    for status in [200, 503] {
        let path = format!("/limit{status}");
        let request = mock("GET", path.as_str())
            .with_status(status)
            .with_body("x".repeat(1024))
            .create();
        let error = transport()
            .request_text(
                Method::GET,
                &path,
                None,
                None,
                BodyLimits {
                    success: 64,
                    error: 32,
                },
            )
            .await
            .unwrap_err();
        let limit = error.downcast_ref::<BodyLimitError>().unwrap();
        assert_eq!(limit.status.as_u16(), status as u16);
        assert_eq!(limit.limit, if status == 200 { 64 } else { 32 });
        request.assert();
    }
}

#[tokio::test]
async fn chunked_bodies_are_bounded_without_a_length() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    for status in [200, 503] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(
                    format!("HTTP/1.1 {status} Test\r\nTransfer-Encoding: chunked\r\n\r\n")
                        .as_bytes(),
                )
                .await
                .unwrap();
            for _ in 0..100 {
                if socket
                    .write_all(b"10\r\nabcdefghijklmnop\r\n")
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let transport = Transport::new(&Configuration::new(
            url::Url::parse(&format!("http://{address}")).unwrap(),
        ));
        let response = transport
            .request_response(Method::GET, "test", None, None)
            .await
            .unwrap();
        assert!(response.content_length().is_none());
        let error = read_bounded(response, 31).await.unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<BodyLimitError>()
                .unwrap()
                .status
                .as_u16(),
            status
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn trs_pagination_uses_urls_and_zip_is_opaque() {
    use ga4gh_sdk::clients::trs::{
        models::DescriptorType,
        responses::{files_zip, list_tools_page, next_tools_page},
    };
    let next = format!("{}/tools?offset=opaque%2B%2F%25&limit=2", server_url());
    let first = mock("GET", "/tools")
        .with_header("next_page", &next)
        .with_body("[]")
        .create();
    let second = mock("GET", "/tools?offset=opaque%2B%2F%25&limit=2")
        .with_body("[]")
        .expect(1)
        .create();
    let transport = transport();
    let page = list_tools_page(&transport, None, BodyLimits::default())
        .await
        .unwrap();
    assert_eq!(page.next_page().unwrap(), Some(next.as_str()));
    assert!(next_tools_page(
        &transport,
        page.next_page().unwrap().unwrap(),
        BodyLimits::default()
    )
    .await
    .unwrap()
    .tools
    .is_empty());
    assert!(next_tools_page(
        &transport,
        "https://foreign.example/tools",
        BodyLimits::default()
    )
    .await
    .is_err());
    first.assert();
    second.assert();
    let bytes = include_bytes!("../../tests/fixtures/trs/workflow.zip").to_vec();
    let zip = mock("GET", "/tools/a%2Fb/versions/v%201/CWL/files?format=zip")
        .with_body(bytes.clone())
        .with_header("content-type", "application/zip")
        .expect(1)
        .create();
    let response = files_zip(&transport, "a/b", "v 1", DescriptorType::Cwl)
        .await
        .unwrap();
    assert_eq!(read_bounded(response, 1024).await.unwrap().body, bytes);
    zip.assert();

    // Unrecognized descriptor types are one encoded path segment.
    let unrecognized = mock(
        "GET",
        "/tools/a%2Fb/versions/v%201/CWL%2Ffiles%3Fformat%3Djson/files?format=zip",
    )
    .with_body(bytes.clone())
    .expect(1)
    .create();
    let descriptor = DescriptorType::Unrecognized("CWL/files?format=json".into());
    let response = files_zip(&transport, "a/b", "v 1", descriptor)
        .await
        .unwrap();
    assert_eq!(read_bounded(response, 1024).await.unwrap().body, bytes);
    unrecognized.assert();
    for value in ["", ".", ".."] {
        let descriptor = DescriptorType::Unrecognized(value.into());
        let error = files_zip(&transport, "a/b", "v 1", descriptor)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("descriptor type"), "{error}");
    }
}

#[tokio::test]
async fn dropping_a_response_stops_the_producer() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::{timeout, Duration},
    };
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let producer = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 2048];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        loop {
            if socket
                .write_all(b"10\r\nabcdefghijklmnop\r\n")
                .await
                .is_err()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    let transport = Transport::new(&Configuration::new(
        url::Url::parse(&format!("http://{address}")).unwrap(),
    ));
    let mut response = transport
        .request_response(Method::GET, "stream", None, None)
        .await
        .unwrap();
    assert!(response.chunk().await.unwrap().is_some());
    drop(response);
    timeout(Duration::from_secs(2), producer)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn trs_relative_page_link_keeps_configured_basic_auth_scoped_to_the_api() {
    use ga4gh_sdk::clients::trs::responses::next_tools_page;
    use ga4gh_sdk::utils::configuration::BasicAuth;
    let transport = Transport::new(
        &Configuration::new(url::Url::parse(&server_url()).unwrap()).with_basic_auth(BasicAuth {
            username: "user".into(),
            password: Some("pass".into()),
        }),
    );
    let request = mock("GET", "/tools?offset=opaque")
        .match_header("authorization", "Basic dXNlcjpwYXNz")
        .with_body("[]")
        .expect(1)
        .create();
    assert!(
        next_tools_page(&transport, "?offset=opaque", BodyLimits::default())
            .await
            .unwrap()
            .tools
            .is_empty()
    );
    request.assert();
}

#[tokio::test]
async fn strict_json_validates_utf8_even_in_ignored_properties() {
    let request = mock("GET", "/strict-json-ignored-utf8")
        .with_status(200)
        .with_body(b"{\"ignored\":\"\xff\"}")
        .create();
    let response = transport()
        .request_response(Method::GET, "strict-json-ignored-utf8", None, None)
        .await
        .unwrap();
    let error = ga4gh_sdk::utils::transport::response_json::<serde::de::IgnoredAny>(
        response,
        BodyLimits::default(),
    )
    .await
    .expect_err("invalid UTF-8 must not be skipped with an unknown field");
    assert!(error.downcast_ref::<std::str::Utf8Error>().is_some());
    request.assert();
}
