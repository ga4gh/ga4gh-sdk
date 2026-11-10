# GA4GH SDK
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](./LICENSE)

## Synopsis

A Generic SDK and CLI for GA4GH API services.

## Basic usage

Set `Configuration.base_path` to the API root, including the service's path
prefix. For example, use `https://example.org/ga4gh/tes/v1` for TES,
`https://example.org/ga4gh/drs/v1` for DRS, or
`https://example.org/ga4gh/wes/v1` for WES. A trailing slash is optional.
The SDK resolves `service-info`, `tasks`, `objects`, and `runs` below that root.
The API root must not contain a query string or fragment.

```rust
use ga4gh_sdk::clients::tes::TES;
use ga4gh_sdk::utils::configuration::Configuration;

let config = Configuration::new(
    url::Url::parse("https://example.org/ga4gh/tes/v1")?
);
let tes = TES::new(&config).await?;
```

### Migration notes

Configs that previously used only the host must include the service path prefix
shown above. `Transport::endpoint` now accepts only relative paths inside that
root and returns `EndpointError` for invalid paths or roots; callers should not
pass absolute URLs or `.` and `..` path segments. `tes::urlencode` and
`wes::urlencode` now encode a path segment (so a space becomes `%20`, not `+`);
use form encoding for query values. `Transport` now returns `HttpStatusError`
for non-2xx HTTP responses rather than `std::io::Error`.

The WES models follow WES 1.1.0 and use the schema names of the spec:
`WesRunRequest` is now `RunRequest`, `WesRunLog` is `RunLog`, `WesState` is
`State`, and so on (the `Wes` prefix is gone). `RunRequest::new` takes the
required `workflow_type`, `workflow_type_version` and `workflow_url`, which are
now `String`; `tags` and `workflow_engine_parameters` are string maps. Run list
items are `RunListResponseRunsInner` and deprecated task log items are
`RunLogTaskLogsInner`. `Run::list_tasks` and `Run::get_task` call the new task
endpoints. As for `TES`, the error type of `WES::service` is now
`Box<dyn Error + Send + Sync>`, so a `WES` client can move between threads.

## Installation

These instructions install the SDK from this source repository.

**Prerequisites:** [Rust](https://www.rust-lang.org/tools/install) via
`rustup` and Git. The repository pins its toolchain in
[`rust-toolchain.toml`](rust-toolchain.toml); `rustup` installs that version
on the first build.

Install the command-line tool `ga4gh-cli` into `~/.cargo/bin`:

```sh
git clone https://github.com/ga4gh/ga4gh-sdk.git
cd ga4gh-sdk
cargo install --path cli --locked
ga4gh-cli --help
```

To use the Rust library in your own project, add it as a Git dependency
(package `ga4gh-lib`, crate `ga4gh_sdk`):

```toml
[dependencies]
ga4gh-lib = { git = "https://github.com/ga4gh/ga4gh-sdk" }
```

For the Python bindings (built with [maturin](https://www.maturin.rs/)), see
the [Python README](python/README.md).

## Quick start

Install [Funnel](https://ohsu-comp-bio.github.io/funnel/download/) and Docker,
then start a local TES server in another terminal. The repository's integration
suite uses Funnel v0.11.14; Docker is needed to execute tasks.

```sh
funnel server run --Server.HTTPPort=8000
```

Point the CLI at that service by writing `~/.ga4gh-cli/config.json`.
`base_path` is the API root, including the service's path prefix:

```sh
mkdir -p ~/.ga4gh-cli
cat > ~/.ga4gh-cli/config.json <<'JSON'
{
  "TES": {
    "base_path": "http://localhost:8000/ga4gh/tes/v1"
  }
}
JSON
```

Submit the checked-in [task file](examples/tes-hello.json), which runs a
greeting in an Alpine container without external storage. Then check its status
and list tasks:

```sh
task_id=$(ga4gh-cli tes create examples/tes-hello.json)
ga4gh-cli tes status "$task_id"
ga4gh-cli tes list
```

Docker may download the Alpine image on the first run.

The [CLI README](cli/README.md) describes all CLI commands. The
[usage guide](docs/usage.md) covers configuration, authentication, and the
Rust library's `ServiceInfo`, `TES`, `WES`, `DRS`, `TRS`, and `ServiceRegistry`
clients. The CLI also has [Service Registry commands](cli/README.md#service-registry-commands).

## Advanced transport

Advanced transport callers can use `request_response` for status and headers
before parsing, then consume `reqwest::Response::chunk()` incrementally or call
`utils::transport::read_bounded` for lossless bytes. Dropping the response stops
consumption. The existing text helpers remain available and now bound success
bodies to 8 MiB and error bodies to 16 KiB while reading, including chunked bodies.
Use `request_text` with explicit `BodyLimits` for different bounds. A
`BodyLimitError` retains HTTP status even for an oversized error response.
`Transport::with_client` and `request_builder` support custom client budgets and
headers; HTTP types are re-exported as `ga4gh_sdk::reqwest`.

`clients::trs::responses` offers tools pages with all pagination headers, follows
the server's `next_page` URL within the same tools API root, and returns an
opaque `files_zip` response without extraction or text conversion. These helpers
compose with a configured `Transport` independently of the basic TRS client.

The default client uses a 10-second connection budget and 60-second request
budget. Use `with_timeouts` or inject a client to change them. For a multi-attempt
operation, `RequestOptions` holds one absolute deadline, a local cancellation
token, body bounds and an explicit replay classification. Retries are opt-in,
capped at 32 attempts, and use exponential backoff with jitter; 429/503 honor
delta/date `Retry-After` within that same deadline. Mark only protocol-defined
reads as `ReadOnly` (including appropriate lookup POSTs). A non-replayable
operation gets one attempt and reports lost-response ambiguity for reconciliation.
`send_once` also covers multipart headers/body and permits an explicit larger
upload budget. Dropping a future or canceling its token stops local I/O.

DRS `get_object_once` and `get_access_url_once` return `Resolution::Ready` or
`Pending` with a credential-free resume reference. The immediate convenience
methods return a downcastable `PendingResolution` error on 202. Opt-in
`resolve_object`/`resolve_access_url` honor integral-second `Retry-After`, one
absolute `Operation` budget, cancellation and an attempt bound; failures retain
the reference and last pending state. Zero delay becomes one second; missing,
malformed, overflow or greater-than-one-hour delays are protocol errors.
Authentication and malformed 200 responses remain errors, without pending retries.

`clients::drs::uri::DrsResolver` accepts hostname-based `drs://host/encoded-id`
references, decodes IDs once, and returns an explicit access plan (URL and
object-store headers). Configure an authority-to-`Transport` mapping for gateways
and per-service credentials; unmapped authorities get a fresh anonymous HTTPS
transport. Protocol/region preferences and an explicit method index control
selection. Direct URLs and access-ID negotiation share deferred resolution;
pending, unsupported/unavailable methods and local interruption stay distinct.
Compact identifier forms are explicitly unsupported in this MVP. It never
downloads object bytes or copies DRS credentials into an object-store request.

DRS `get_bulk_objects` and `get_bulk_access_urls` accept optional Passports and
one service/credential context per call. `BulkPolicy` bounds chunk size,
concurrency, total HTTP attempts and 413 split depth; an advertised
`maxBulkRequestLength` can lower the caller-selected chunk size. Missing metadata
stays unknown. Replies are correlated by object/access identity rather than
position, with successful siblings, grouped failures, duplicate/omitted identity
diagnostics and pending outcomes retained. Duplicate inputs collapse. The MVP
requires one access ID per object per call, so object-only grouped errors never
receive fabricated per-access attribution. A 202 retains pending keys and delay;
resume through the same bulk API with caller-supplied credentials/Passports.
There is no singular or anonymous fallback. Compatibility is verified against
protocol fixtures, without a claim of broad real-service interoperability.

## Service composition

`TES::discover` and `WES::discover` return a `Discovered<T>` wrapper with a
cached `CapabilitySnapshot`. One service-info request supplies both the client
and the snapshot; typed/raw accessors make no requests and `refresh()` is
explicit. Raw extensions are preserved. Preflight diagnostics distinguish
supported, unsupported and unknown/stale metadata. `create_validated` and
`run_validated` accept an explicit strict flag; ordinary submission stays
advisory. TES backend keys are case insensitive and storage is nonexhaustive.

`Task::wait` and `Run::wait` accept `WaitOptions` with an operation budget, local
cancellation and polling/retry settings. The result distinguishes successful
completion, executor/system failures, preemption, cancellation, polling failure
and observation interruption, retaining the last state and a credential-free
`RemoteHandle`. Rebuild observation with caller-supplied configuration. Dropping
or canceling observation never cancels remote work; `cancel_and_wait` is an
explicit separate action. Optional failure logs are fetched once.

TES/WES `pages` return a lazy `Pager` with `next_page` and `next_item` methods.
It preserves filters/view and opaque tokens, continues past empty intermediate
pages, detects token cycles, and bounds requests/items. Memory holds one page
plus fixed-size token digests bounded by `max_requests` (default 1000). Checkpoints bind
service, API root and query; a failed request retains the valid continuation.
At a partially consumed or truncated page, a checkpoint replays that page.
Mutable listings provide neither snapshot isolation nor exactly-once processing;
callers reconcile replays. Credentials are supplied separately when resuming.

The CLI streams `tes list --all --format jsonl` through caller-owned writers,
with one JSON object per stdout line. `--max-pages` and `--max-items` bound work;
limits or later failures preserve emitted records, report a resume checkpoint on
stderr and return a nonzero exit code. Reusable CLIs use `dispatch_to` or
`dispatch_with_config_to`; existing buffered dispatch remains available for
ordinary commands.

`WES::run_workflow_streaming` accepts `StreamingAttachment` sources as bytes,
files or owned async readers, with optional known lengths. Multipart content
follows backpressure and shares the normal transport header policy. Paths are
validated before I/O, nested relative filenames stay intact, and one absolute
operation budget covers preparation, upload and response. A consumed source
is never replayed; cancellation or a lost response after transmission reports
that submission may have been accepted. The existing bytes API remains usable.
## Building

**Developer note:** If you want to update any of the [OpenAPI](https://www.openapis.org/) specifications supported by this tool or extend it to support any other OpenAPI-backed services (e.g., not currently supported GA4GH API-based services or your own services), please refer to our [documentation for auto-generating OpenAPI models](docs/autogenerate_OAI_models.md).

To build the project:
```sh
cargo build --workspace --locked
```

## TRS client

The library includes a client for the
[GA4GH Tool Registry Service (TRS) API 2.0.1][trs], for registries such as
[WorkflowHub][workflowhub] or [Dockstore][dockstore]. Set
`Configuration.base_path` to the TRS API root, including its path prefix,
with or without a trailing slash, e.g. `https://workflowhub.eu/ga4gh/trs/v2`
or `https://dockstore.org/api/ga4gh/trs/v2`.

```rust
use ga4gh_sdk::clients::trs::models::DescriptorTypeWithPlain;
use ga4gh_sdk::clients::trs::{ToolFilter, TRS};
use ga4gh_sdk::utils::configuration::Configuration;

let config = Configuration::new(url::Url::parse("https://workflowhub.eu/ga4gh/trs/v2")?);
let trs = TRS::new(&config).await?; // fetches and checks service-info
let filter = ToolFilter {
    tool_class: Some("Workflow".into()),
    limit: Some(10),
    ..Default::default()
};
let tools = trs.list_tools(&filter).await?;
let descriptor = trs
    .get_descriptor("2", "1", DescriptorTypeWithPlain::PlainGalaxy)
    .await?;
```

`TRS` covers `service-info`, `toolClasses`, `tools` with all filters, single
tools, versions, descriptors (also by relative path), test files, file
listings and container files. Tool IDs, version IDs and relative paths are
percent-encoded as single path segments, so Dockstore IDs such as
`#workflow/github.com/org/repo` work as they are. For `PLAIN_*` descriptor
types, the bare file is returned as a `FileWrapper` with only `content` set;
`list_tests` then returns a list with that one wrapper (or the wrappers, if
the registry sends a JSON array of them). Dockstore sends the
service-info timestamps `createdAt` and `updatedAt` as numbers; the client
drops such non-string values instead of failing.
The basic `list_tools` method returns a page without headers. Use
`clients::trs::responses` with the client's transport for server-provided paging
URLs and opaque `format=zip` downloads (see the advanced transport API above).
Authentication isn't supported yet: the SDK's transport doesn't send
the configured tokens, so only public registries and tools work. The CLI has
no TRS commands yet.

## Testing
Unit tests run by default. Integration suites are opt-in.

To run the workspace's unit and documentation tests, use:

```sh
cargo test --workspace --locked
```

For opt-in integration suites, see the [integration test guide](tests/README.md)
for prerequisites, commands, and service-specific behavior.

Optional tools for running or measuring the default tests:

```sh
cargo nextest run
cargo llvm-cov nextest
```

## Versioning
The project adopts the [semver] scheme for versioning.
Currently the software is in a pre-release stage, so changes to the API,
including breaking changes, may occur at any time without further notice.

## Contributing

This project is a community effort and lives off your contributions, be it in
the form of bug reports, feature requests, discussions, ideas, fixes, or other
code changes. Please read [these guidelines][contributing] if you want to
contribute. And please mind the [code of conduct][code-of-conduct] for all
interactions with the community.

## License

This project is covered by the [Apache License 2.0][badge-license-url] also
[shipped with this repository][license].

## Contact
The project is maintained by [ELIXIR Cloud & AAI][elixir-cloud-aai], a Driver
Project of the [Global Alliance for Genomics and Health (GA4GH)][ga4gh], under
the umbrella of the [ELIXIR] [Compute Platform][elixir-compute].

To get in touch with us, please use one of the following routes:

- For filing bug reports, feature requests or other code-related issues, please
  make use of the project's [issue tracker][issue-tracker].
- For private/personal issues, more involved communication, or if you would like
  to join our team as a regular contributor, you can either join our
  [chat board][badge-chat-url] or [email] the community leaders.


[![logo-elixir]][elixir] [![logo-elixir-cloud-aai]][elixir-cloud-aai]

[badge-chat-url]: https://join.slack.com/t/elixir-cloud/shared_invite/enQtNzA3NTQ5Mzg2NjQ3LTZjZGI1OGQ5ZTRiOTRkY2ExMGUxNmQyODAxMDdjM2EyZDQ1YWM0ZGFjOTJhNzg5NjE0YmJiZTZhZDVhOWE4MWM
[badge-license-url]: http://www.apache.org/licenses/LICENSE-2.0
[code-of-conduct]: https://elixir-cloud-aai.github.io/about/code-of-conduct/
[contributing]: https://elixir-cloud-aai.github.io/guides/guide-contributor/
[dockstore]: https://dockstore.org/
[elixir]: https://elixir-europe.org/
[elixir-cloud-aai]: https://elixir-cloud.dcc.sib.swiss/
[elixir-compute]: https://elixir-europe.org/platforms/compute
[email]: mailto:cloud-service@elixir-europe.org
[ga4gh]: https://ga4gh.org/
[issue-tracker]: https://github.com/ga4gh/ga4gh-sdk/issues
[license]: LICENSE
[logo-elixir]: ./images/logo-elixir.svg
[logo-elixir-cloud-aai]: ./images/logo-elixir-cloud-aai.svg
[semver]: https://semver.org/
[trs]: https://ga4gh.github.io/tool-registry-service-schemas/
[workflowhub]: https://workflowhub.eu/
