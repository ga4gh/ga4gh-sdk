# Developer guide

This guide is the starting point for developers who join the project. It
describes how the repository is laid out, how to set up a development
environment, and how to build, test and document a change. It links to the
detailed guides instead of repeating them:

| Guide | Contents |
| --- | --- |
| [Development](docs/development.md) | `make` targets, pre-commit hooks, coverage, lints, the documentation site and the CI jobs |
| [Integration suites](tests/README.md) | The opt-in suites against real services, with their prerequisites |
| [Python bindings](python/README.md) | Build, tests and supported API of the bindings |
| [Generated models](docs/autogenerate_OAI_models.md) | How the OpenAPI models are generated and patched |
| [Build on this SDK](docs/downstream.md) | Extending the library, CLI and models from another crate |
| [Repository guide](AGENTS.md) | Rules for changes and pull requests, for people and coding agents |
| [Engineering principles](docs/engineering_principles.md) | Properties that a change must preserve |
| [Reviewer framework](docs/reviewer_framework.md) | How pull requests are reviewed |
| [Releasing](docs/releasing.md) | How maintainers publish a release |

Run all commands in this guide from the repository root, unless a step
changes the directory.

## Repository layout

The repository is a Cargo workspace (root [`Cargo.toml`](Cargo.toml)) with
three members:

| Directory | Package | Crate | Contents |
| --- | --- | --- | --- |
| [`lib/`](lib/) | `ga4gh-lib` | `ga4gh_sdk` | The Rust library: clients, models, transport, configuration |
| [`cli/`](cli/) | `ga4gh-cli` | `ga4gh_cli` | The command-line tool `ga4gh-cli` and a library with its commands |
| [`python/`](python/) | `ga4gh` | `ga4gh` | The Python bindings (PyO3), published on PyPI as `ga4gh-sdk` with the import name `ga4gh_sdk` |

The code of each member is organized by service: one module per GA4GH API.

- **Library** ([`lib/src/`](lib/src/)):
  - `clients/<api>/` holds the client of one API: `drs`, `registry`,
    `serviceinfo`, `tes`, `trs` and `wes`. Its `mod.rs` defines the client
    (for example `TES` or `DRS`), and other files hold hand-written parts,
    such as `tes/view.rs` or `drs/uri.rs`.
  - `clients/<api>/models/` holds the models generated from the API's
    OpenAPI specification. TRS keeps them per version under
    `trs/wire/<version>/models/`. Never edit these folders by hand; see
    [Generated models](#generated-models).
  - `clients/mod.rs` defines `ServiceType`, the sections of the
    configuration file. Shared client code, such as pagination and waiting,
    lives next to it in `clients/`.
  - `utils/` holds the parts that all clients share: `Transport` (HTTP
    requests below the API root), `Configuration`, authentication and
    timeouts.
  - `workflow/` holds the experimental workflow contracts (feature
    `workflow-contracts`; see [the guide](docs/workflow-contracts.md)).
- **CLI** ([`cli/src/`](cli/src/)):
  - `main.rs` only sets up logging, calls the library and turns the result
    into an exit code.
  - `lib.rs` builds the command tree (`command()`) and dispatches the
    subcommands (`dispatch()`). Downstream binaries reuse both.
  - One module per service (`drs.rs`, `tes.rs`, `trs.rs`, `wes.rs`,
    `registry.rs`, `workflow.rs`). Each has
    `pub fn command() -> clap::Command` and an `async fn run(...)` that
    returns a `CommandOutput`.
  - `login.rs`, `render.rs` and `tes_files.rs` hold the OpenID Connect login,
    the text and JSON output and the TES file downloads.
- **Python bindings** ([`python/src/`](python/src/)):
  - `lib.rs` registers the classes of the Python module `ga4gh_sdk`.
  - `clients/` holds one binding per service. A binding with several classes
    is a folder, as `drs/`, `trs/` and `wes/` are: `mod.rs` with the module
    docs and `register()`, one file per topic such as `client.rs`, and
    `helpers.rs` for helpers that only this binding uses. Bindings that are
    still single files (`tes.rs`, `tes_async.rs`, `serviceinfo.rs`) move to
    this layout one API at a time. The
    [Python agent guide](python/AGENTS.md#lay-out-a-service-binding)
    describes the layout in detail.
  - `utils/` holds the helpers that several bindings share, such as
    `json.rs`, `validate.rs` and the shared Tokio runtime in `runtime.rs`.
  - [`ga4gh_sdk.pyi`](python/ga4gh_sdk.pyi) holds the type stubs, and
    `tests/` the pytest suite.

Other top-level folders:

| Path | Contents |
| --- | --- |
| [`utils/`](utils/) | Scripts, templates and patches that generate the models (`make models`) |
| [`tests/`](tests/) | Scripts and fixtures of the integration suites and the CLI image tests |
| [`examples/`](examples/) | Example crates and files, among them [`downstream-sdk/`](examples/downstream-sdk/), which CI builds |
| [`docs/`](docs/) | The guides; [`mkdocs.yml`](mkdocs.yml) lists the pages of the documentation site |
| [`.github/`](.github/) | CI and release workflows, issue forms, the pull request template, Dependabot |

## Set up a development environment

1. Install [rustup](https://rustup.rs/) and Git.
2. On Linux, install the OpenSSL development files and `pkg-config`, which
   the default TLS backend needs. On Debian and Ubuntu, run
   `sudo apt-get install libssl-dev pkg-config`; on Fedora and RHEL, install
   `openssl-devel` and `pkgconf-pkg-config`. A build of only the Python
   bindings with maturin doesn't need them, because the bindings use rustls.
3. Clone the repository and install the pinned toolchain:

   ```sh
   git clone https://github.com/ga4gh/ga4gh-sdk.git
   cd ga4gh-sdk
   rustup toolchain install
   ```

   [`rust-toolchain.toml`](rust-toolchain.toml) pins the Rust version with
   `rustfmt` and `clippy`.

4. Run the standard checks to confirm the setup:

   ```sh
   make check
   ```

   This runs `cargo fmt --check`, Clippy, the workspace build and the unit
   and documentation tests. Some tests start mock HTTP servers on the
   loopback interface, so the environment must allow local sockets.

5. Optionally, install the pre-commit hooks as described in
   [Development](docs/development.md#pre-commit-hooks).

For the Python bindings, you also need CPython 3.11 or newer; see
[Work on the Python bindings](#work-on-the-python-bindings).

## Build and run

Build the library, the CLI and the Python bindings' Rust crate:

```sh
make build
```

Run the CLI from the source tree, with arguments after `--`:

```sh
cargo run -p ga4gh-cli --locked -- --help
```

The [CLI guide](cli/README.md) describes the configuration file and the
commands. The [Quick start](README.md#quick-start) shows a local TES server
to try the CLI against.

## Tests

The tests have four levels. CI runs the first three on every pull request.
It runs the Funnel, proTES, WES Starter Kit and TRS-Filer suites only when
their files change; the other service suites run only by hand.

| Level | Where | Run with |
| --- | --- | --- |
| Unit and documentation tests | `#[cfg(test)]` modules in `src/`, code examples in doc comments | `make test` |
| Crate tests against mock services | `lib/tests/`, `cli/tests/` (with [mockito](https://docs.rs/mockito)) | `make test` |
| Python binding tests against mock services | `python/tests/` | `python -m pytest` in `python/` (see below) |
| Integration suites against real services | `tests/`, feature-gated files in `lib/tests/` and `cli/tests/` | One command per suite in [the integration test guide](tests/README.md) |

`make test` runs `cargo test --workspace --locked`. To run a single test
file, name its package and target, for example:

```sh
cargo test -p ga4gh-lib --locked --test pagination
```

The integration suites need services such as Funnel, proTES, the WES
Starter Kit or TRS-Filer, and usually Docker. Each is behind a Cargo feature
(for example `integration_tests` for Funnel) or marked `#[ignore]`, so
`make test` skips them. Add a new suite to
[the integration test guide](tests/README.md).

Two more checks protect users of the SDK:

- `make downstream` builds and tests the
  [downstream example](docs/downstream.md). Run it when you change a public
  Rust API.
- `tests/run-image-tests.sh` builds and tests the
  [CLI container image](docs/container-image.md); it needs Docker.

`make coverage` measures the coverage of the unit and crate tests; see
[Development](docs/development.md#coverage).

When you fix a bug, first add a test that reproduces it. Test at the
boundary that users see, as the
[engineering principles](docs/engineering_principles.md#prove-behavior-at-the-relevant-boundary)
explain.

## Lint and format

```sh
make fmt    # format all Rust code
make lint   # check the formatting and run Clippy with -D warnings
```

The pre-commit hooks also check file hygiene, YAML, TOML, the workflows
and spelling. Run them all with `make pre-commit`. See
[Development](docs/development.md#pre-commit-hooks) for the list.

## Work on the Python bindings

Build the bindings into a virtual environment and run their tests:

```sh
cd python
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.7,<2' pytest
maturin develop --locked
python -m pytest
```

Run `maturin develop --locked` again after each change to Rust code. The
bindings are written by hand: a new Rust method or model does not appear in
Python by itself. Read the [Python agent guide](python/AGENTS.md) before
changing them, and the [Python bindings guide](python/README.md) for the
supported API.

## Documentation

The guides are Markdown files next to the code they describe. The
documentation site on [Read the Docs](https://ga4gh-sdk.readthedocs.io/)
combines the pages listed in the `nav` of [`mkdocs.yml`](mkdocs.yml) with the
Rust API reference. Build it as CI does:

```sh
python3 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade 'pip>=25.1'
python -m pip install --group python/pyproject.toml:docs
python -m pytest docs/test_hooks.py
make site
```

`make site` fails on rustdoc warnings and on broken links. `make docs`
builds only the Rust API reference, in `target/doc/`. To add a page, list it
in the `nav` of `mkdocs.yml`; see
[Development](docs/development.md#documentation).

Update the affected guides, examples, CLI help and Python stubs in the same
pull request as a change of behavior. The
[technical writing guide](docs/technical_writing.md) offers advisory rules
for the text.

## Generated models

The models in `lib/src/clients/<api>/models/` (and
`trs/wire/<version>/models/`) are generated from pinned OpenAPI
specifications by `make models`, which runs
[`utils/build_models_wrapper.sh`](utils/build_models_wrapper.sh) with Docker
or a local OpenAPI Generator. The script replaces these folders on every run, so put hand edits into
`utils/patches/<api>.patch`. Run `make fmt` after regenerating. A clean run
must not change `lib/src/clients`. The
[generation guide](docs/autogenerate_OAI_models.md) has the details, also
for generating models for another crate.

## Add a client for another API

Follow the existing services. For an API named `<api>`:

1. **Models.** Add an entry for the pinned specification to `SPECS` in
   [`utils/build_models_wrapper.sh`](utils/build_models_wrapper.sh) and run
   `./utils/build_models_wrapper.sh <api>`, as the
   [generation guide](docs/autogenerate_OAI_models.md#usage) describes. The
   models land in `lib/src/clients/<api>/models/`.
2. **Library client.** Add `lib/src/clients/<api>/mod.rs` with the client
   and declare the module in `lib/src/clients/mod.rs`. Send requests through
   `Transport` (`utils/transport.rs`), with paths relative to the API root,
   and read the service info with `clients::serviceinfo`. If the service has
   its own section in the configuration file, add a variant to `ServiceType`.
   Test the client against a mock server in `lib/tests/`.
3. **CLI.** Add `cli/src/<api>.rs` with `command()` and `run()`, declare the
   module in `cli/src/lib.rs`, and add it to the command tree and to the
   dispatch there. Test it in `cli/tests/`, and document the commands in
   the [CLI guide](cli/README.md).
4. **Python bindings**, if Python should expose the API: add a folder
   `python/src/clients/<api>/` with the layout above, call its `register()`
   in `python/src/lib.rs`, and add the classes to `python/ga4gh_sdk.pyi` and
   the supported surface in the [Python bindings guide](python/README.md).
   `python/tests/test_stubs.py` checks that the stubs match the module.
5. **Documentation and checks.** Describe the client in the
   [README](README.md), run `make check`, `make downstream` and the Python
   tests, and add an integration suite if a test service is available.

Services that only extend a GA4GH API, or that are not GA4GH APIs, can also
live in a separate crate that builds on this SDK; see
[Build on this SDK](docs/downstream.md).

## Contribute a change

1. Find or open an [issue](https://github.com/ga4gh/ga4gh-sdk/issues) for
   the change.
2. Create a branch from `main`, in this repository or in your fork.
3. Make the change with tests and documentation. Use
   [Conventional Commits](https://www.conventionalcommits.org/) for commit
   messages, for example `fix(cli): handle missing config`.
4. Run `make check`, plus the checks for the parts you changed: the Python
   tests, `make downstream`, `make site` or an integration suite.
5. Review the complete diff against `main` with the
   [reviewer framework](docs/reviewer_framework.md#author-preflight).
6. Open a pull request against `main` and fill in the
   [template](.github/pull_request_template.md). Write `Closes #N` only if
   the pull request resolves the issue completely; otherwise write `Refs #N`.
7. Wait for the required check **CI passed**. The CI jobs are listed in
   [Development](docs/development.md#ci).

The [repository guide](AGENTS.md) lists the rules for changes and pull
requests in full. Please also follow the
[contributor guidelines](https://elixir-cloud-aai.github.io/guides/guide-contributor/)
and the
[code of conduct](https://elixir-cloud-aai.github.io/about/code-of-conduct/)
of ELIXIR Cloud & AAI.

## Releases

Maintainers release by pushing a version tag `v<version>` on `main`. The
release workflow publishes the Python package to PyPI, the CLI binaries to a
draft GitHub release and the CLI image to the GitHub Container Registry. The
[release guide](docs/releasing.md) describes each step and how to test the
workflow in a fork.

## Get help

- Ask questions on the
  [discussion board](https://github.com/ga4gh/ga4gh-sdk/discussions).
- Chat with the community in the
  [ELIXIR Cloud & AAI Slack workspace](https://join.slack.com/t/elixir-cloud/shared_invite/zt-4f6zdhi84-X4YBkJHxhYmVIHJkRfY2JQ).
- Report bugs and request features with the
  [issue forms](https://github.com/ga4gh/ga4gh-sdk/issues/new/choose).
- Report potential security problems by email to
  `security-notification@ga4gh.org`, not in public issues.
