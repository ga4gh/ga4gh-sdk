# Configured authentication

The SDK applies one configured credential source to requests under each
service's `base_path`. This applies to Rust service clients, Python bindings,
and built-in CLI service commands, including service discovery and TES/WES
submissions. Configure each service section separately. The library does not
log in, refresh a token, or infer a token from a global environment variable.
The CLI's `login` command can get tokens from an OpenID Connect provider with
the device flow, for sections with `oidc` settings, and refreshes them; see
[CLI login](#cli-login-with-openid-connect).

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

## CLI login with OpenID Connect

A CLI section with an `oidc` object (`issuer`, `client_id`, optional `scope`)
gets its token from the CLI's token store instead of the configuration file.
`ga4gh-cli login <SECTION>` fills the store through the OAuth 2.0 device
authorization grant ([RFC 8628](https://www.rfc-editor.org/rfc/rfc8628)) with
any provider that publishes a `device_authorization_endpoint` in its
[OpenID Connect discovery](https://openid.net/specs/openid-connect-discovery-1_0.html)
document. The client is public: no client secret is configured, sent, or
stored. The [CLI README](../cli/README.md#logging-in-with-openid-connect)
describes the settings and commands.

- `oidc` is one credential source: a section that also sets Basic, OAuth or
  generic bearer credentials is an error. A downstream binary's token
  variable, when set, takes the place of the login for its listed sections.
- The issuer and the discovered endpoints must use HTTPS (HTTP only on
  loopback addresses), and the discovery document must name the configured
  issuer. Requests to the provider don't follow redirects.
- The store, `tokens.json` next to the configuration file, is written with
  mode `0600` in a directory created with mode `0700`, and a store that other
  users can access is rejected. It is keyed by issuer and client ID.
  Processes lock `tokens.json.lock` while they change the store, and a
  refresh starts from the stored tokens, so that processes don't reuse a
  refresh token that another one has replaced. A refresh doesn't store a
  login again that `logout` has deleted.
- The CLI sends the access token as an OAuth bearer token through a
  [token provider](usage.md#token-providers), so it reaches only the
  section's API root, as other credentials do. It refreshes an expired token
  before the command and once after a `401`, and writes refreshed tokens back
  to the store. A refresh that fails before the command is reported with its
  reason; one that fails after a `401` is reported as a token-provider
  failure.
- Tokens are never printed. Errors from the provider are reported by their
  OAuth `error` code; other fields of the response are not shown. A rejected
  refresh token (`invalid_grant`) or a missing one asks the user to log in
  again.
- `logout` deletes the stored tokens but doesn't revoke them at the provider.
