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
| Dynamic bearer | Not available in the file or CLI | `with_token_provider(...)` | `set_token_provider(provider, audience=None)` |

An `_env` field is the **name** of an environment variable, not its token.
The file reader resolves it when it loads the section. An unset or non-Unicode
variable is an error. Set exactly one source; combining Basic, OAuth, generic
bearer, dynamic provider, API key, or URL userinfo fails instead of choosing a
precedence. Empty or header-invalid bearer values, Basic usernames containing
`:`, and Basic usernames or passwords containing
[HTTP control characters](https://www.rfc-editor.org/rfc/rfc7617.html#section-2)
fail before the request. File configurations are checked when loaded;
programmatic Rust fields are checked at request time. In Python, call
`clear_auth()` before switching schemes. The file does not store token-provider
callbacks.

`api_key` is currently unsupported: its placement and prefix are not defined
by a common GA4GH contract. Supplying one is an error, not an anonymous
request. Embedded `user:password@host` credentials in `base_path` are also
unsupported; use `basic_auth` instead. Keep configuration files private, for
example `chmod 600 ~/.ga4gh-cli/config.json`. Prefer an explicitly named
environment variable when a token should stay out of that file. Use HTTPS for
remote services: the SDK warns once per process when preparing a credential
for a non-loopback `http` destination, but does not upgrade it to TLS.

## Request boundary and failures

SDK-owned HTTP clients do not follow redirects automatically. A `3xx` is
returned as an HTTP status; the SDK never forwards configured credentials to
another origin, a downgraded scheme, or a path outside the configured API root
through an automatic redirect. It also does not replay a submission because of
a redirect. With a provider, a read-only request may be sent once more after a
`401` with a fresh token; a submission is never replayed. An unrecovered
server `401` remains inspectable by its status. If a
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
With a token provider, this synchronous builder returns an error; use
`authorized_request_builder()` to obtain a request carrying a provider token.

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

The [token-provider guide](usage.md#token-providers) describes reuse,
audiences, 401 handling, and the distinction between provider failures and
HTTP errors. The provider's own error and a caller-selected audience can
contain sensitive data; default `TokenProviderError` diagnostics omit both.
Rust callers may explicitly inspect its underlying source error.
