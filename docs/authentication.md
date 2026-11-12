# Configured authentication

The SDK applies one configured credential source to requests under each
service's `base_path`. This applies to Rust service clients, Python bindings,
and built-in CLI service commands, including service discovery and TES/WES
submissions. Configure each service section separately. The SDK does not log in,
refresh a token, or infer a token from a global environment variable.

## Choose one source

| Source | Configuration file | Rust | Python |
| --- | --- | --- | --- |
| HTTP Basic | `basic_auth`: `username` and optional `password` | `with_basic_auth(BasicAuth { ... })` | `set_basic_auth(username, password=None)` |
| OAuth bearer | `oauth_access_token` or `oauth_access_token_env` | `with_oauth_access_token(...)` or `with_oauth_access_token_from_env(name)` | `set_oauth_access_token(token)` |
| Generic bearer | `bearer_access_token` or `bearer_access_token_env` | `with_bearer_access_token(...)` or `with_bearer_access_token_from_env(name)` | `set_bearer_access_token(token)` |

An `_env` field is the **name** of an environment variable, not its token.
The file reader resolves it when it loads the section. An unset or non-Unicode
variable is an error. Set exactly one source; combining Basic, OAuth, generic
bearer, API key, or URL userinfo fails instead of choosing a precedence. Empty
or header-invalid bearer values and Basic usernames containing `:` fail before
the request. File configurations are checked when loaded; programmatic Rust
fields are checked at request time. In Python, call `clear_auth()` before
switching schemes. The file does not store token-provider callbacks.

`api_key` is currently unsupported: its placement and prefix are not defined
by a common GA4GH contract. Supplying one is an error, not an anonymous
request. Embedded `user:password@host` credentials in `base_path` are also
unsupported; use `basic_auth` instead. Keep configuration files private, for
example `chmod 600 ~/.ga4gh-cli/config.json`. Prefer an explicitly named
environment variable when a token should stay out of that file. Use HTTPS for
remote services: the SDK does not upgrade a configured `http` API root to TLS.

## Request boundary and failures

SDK-owned HTTP clients do not follow redirects automatically. A `3xx` is
returned as an HTTP status; the SDK never forwards configured credentials to
another origin, a downgraded scheme, or a path outside the configured API root
through an automatic redirect. It also does not replay a submission because of
a redirect or a `401`. A server `401` remains inspectable by its status. If a
submission loses its response after it starts, its acceptance may be uncertain;
check the service before resubmitting.

`Transport::with_client(config, client)` keeps the supplied client's
`Authorization` header, TLS settings, timeouts, and redirect policy. The
configured `user_agent`, when present, still sets the request's User-Agent.
When the configuration also has credentials, requests fail until the caller
explicitly chooses
`Transport::with_client_and_configured_auth(config, client)`. With that opt-in,
the caller owns the injected client's redirect policy and must prevent
credential forwarding and submission replay. `WES::from_transport` and the
other injected-transport constructors retain that policy for discovery and
subsequent requests. Requests made directly through `Transport::client()` do
not acquire SDK-configured credentials automatically.

For downstream Rust code, `Transport.client` is now private so replacement
cannot silently change the redirect policy of SDK-owned authenticated
requests. Use `Transport::with_client` at construction time and `client()` for
read-only access. `request_builder()` can now return an `AuthError` as well as
an endpoint error, so its return type is `Result<_, Box<dyn Error>>`; callers
with an explicit `EndpointError` annotation must adapt their error handling.

DRS object-store access plans have their own URLs and headers. The service
credential is not copied to those requests. Offline `workflow` commands do
not load a service configuration or contact a server; `workflow submit` uses
the same WES configuration and authentication path as `wes run`.

Errors name invalid or conflicting **sources**, not credential values.
`Configuration` debug output also redacts known credentials. A remote server
can echo a credential or other sensitive data in an HTTP error body. The
default `Display` and `Debug` output of `HttpError` therefore includes its
status but omits the body; CLI error output and Python exceptions use that
safe message. Rust callers can explicitly read the bounded `HttpError.body`
for diagnosis. It is raw, untrusted data and must not be logged or shown
without reviewing it. This rule also applies to bodies obtained through the
low-level raw-response API.

Dynamic token-provider authentication is tracked separately in
[#215](https://github.com/ga4gh/ga4gh-sdk/pull/215). Its integration with
static sources and the shared request/redirect policy remains tracked by
[#203](https://github.com/ga4gh/ga4gh-sdk/issues/203).
