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
plus token history bounded by `max_requests` (default 1000). Checkpoints bind
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

## Installation

TBA

## Building

**Developer note:** If you want to update any of the [OpenAPI](https://www.openapis.org/) specifications supported by this tool or extend it to support any other OpenAPI-backed services (e.g., not currently supported GA4GH API-based services or your own services), please refer to our [documentation for auto-generating OpenAPI models](docs/autogenerate_OAI_models.md).

To build the project:
```
cargo build
```

## Testing
Unit tests run by default. Integration suites are opt-in.

To run the unit tests, which are being tested with mock servers, you can simply run
```
cargo test
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
code changes. Please read [these guidelines][docs-contributing] if you want to
contribute. And please mind the [code of conduct][docs-coc] for all
interactions with the community.

## License

This project is covered by the [Apache License 2.0](badge-url-license) also
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
[elixir]: https://elixir-europe.org/
[elixir-cloud-aai]: https://elixir-cloud.dcc.sib.swiss/
[elixir-compute]: https://elixir-europe.org/platforms/compute
[email]: mailto:cloud-service@elixir-europe.org
[ga4gh]: https://ga4gh.org/
[license]: LICENSE
[logo-elixir]: ./images/logo-elixir.svg
[logo-elixir-cloud-aai]: ./images/logo-elixir-cloud-aai.svg
[semver]: https://semver.org/
