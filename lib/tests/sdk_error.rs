//! The SDK error: clients and their futures move between threads, and callers
//! match on the kind of failure.

use ga4gh_sdk::clients::drs::DRS;
use ga4gh_sdk::clients::registry::ServiceRegistry;
use ga4gh_sdk::clients::tes::{Task, TesView, TES};
use ga4gh_sdk::clients::trs::TRS;
use ga4gh_sdk::clients::wes::{Run, WES};
use ga4gh_sdk::utils::configuration::Configuration;
use ga4gh_sdk::utils::transport::{HttpError, Transport};
use ga4gh_sdk::Error;

fn service_info(artifact: &str) -> String {
    serde_json::json!({
        "id": "org.example.service",
        "name": "Example",
        "type": { "group": "org.ga4gh", "artifact": artifact, "version": "1.1.0" },
        "organization": { "name": "Example", "url": "https://example.org" },
        "version": "1.0.0"
    })
    .to_string()
}

fn config(server: &mockito::ServerGuard, path: &str) -> Configuration {
    Configuration::new(url::Url::parse(&format!("{}{path}", server.url())).unwrap())
}

#[test]
fn clients_and_errors_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<Error>();
    assert_send_sync::<TES>();
    assert_send_sync::<Task>();
    assert_send_sync::<WES>();
    assert_send_sync::<Run>();
    assert_send_sync::<DRS>();
    assert_send_sync::<TRS>();
    assert_send_sync::<ServiceRegistry>();
}

#[tokio::test]
async fn spawned_client_reports_http_and_parse_errors() {
    let mut server = mockito::Server::new_async().await;
    let _service = server
        .mock("GET", "/kinds/tes/v1/service-info")
        .with_body(service_info("tes"))
        .create();
    let _denied = server
        .mock("GET", "/kinds/tes/v1/tasks/denied?view=FULL")
        .with_status(401)
        .with_body("unauthorized")
        .create();
    let _malformed = server
        .mock("GET", "/kinds/tes/v1/tasks/malformed?view=FULL")
        .with_body("not JSON")
        .create();

    let config = config(&server, "/kinds/tes/v1");
    let tes = tokio::spawn(async move { TES::new(&config).await })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(tes.discovered_service_info().r#type.artifact, "tes");
    let (denied, malformed) = tokio::spawn(async move {
        (
            tes.get(TesView::Full, "denied").await.unwrap_err(),
            tes.get(TesView::Full, "malformed").await.unwrap_err(),
        )
    })
    .await
    .unwrap();

    match denied {
        Error::Http(HttpError { status, body }) => {
            assert_eq!(status, 401);
            assert_eq!(body, "unauthorized");
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }
    assert!(matches!(malformed, Error::Parse(_)), "{malformed:?}");
}

#[tokio::test]
async fn constructors_return_service_and_transport_errors() {
    let mut server = mockito::Server::new_async().await;
    let _service = server
        .mock("GET", "/kinds/wrong/service-info")
        .with_body(service_info("wes"))
        .create();
    let _other = server
        .mock("GET", "/kinds/other/service-info")
        .with_body(service_info("tes"))
        .create();
    let errors = [
        (
            "TES",
            TES::new(&config(&server, "/kinds/wrong"))
                .await
                .unwrap_err(),
        ),
        (
            "WES",
            WES::new(&config(&server, "/kinds/other"))
                .await
                .unwrap_err(),
        ),
        (
            "DRS",
            DRS::new(&config(&server, "/kinds/other"))
                .await
                .unwrap_err(),
        ),
        (
            "TRS",
            TRS::new(&config(&server, "/kinds/other"))
                .await
                .unwrap_err(),
        ),
    ];
    for (api, error) in errors {
        assert!(matches!(error, Error::UnexpectedService(_)), "{error:?}");
        assert_eq!(
            error.to_string(),
            format!("The endpoint is not an instance of {api}")
        );
    }

    // Nothing listens on the port of a listener that was just closed.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let closed = Configuration::new(url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap());
    // DRS and WES keep the variant, too; only an error status gets their hint.
    let errors = [
        TES::new(&closed).await.unwrap_err(),
        WES::new(&closed).await.unwrap_err(),
        DRS::new(&closed).await.unwrap_err(),
        TRS::new(&closed).await.unwrap_err(),
    ];
    for error in errors {
        assert!(matches!(error, Error::Transport(_)), "{error:?}");
    }
}

#[tokio::test]
async fn discovery_hint_keeps_the_http_error() {
    let mut server = mockito::Server::new_async().await;
    let _service = server
        .mock("GET", "/kinds/missing/service-info")
        .with_status(404)
        .with_body("missing")
        .create();
    let errors = [
        (
            "DRS",
            DRS::new(&config(&server, "/kinds/missing"))
                .await
                .unwrap_err(),
        ),
        (
            "WES",
            WES::new(&config(&server, "/kinds/missing"))
                .await
                .unwrap_err(),
        ),
    ];
    for (api, error) in errors {
        assert!(matches!(error, Error::Other(_)), "{error:?}");
        assert!(
            error
                .to_string()
                .ends_with(&format!("Check that base_path is the {api} API root")),
            "{error}"
        );
        let http = error.downcast_ref::<HttpError>().expect("HTTP error");
        assert_eq!(http.status, 404);
        // A box holds the SDK error; downcast to it first.
        let boxed: Box<dyn std::error::Error> = error.into();
        assert!(boxed.downcast_ref::<HttpError>().is_none());
        let sdk = boxed.downcast_ref::<Error>().expect("SDK error");
        assert!(sdk.downcast_ref::<HttpError>().is_some());
    }
}

#[tokio::test]
async fn resolution_keeps_the_typed_request_error() {
    use ga4gh_sdk::clients::drs::resolution::{resolve_until, ResolutionReference, ResolveCause};
    use ga4gh_sdk::utils::operation::Operation;

    let reference = ResolutionReference {
        api_root: "https://example.org/".to_owned(),
        endpoint: "objects/1".to_owned(),
    };
    let operation = Operation::new(std::time::Duration::from_secs(5));
    let error = resolve_until::<(), _, _>(reference, &operation, 1, || async {
        Err(Error::from(serde_json::from_str::<u8>("x").unwrap_err()))
    })
    .await
    .unwrap_err();
    assert!(
        matches!(error.cause, ResolveCause::Request(Error::Parse(_))),
        "{error:?}"
    );
    // The request error is the source of the resolution error, so that
    // `downcast_ref` finds it through the SDK error, too.
    let error = Error::other(error);
    assert!(error.downcast_ref::<serde_json::Error>().is_some());
}

#[tokio::test]
async fn discovery_reports_another_service_as_unexpected() {
    let mut server = mockito::Server::new_async().await;
    let _service = server
        .mock("GET", "/kinds/discover/service-info")
        .with_body(service_info("wes"))
        .create();
    let max_age = std::time::Duration::from_secs(60);
    let error = TES::discover(&config(&server, "/kinds/discover"), max_age)
        .await
        .map(|_| ())
        .unwrap_err();
    assert!(matches!(error, Error::UnexpectedService(_)), "{error:?}");
    assert_eq!(error.to_string(), "expected tes service-info");
}

#[test]
fn bearer_token_from_env_errors_are_configuration_errors() {
    let error = Configuration::new(url::Url::parse("https://example.org/").unwrap())
        .with_bearer_access_token_from_env("GA4GH_SDK_TEST_SDK_ERROR_BEARER_UNSET")
        .unwrap_err();
    assert!(matches!(error, Error::Configuration(_)), "{error:?}");
    assert!(
        error
            .to_string()
            .contains("GA4GH_SDK_TEST_SDK_ERROR_BEARER_UNSET"),
        "{error}"
    );
}

#[tokio::test]
async fn endpoint_and_configuration_errors_have_their_own_kind() {
    let server = mockito::Server::new_async().await;
    let task = Task::new("..".to_owned(), Transport::new(&config(&server, "/kinds")));
    let error = task.status().await.unwrap_err();
    assert!(matches!(error, Error::Endpoint(_)), "{error:?}");

    let dir = tempfile::tempdir().unwrap();
    let error = Configuration::from_path(dir.path().join("missing.json"), "TES").unwrap_err();
    assert!(matches!(error, Error::Configuration(_)), "{error:?}");
    assert_eq!(
        error.downcast_ref::<std::io::Error>().unwrap().kind(),
        std::io::ErrorKind::NotFound
    );

    let error = Configuration::new(url::Url::parse("https://example.org/").unwrap())
        .with_oauth_access_token_from_env("GA4GH_SDK_TEST_SDK_ERROR_TOKEN_UNSET")
        .unwrap_err();
    assert!(matches!(error, Error::Configuration(_)), "{error:?}");
}

#[test]
fn errors_convert_into_boxed_errors() {
    fn caller() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err(Error::UnexpectedService("not a TES".to_owned()))?
    }
    assert_eq!(caller().unwrap_err().to_string(), "not a TES");
}
