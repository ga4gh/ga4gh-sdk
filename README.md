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
