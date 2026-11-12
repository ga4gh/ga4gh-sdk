use ga4gh_sdk::utils::configuration::{AuthError, BasicAuth, Configuration};
use ga4gh_sdk::utils::transport::{HttpError, Transport};
use mockito::{mock, Matcher};
use reqwest::{header, multipart::Form, Client, Method, StatusCode};
use url::Url;

fn config(path: &str) -> Configuration {
    Configuration::new(Url::parse(&format!("{}{path}", mockito::server_url())).unwrap())
}

#[tokio::test]
async fn configured_basic_is_sent_to_discovery_and_multipart() {
    let discovery = mock("GET", "/basic/service-info")
        .match_header("authorization", "Basic YWxpY2U6cHc=")
        .with_body("ok")
        .create();
    let submission = mock("POST", "/basic/runs")
        .match_header("authorization", "Basic YWxpY2U6cHc=")
        .with_body("ok")
        .create();
    let config = config("/basic").with_basic_auth(BasicAuth {
        username: "alice".into(),
        password: Some("pw".into()),
    });
    let transport = Transport::new(&config);
    let built = transport
        .request_builder(Method::GET, "service-info")
        .unwrap()
        .build()
        .unwrap();
    assert!(built.headers()[header::AUTHORIZATION].is_sensitive());
    assert_eq!(transport.get("service-info", None).await.unwrap(), "ok");
    assert_eq!(
        transport
            .post_multipart("runs", Form::new().text("part", "value"))
            .await
            .unwrap(),
        "ok"
    );
    discovery.assert();
    submission.assert();
}

#[tokio::test]
async fn configured_bearer_is_sent_on_all_standard_methods() {
    let config = config("/bearer").with_oauth_access_token("test-token".into());
    let transport = Transport::new(&config);
    for (method, endpoint) in [
        (Method::GET, "get"),
        (Method::POST, "post"),
        (Method::PUT, "put"),
        (Method::DELETE, "delete"),
    ] {
        let call = mock(method.as_str(), format!("/bearer/{endpoint}").as_str())
            .match_header("authorization", "Bearer test-token")
            .with_body("ok")
            .create();
        let response = transport
            .request_response(method, endpoint, None, None)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        call.assert();
    }
}

#[tokio::test]
async fn invalid_or_conflicting_sources_fail_before_network_io() {
    let request = mock("GET", "/invalid/objects").expect(0).create();
    let mut config = config("/invalid").with_oauth_access_token("secret-token".into());
    config.bearer_access_token = Some("another-token".into());
    let error = Transport::new(&config)
        .get("objects", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AuthError>(),
        Some(AuthError::Conflicting { .. })
    ));
    assert!(!error.to_string().contains("secret-token"));
    config.oauth_access_token = None;
    config.bearer_access_token = Some("bad\nvalue".into());
    let error = Transport::new(&config)
        .get("objects", None)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AuthError>(),
        Some(AuthError::InvalidBearer { .. })
    ));
    assert!(!error.to_string().contains("bad"));
    request.assert();
}

#[tokio::test]
async fn server_echoing_the_credential_cannot_put_it_in_error_diagnostics() {
    const TOKEN: &str = "SENTINEL_TOKEN";
    let body = format!("Authorization: Bearer {TOKEN}");
    let request = mock("GET", "/echo/error")
        .match_header("authorization", format!("Bearer {TOKEN}").as_str())
        .with_status(401)
        .with_body(&body)
        .create();
    let config = config("/echo").with_bearer_access_token(TOKEN.into());
    let error = Transport::new(&config)
        .get("error", None)
        .await
        .unwrap_err();
    let http = error.downcast_ref::<HttpError>().unwrap();
    assert_eq!(http.status, StatusCode::UNAUTHORIZED);
    assert_eq!(http.body, body);
    for diagnostic in [error.to_string(), format!("{error:?}"), format!("{http:?}")] {
        assert!(diagnostic.contains("401"), "{diagnostic}");
        assert!(!diagnostic.contains(TOKEN), "{diagnostic}");
    }
    request.assert();
}

#[tokio::test]
async fn authorized_requests_do_not_follow_same_origin_redirects_outside_root() {
    let first = mock("GET", "/root/start")
        .match_header("authorization", "Bearer secret-token")
        .with_status(302)
        .with_header("location", "/outside")
        .create();
    let outside = mock("GET", "/outside").expect(0).create();
    let mut config = config("/root");
    config.bearer_access_token = Some("secret-token".into());
    let error = Transport::new(&config)
        .get("start", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpError>().unwrap().status,
        StatusCode::FOUND
    );
    first.assert();
    outside.assert();
}

#[tokio::test]
async fn authenticated_requests_do_not_follow_cross_origin_or_replay_submissions() {
    let second_origin = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    second_origin.set_nonblocking(true).unwrap();
    let location = format!("http://{}/stolen", second_origin.local_addr().unwrap());
    let first = mock("GET", "/root/cross-origin")
        .match_header("authorization", "Bearer secret-token")
        .with_status(302)
        .with_header("location", &location)
        .create();
    let mut config = config("/root");
    config.bearer_access_token = Some("secret-token".into());
    let error = Transport::new(&config)
        .get("cross-origin", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpError>().unwrap().status,
        StatusCode::FOUND
    );
    assert_eq!(
        second_origin.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    first.assert();

    let submission = mock("POST", "/root/runs")
        .match_header("authorization", "Bearer secret-token")
        .with_status(307)
        .with_header("location", "/root/runs-again")
        .expect(1)
        .create();
    let replay = mock("POST", "/root/runs-again").expect(0).create();
    let error = Transport::new(&config)
        .post("runs", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpError>().unwrap().status,
        StatusCode::TEMPORARY_REDIRECT
    );
    submission.assert();
    replay.assert();
}

#[tokio::test]
async fn caller_owned_client_keeps_its_header_and_needs_opt_in_for_config_auth() {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_static("Bearer caller-token"),
    );
    let client = Client::builder().default_headers(headers).build().unwrap();
    let caller = mock("GET", "/injected/objects")
        .match_header("authorization", "Bearer caller-token")
        .with_body("ok")
        .create();
    let no_config_auth = config("/injected");
    let transport = Transport::with_client(&no_config_auth, client.clone());
    assert_eq!(transport.get("objects", None).await.unwrap(), "ok");
    caller.assert();

    let redirect = mock("GET", "/injected/redirect")
        .with_status(302)
        .with_header("location", "/injected/followed")
        .create();
    let followed = mock("GET", "/injected/followed")
        .match_header("authorization", "Bearer caller-token")
        .with_body("followed")
        .create();
    assert_eq!(transport.get("redirect", None).await.unwrap(), "followed");
    redirect.assert();
    followed.assert();

    let blocked = mock("GET", "/injected/blocked").expect(0).create();
    let with_config_auth = no_config_auth.with_oauth_access_token("sdk-token".into());
    let transport = Transport::with_client(&with_config_auth, client.clone());
    let error = transport.get("blocked", None).await.unwrap_err();
    assert!(matches!(
        error.downcast_ref::<AuthError>(),
        Some(AuthError::CallerOwnedClient)
    ));
    blocked.assert();

    let opted_in = mock("GET", "/injected/opted-in")
        .match_header("authorization", "Bearer sdk-token")
        .with_body("ok")
        .create();
    let transport = Transport::with_client_and_configured_auth(&with_config_auth, client);
    assert_eq!(transport.get("opted-in", None).await.unwrap(), "ok");
    opted_in.assert();
}

#[tokio::test]
async fn unauthenticated_request_does_not_gain_an_authorization_header() {
    let request = mock("GET", "/anonymous/objects")
        .match_header("authorization", Matcher::Missing)
        .with_body("ok")
        .create();
    assert_eq!(
        Transport::new(&config("/anonymous"))
            .get("objects", None)
            .await
            .unwrap(),
        "ok"
    );
    request.assert();
}

#[tokio::test]
async fn sdk_owned_anonymous_client_also_reports_redirects_without_following_them() {
    let redirect = mock("GET", "/anonymous/redirect")
        .with_status(301)
        .with_header("location", "/anonymous/elsewhere")
        .create();
    let destination = mock("GET", "/anonymous/elsewhere").expect(0).create();
    let error = Transport::new(&config("/anonymous"))
        .get("redirect", None)
        .await
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<HttpError>().unwrap().status,
        StatusCode::MOVED_PERMANENTLY
    );
    redirect.assert();
    destination.assert();
}
