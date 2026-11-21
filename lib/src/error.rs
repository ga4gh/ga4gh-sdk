//! The error type of the SDK.
//!
//! Clients, the transport and the configuration return [`Error`], an enum of
//! the kinds of failure that callers usually handle differently. It is `Send`
//! and `Sync`, so that clients and their futures can move between threads, for
//! example into [`tokio::spawn`].
//!
//! ```
//! use ga4gh_sdk::utils::operation::AmbiguousSubmissionError;
//! use ga4gh_sdk::Error;
//!
//! fn describe(error: &Error) -> String {
//!     match error {
//!         Error::Http(http) if http.status == 404 => "not found".to_owned(),
//!         Error::Http(http) => format!("service answered {}", http.status),
//!         Error::Transport(_) => "no response from the service".to_owned(),
//!         // Errors without a variant of their own can be downcast.
//!         other if other.downcast_ref::<AmbiguousSubmissionError>().is_some() => {
//!             "the service may have accepted the request".to_owned()
//!         }
//!         // More variants may be added.
//!         other => other.to_string(),
//!     }
//! }
//! # let _ = describe;
//! ```
//!
//! Code of its own, for example a [`DescriptorFetcher`] or a downstream
//! client, returns an error of another type with `?` where that type converts
//! into [`Error`], or with [`Error::other`] for any other error.
//!
//! [`DescriptorFetcher`]: crate::clients::bundle::DescriptorFetcher

use crate::utils::transport::{EndpointError, HttpError};
use std::error::Error as StdError;
use std::fmt;

/// A `Result` with [`Error`] as its default error type.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A boxed error that can move between threads, as held by
/// [`Error::Configuration`] and [`Error::Other`].
pub type BoxError = Box<dyn StdError + Send + Sync>;

/// An error of the SDK.
///
/// Most variants display as the error they hold; [`Error::Transport`] displays
/// only a safe failure category. Errors of other types, such
/// as [`OperationError`](crate::utils::operation::OperationError), are kept in
/// [`Error::Other`]; [`Error::downcast_ref`] finds the error of a given type in
/// any variant. More variants may be added, so a `match` needs a wildcard arm.
#[non_exhaustive]
pub enum Error {
    /// The service answered with an unsuccessful HTTP status.
    Http(HttpError),
    /// A request couldn't be sent, or its response couldn't be read, for
    /// example because of a connection failure or the HTTP client's timeout.
    ///
    /// Its default formatting classifies the failure without displaying the URL
    /// or source text, which may contain credentials or caller-provided data.
    /// Inspect the contained `reqwest::Error` and its sources explicitly for
    /// detailed diagnostics.
    Transport(reqwest::Error),
    /// A path or API root that can't be used for a request.
    Endpoint(EndpointError),
    /// A request or response body that isn't valid JSON for its model.
    Parse(serde_json::Error),
    /// A configuration file, section or environment variable that can't be
    /// used.
    Configuration(BoxError),
    /// The service isn't of the expected type, according to its service info.
    UnexpectedService(String),
    /// Any other error.
    Other(BoxError),
}

impl fmt::Debug for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(error) => f.debug_tuple("Http").field(error).finish(),
            Self::Transport(_) => f.debug_tuple("Transport").field(&self.to_string()).finish(),
            Self::Endpoint(error) => f.debug_tuple("Endpoint").field(error).finish(),
            Self::Parse(error) => f.debug_tuple("Parse").field(error).finish(),
            Self::Configuration(error) => f.debug_tuple("Configuration").field(error).finish(),
            Self::UnexpectedService(message) => {
                f.debug_tuple("UnexpectedService").field(message).finish()
            }
            Self::Other(error) => f.debug_tuple("Other").field(error).finish(),
        }
    }
}

impl Error {
    /// Wraps any error in [`Error::Other`], or in its own variant if it has one.
    pub fn other<E: Into<BoxError>>(error: E) -> Self {
        Self::from(error.into())
    }

    /// Returns the error of type `T` that this error holds, or the first of
    /// its sources of that type, if any.
    ///
    /// This works for the errors of every variant, for example
    /// `downcast_ref::<HttpError>()` for [`Error::Http`], so code written for
    /// a boxed error keeps working. It also finds the HTTP error of a failed
    /// service-info request that a DRS or WES constructor reports with a hint.
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        let mut next = self.inner();
        while let Some(error) = next {
            if let Some(found) = error.downcast_ref::<T>() {
                return Some(found);
            }
            next = match error.downcast_ref::<Error>() {
                Some(error) => error.inner(),
                None => error.source(),
            };
        }
        None
    }

    // The error that this error holds, if any.
    pub(crate) fn inner(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Http(error) => Some(error),
            Self::Transport(error) => Some(error),
            Self::Endpoint(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::Configuration(error) | Self::Other(error) => Some(error.as_ref()),
            Self::UnexpectedService(_) => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(error) => error.fmt(f),
            Self::Transport(error) if error.is_timeout() => f.write_str("HTTP request timed out"),
            Self::Transport(error) if error.is_connect() => f.write_str("HTTP connection failed"),
            Self::Transport(error) if error.is_builder() => {
                f.write_str("HTTP client configuration failed")
            }
            Self::Transport(error) if error.is_redirect() => f.write_str("HTTP redirect failed"),
            Self::Transport(error) if error.is_status() => match error.status() {
                Some(status) => write!(f, "HTTP response returned status {status}"),
                None => f.write_str("HTTP response returned an unsuccessful status"),
            },
            Self::Transport(error) if error.is_body() => {
                f.write_str("HTTP request or response body failed")
            }
            Self::Transport(error) if error.is_decode() => f.write_str("HTTP response body failed"),
            Self::Transport(error) if error.is_request() => f.write_str("HTTP request failed"),
            Self::Transport(_) => f.write_str("HTTP transport failed"),
            Self::Endpoint(error) => error.fmt(f),
            Self::Parse(error) => error.fmt(f),
            Self::Configuration(error) | Self::Other(error) => error.fmt(f),
            Self::UnexpectedService(message) => f.write_str(message),
        }
    }
}

impl StdError for Error {
    // Keep the underlying cause available even when default formatting is
    // intentionally safer than the contained error's Display implementation.
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        self.inner()?.source()
    }
}

impl From<HttpError> for Error {
    fn from(error: HttpError) -> Self {
        Self::Http(error)
    }
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Transport(error)
    }
}

impl From<EndpointError> for Error {
    fn from(error: EndpointError) -> Self {
        Self::Endpoint(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Parse(error)
    }
}

impl From<BoxError> for Error {
    /// Uses the variant of the boxed error's type, if it has one.
    fn from(error: BoxError) -> Self {
        let error = match error.downcast::<Error>() {
            Ok(error) => return *error,
            Err(error) => error,
        };
        let error = match error.downcast::<HttpError>() {
            Ok(error) => return Self::Http(*error),
            Err(error) => error,
        };
        let error = match error.downcast::<reqwest::Error>() {
            Ok(error) => return Self::Transport(*error),
            Err(error) => error,
        };
        let error = match error.downcast::<EndpointError>() {
            Ok(error) => return Self::Endpoint(*error),
            Err(error) => error,
        };
        match error.downcast::<serde_json::Error>() {
            Ok(error) => Self::Parse(*error),
            Err(error) => Self::Other(error),
        }
    }
}

impl From<String> for Error {
    fn from(message: String) -> Self {
        Self::Other(message.into())
    }
}

impl From<&str> for Error {
    fn from(message: &str) -> Self {
        Self::Other(message.into())
    }
}

// Errors without a variant of their own. Add every new public error type of
// the library here, so that `?` converts it.
macro_rules! from_other {
    ($($error:ty),* $(,)?) => {
        $(
            impl From<$error> for Error {
                fn from(error: $error) -> Self {
                    Self::Other(Box::new(error))
                }
            }
        )*
    };
}

from_other!(
    std::io::Error,
    std::num::ParseIntError,
    std::num::TryFromIntError,
    std::str::Utf8Error,
    std::string::FromUtf8Error,
    reqwest::header::ToStrError,
    url::ParseError,
    serde_yaml::Error,
    crate::compatibility::CompatibilityError,
    crate::utils::acknowledgement::AcknowledgementError,
    crate::utils::auth::AuthConflictError,
    crate::utils::auth::TokenProviderError,
    crate::utils::configuration::AuthError,
    crate::utils::operation::AmbiguousSubmissionError,
    crate::utils::operation::OperationError,
    crate::utils::query::QueryError,
    crate::utils::transport::BodyLimitError,
    crate::clients::drs::resolution::PendingResolution,
    crate::clients::drs::resolution::ResolveError,
    crate::clients::drs::uri::AccessChoiceError,
    crate::clients::drs::uri::UriError,
    crate::clients::pagination::PageFailure,
    crate::clients::tes::ParseTesViewError,
);

#[cfg(test)]
mod tests {
    use super::{BoxError, Error};
    use crate::utils::operation::OperationError;
    use crate::utils::transport::{EndpointError, HttpError};

    fn http() -> HttpError {
        HttpError {
            status: reqwest::StatusCode::NOT_FOUND,
            body: "missing".to_owned(),
        }
    }

    #[test]
    fn test_error_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Error>();
    }

    #[test]
    fn test_from_uses_the_variant_of_the_error() {
        assert!(matches!(Error::from(http()), Error::Http(_)));
        assert!(matches!(
            Error::from(EndpointError::InvalidPath),
            Error::Endpoint(EndpointError::InvalidPath)
        ));
        let parse = serde_json::from_str::<u8>("x").unwrap_err();
        assert!(matches!(Error::from(parse), Error::Parse(_)));
        assert!(matches!(Error::from("message"), Error::Other(_)));
        assert!(matches!(
            Error::from(OperationError::Deadline),
            Error::Other(_)
        ));
    }

    #[test]
    fn test_boxed_errors_keep_their_variant() {
        let boxed: BoxError = Box::new(http());
        assert!(matches!(Error::from(boxed), Error::Http(_)));
        let boxed: BoxError = Box::new(Error::UnexpectedService("not TES".to_owned()));
        assert!(matches!(Error::from(boxed), Error::UnexpectedService(_)));
        assert!(matches!(
            Error::other(OperationError::Cancelled),
            Error::Other(_)
        ));
        assert!(matches!(
            Error::other(EndpointError::InvalidBase),
            Error::Endpoint(_)
        ));
        let parse = serde_json::from_str::<u8>("x").unwrap_err();
        assert!(matches!(Error::other(parse), Error::Parse(_)));
        let boxed: BoxError = Box::new(OperationError::Deadline);
        assert!(matches!(Error::from(boxed), Error::Other(_)));
    }

    #[tokio::test]
    async fn test_boxed_reqwest_errors_are_transport_errors() {
        let error = reqwest::Client::new()
            .get("http://[::1]:0/")
            .send()
            .await
            .unwrap_err();
        let boxed: BoxError = Box::new(error);
        assert!(matches!(Error::from(boxed), Error::Transport(_)));
    }

    #[tokio::test]
    async fn test_transport_errors_classify_without_displaying_url_or_cause() {
        // A port that refuses connections: bind, read the port, close.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = reqwest::Client::new()
            .get(format!("http://{address}/?token=SENTINEL"))
            .send()
            .await
            .unwrap_err();
        let error = Error::from(error);
        assert_eq!(error.to_string(), "HTTP connection failed");
        let debug = format!("{error:?}");
        assert!(debug.contains("HTTP connection failed"), "{debug}");
        assert!(!debug.contains("SENTINEL"), "{debug}");
        assert!(std::error::Error::source(&error).is_some());
        assert!(error.downcast_ref::<reqwest::Error>().is_some());
    }

    /// The transport errors of each kind, with their messages.
    #[tokio::test]
    async fn test_transport_error_kinds_have_their_messages() {
        let mut server = mockito::Server::new_async().await;
        let _gateway = server.mock("GET", "/").with_status(502).create();
        let builder = reqwest::Client::new()
            .get("http://[")
            .build()
            .unwrap_err();
        let response = || async { reqwest::get(server.url()).await.unwrap() };
        let status = response().await.error_for_status().unwrap_err();
        // A response without `101 Switching Protocols` fails to upgrade.
        let upgrade = response().await.upgrade().await.unwrap_err();
        let failed: Vec<Result<Vec<u8>, std::io::Error>> =
            vec![Err(std::io::Error::other("broken"))];
        let mut body = reqwest::Body::wrap_stream(futures_util::stream::iter(failed));
        let body = http_body_util::BodyExt::frame(&mut body)
            .await
            .unwrap()
            .unwrap_err();
        for (error, message) in [
            (builder, "HTTP client configuration failed"),
            (status, "HTTP response returned status 502 Bad Gateway"),
            (body, "HTTP request or response body failed"),
            (upgrade, "HTTP transport failed"),
        ] {
            assert_eq!(Error::from(error).to_string(), message);
        }
    }

    #[test]
    fn test_debug_names_the_variant() {
        let parse = serde_json::from_str::<u8>("x").unwrap_err();
        for (error, variant) in [
            (Error::from(EndpointError::InvalidPath), "Endpoint(InvalidPath)"),
            (Error::from(parse), "Parse("),
            (Error::Configuration("bad file".into()), "Configuration(\"bad file\")"),
            (
                Error::UnexpectedService("not TES".to_owned()),
                "UnexpectedService(\"not TES\")",
            ),
        ] {
            let debug = format!("{error:?}");
            assert!(debug.starts_with(variant), "{debug}");
        }
    }

    #[test]
    fn test_downcast_ref_finds_the_error_of_every_variant() {
        let error = Error::from(http());
        assert_eq!(
            error.downcast_ref::<HttpError>().unwrap().status,
            reqwest::StatusCode::NOT_FOUND
        );
        assert!(error.downcast_ref::<OperationError>().is_none());
        let error = Error::from(OperationError::Deadline);
        assert!(matches!(
            error.downcast_ref::<OperationError>(),
            Some(OperationError::Deadline)
        ));
        let error =
            Error::Configuration(Box::new(std::io::Error::from(std::io::ErrorKind::NotFound)));
        assert_eq!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(Error::UnexpectedService("x".to_owned())
            .downcast_ref::<HttpError>()
            .is_none());
    }

    #[test]
    fn test_display_and_source_are_those_of_the_error() {
        let error = Error::from(http());
        assert_eq!(error.to_string(), http().to_string());
        assert!(std::error::Error::source(&error).is_none());
        let parse = url::Url::parse("https://example.org/")
            .unwrap()
            .join("http://[")
            .unwrap_err();
        let error = Error::from(EndpointError::Parse(parse));
        assert_eq!(
            std::error::Error::source(&error).unwrap().to_string(),
            parse.to_string()
        );
        assert_eq!(
            Error::UnexpectedService("not TES".to_owned()).to_string(),
            "not TES"
        );
    }
}
