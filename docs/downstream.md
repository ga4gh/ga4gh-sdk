# Build on this SDK

Services often extend a GA4GH API: they add endpoints, add fields to the
specification's models, or offer commands of their own. A downstream SDK for
such a service can build on this SDK instead of forking it. This guide walks
through [`examples/downstream-sdk/`](../examples/downstream-sdk/), a small
crate that does so through public APIs only. CI builds and tests it (job
`downstream`), so a change to the SDK that would break downstream crates
fails CI.

The example's extensions are made up: a TES service that offers
`GET /tasks/{id}/usage` with the resource usage of a task and a `priority`
field on tasks, and a separate service that offers `GET /quota`.

| File | Shows |
| --- | --- |
| [`src/usage.rs`](../examples/downstream-sdk/src/usage.rs) | A client for an additional endpoint that shares the SDK's `Transport` |
| [`src/task.rs`](../examples/downstream-sdk/src/task.rs) | A model that extends `TesTask` with a field |
| [`src/cli.rs`](../examples/downstream-sdk/src/cli.rs), [`src/main.rs`](../examples/downstream-sdk/src/main.rs) | A binary with its own name, version, configuration file and token variable that adds subcommands to the SDK's CLI |
| [`tests/downstream.rs`](../examples/downstream-sdk/tests/downstream.rs) | Tests of all three against a mock service |

## Dependencies

The library's package is `ga4gh-lib` (crate `ga4gh_sdk`), the CLI's
`ga4gh-cli` (crate `ga4gh_cli`). The example refers to them by path, because
it lives in this repository; a downstream crate uses Git dependencies, pinned
to a commit, because the SDK isn't on crates.io yet and its API may still
change (see [Versioning](../README.md#versioning)):

```toml
[dependencies]
ga4gh-lib = { git = "https://github.com/ga4gh/ga4gh-sdk", rev = "<commit>" }
# Only to extend the CLI
ga4gh-cli = { git = "https://github.com/ga4gh/ga4gh-sdk", rev = "<commit>" }
anyhow = "1"
clap = "4"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
url = "2"
```

Use the same major versions of `clap` and `serde` as the SDK: the CLI hands
out Clap 4 `Command`s, and the models implement Serde 1 traits. HTTP types
are re-exported as `ga4gh_sdk::reqwest`, so a crate that needs them doesn't
have to match the SDK's `reqwest` version.

## A client for an additional endpoint

[`UsageClient`](../examples/downstream-sdk/src/usage.rs) holds a
`Transport` and sends its requests through it:

```rust
pub async fn task_usage(&self, id: &str) -> Result<TaskUsage, Box<dyn Error>> {
    let path = format!("tasks/{}/usage", urlencode(id));
    let body = self.transport.get(&path, None).await?;
    Ok(serde_json::from_str(&body)?)
}
```

- `Transport::get`, `post`, `put` and `delete` take paths relative to the
  configured API root (`base_path`). Absolute URLs and paths that leave the
  root are rejected. `urlencode` encodes a path segment, such as an ID.
- `UsageClient::from_transport(tes.transport.clone())` shares the transport
  of an SDK client, such as `TES`. The new client then uses the same base
  path, user agent and HTTP client: timeouts, TLS settings and connection
  pool. `UsageClient::new(&config)` creates its own.
- For other methods, headers or bodies, `request_builder` returns a
  `reqwest::RequestBuilder` for a path beneath the root, and
  `request_response` returns the response with its status and headers; see
  [Advanced transport](../README.md#advanced-transport).
- An error status comes back as `ga4gh_sdk::utils::transport::HttpError`,
  with the status and the (bounded) body.
- A custom section in the configuration file (any name besides the built-in
  ones) can hold the service's settings; read it with
  `CliContext::load_config(ServiceType::Custom("MY_SERVICE".into()))` from
  the binary's own file (see [below](#a-cli-with-added-subcommands)), or
  with `Configuration::from_file`, which reads `ga4gh-cli`'s file, and find
  unknown keys in `Configuration::extra`. See the
  [usage guide](usage.md#configuration-file).

## A model that extends `TesTask`

[`PrioritizedTask`](../examples/downstream-sdk/src/task.rs) embeds the SDK's
`TesTask` with `#[serde(flatten)]` and adds its own field:

```rust
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PrioritizedTask {
    #[serde(flatten)]
    pub task: TesTask,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
}
```

The JSON stays one flat object. When reading, `priority` goes to the outer
field, the specification's fields to `task`, and any other fields to
`task.extra`, which the SDK's models keep and write back (see
[Models](usage.md#models)). To send and read such a model, the example uses
the TES client's JSON methods, which don't drop unknown fields:

- `TES::create_value` submits a task as a `serde_json::Value`, with the same
  checks as `TES::create`.
- `TES::get_raw` returns the task as the service sent it.

For a single extra field, `TesTask::extra` alone is enough; a typed wrapper
like this one is worth it when the downstream code reads or writes the field
often.

## A CLI with added subcommands

The example's binary, `downstream-cli`, has its own name, version,
configuration file and token variable. A `ga4gh_cli::CliContext` holds them;
[`cli::context`](../examples/downstream-sdk/src/cli.rs) creates it:

```rust
CliContext::new("downstream-cli", env!("CARGO_PKG_VERSION"))
    .with_about("CLI for GA4GH services, with service extensions")
    .with_token_env(
        "DOWNSTREAM_CLI_TOKEN",
        [ServiceType::TES, ServiceType::Custom("QUOTA".into())],
    )
```

| Setting | Default for `CliContext::new(name, …)` | `ga4gh-cli` (`CliContext::default()`) | Example |
| --- | --- | --- | --- |
| Name and `bin_name` | `name` | `ga4gh-cli` | `downstream-cli` |
| Version (`--version`) | as given | the CLI crate's version | the example crate's version |
| Configuration file | `~/.<name>/config.json`; `with_config_path` sets another | `~/.ga4gh-cli/config.json` | `~/.downstream-cli/config.json` |
| Token variable | none; `with_token_env` sets one and its sections | none | `DOWNSTREAM_CLI_TOKEN` for `TES` and `QUOTA` |
| Profile variable | none; `with_profile_env` sets one | `GA4GH_CLI_PROFILE`; `without_profile_env` turns it off | none |

The name, version and help text are `&'static str`, usually literals or
`env!` values. For the default configuration file, the name must be usable
as a directory name (not empty, `.` or `..`, and without `/` or `\`);
otherwise, set the path with `with_config_path`.

The token variable holds an OAuth access token for the sections listed with
it, typically the binary's own services. It applies only to listed sections
that set no credentials (`basic_auth`, `oauth_access_token` or
`oauth_access_token_env`); credentials in a section take precedence, and
sections that aren't listed never receive the token. An unset or empty
variable leaves the sections without a token. (An empty variable named by
`oauth_access_token_env` gives an empty token instead.) To send a token to
another service, name a variable in its section with
`oauth_access_token_env`. Like the configured tokens, it is sent only where
the SDK sends credentials (see [Not covered yet](#not-covered-yet)).

The configuration file can hold
[profiles](usage.md#profiles). The context's command tree has the global
option `--profile`; `dispatch` and `dispatch_to` apply it to built-in
commands. Otherwise, the profile variable selects the profile, if the
context has one, then the file's `default_profile`. For its own commands,
the example calls `context().with_profile_from(&matches)` before
`load_config`, so that `downstream-cli --profile metered quota` reads the
`QUOTA` section of the profile `metered`. The long name `--profile` is
reserved in the whole command tree: an option of a binary's own with that
long name, at the root or in a subcommand, fails Clap's checks. Arguments
merely named `profile`, such as a positional one, are fine; the option's id
is `ga4gh-cli-profile`.

[`cli::command`](../examples/downstream-sdk/src/cli.rs) starts from the
context's command tree and adds `tes usage <ID>` and a top-level `quota`:

```rust
// Abridged: the example also sets help texts.
context()
    .command()
    .mut_subcommand("tes", |tes| {
        tes.subcommand(Command::new("usage").arg(Arg::new("ID").required(true)))
    })
    .subcommand(Command::new("quota"))
```

`cli::run`, which the binary calls, handles its own commands and leaves
everything else to the SDK:

- For `tes usage`, it reads the `TES` section with
  `CliContext::load_config` and calls `cli::run_extension`, which takes the
  configuration as an argument and returns `None` for commands it doesn't
  own.
- For `quota`, it reads its own `QUOTA` section with
  `load_config(ServiceType::Custom("QUOTA".into()))` and calls `cli::quota`.
- For all other commands, it calls `CliContext::dispatch_to`, which runs a
  built-in command with its own service's section of the context's
  configuration file and returns `None` for commands it doesn't own.
  `ga4gh_cli::dispatch_with_config_to` does the same with a configuration
  that the caller supplies; that configuration must be for the command's
  service. The free functions `ga4gh_cli::command`, `dispatch` and
  `dispatch_to` use `CliContext::default()`, so they read `ga4gh-cli`'s
  file.
- They write to caller-owned writers, so tests can capture the output.
- `ga4gh_cli::CommandOutput` gives an added command the output and exit
  codes of the built-in ones: `CommandOutput::json` checks and escapes a
  JSON response, `CommandOutput::failure` escapes an error message and sets
  a failure exit code, `CommandOutput::success` takes text that is already
  escaped, and `write_to` writes both streams and returns the exit code.
  `cli::quota` uses them.
- `ga4gh_cli::printable` escapes control characters in values from the
  service, as the SDK's CLI does in its text output.

Run it from the repository root, with `~/.downstream-cli/config.json` in the
format described in the [usage guide](usage.md#configuration-file), with a
`TES` section and, for `quota`, a `QUOTA` section:

```sh
cargo run --manifest-path examples/downstream-sdk/Cargo.toml -- --version
cargo run --manifest-path examples/downstream-sdk/Cargo.toml -- tes usage <task-id>
cargo run --manifest-path examples/downstream-sdk/Cargo.toml -- tes list
cargo run --manifest-path examples/downstream-sdk/Cargo.toml -- quota
```

## Checks

`make downstream` runs what the CI job `downstream` runs: formatting, Clippy
(warnings are errors) and the tests of the example. The example has its own
Cargo workspace, so that the SDK's crates are built as dependencies, as in a
downstream repository; the workspace commands such as `make check` leave it
out. It has no lockfile of its own: the target copies the repository's
`Cargo.lock` into it, lets Cargo add the example's own entry, and fails if
any other package isn't locked at a version and source from the root
lockfile. The example thus builds with the SDK's dependency versions; a
dependency that only the example needs must be added to the workspace
first. Without
`make downstream`, Cargo resolves the example's dependencies afresh.

When you change a public API that the example uses, update the example and
this guide in the same pull request.

## Not covered yet

- Default clients don't send configured credentials yet
  ([#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)). A downstream client
  can supply an authenticated `reqwest::Client` through
  `Transport::with_client`.
- The example is Rust only. Building on the Python bindings is tracked in
  [#78](https://github.com/ga4gh/ga4gh-sdk/issues/78), with the other
  remaining steps for downstream SDKs.
