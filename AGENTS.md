# Repository guide

This is a public Rust workspace with a library (`lib`), CLI (`cli`), and Python
bindings (`python`). Run the commands below from the repository root unless a
different directory is shown.

## Changes and pull requests

- Open pull requests against `main`.
- Use Conventional Commits for commit subjects, for example
  `docs: add contributor checks` or `fix(cli): handle missing config`.
- Never put secrets or private details in commits, issues, or pull requests.
- Treat `lib/src/clients/*/models/` as generated code. Do not edit models by
  hand. See [the generation guide](docs/autogenerate_OAI_models.md) and
  [#85](https://github.com/ga4gh/ga4gh-sdk/issues/85) before changing them;
  #85 tracks repeatable regeneration and preservation of existing adaptations.
  Running the formatter over generated files is expected.
- When changing a public Rust, CLI, or Python API, consider the downstream
  extension use cases in [#78](https://github.com/ga4gh/ga4gh-sdk/issues/78).

## Standard Rust checks

Install a stable Rust toolchain with `rustfmt` and `clippy`. Run all four checks
from the repository root before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo build --workspace
cargo test --workspace
```

`cargo test --workspace` runs the default unit and documentation tests.
Integration suites are opt-in. Stable rustfmt also formats the generated models;
the `ignore` setting in `rustfmt.toml` requires nightly. Some default tests
start mock HTTP servers on loopback, so the test environment must allow local
sockets.

## Opt-in integration suites

See [the integration test guide](tests/README.md) for the suite list,
prerequisites, and commands. Add new suites there.

## Python bindings

For Python binding changes, use Python 3.12 and a virtual environment. Starting
at the repository root, install `maturin` and `pytest`, build the extension in
development mode, run its tests, and build a wheel:

```sh
cd python
python3.12 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.7,<2' pytest
maturin develop
pytest
maturin build
```

Standalone Python builds currently fail because the library does not enable
`serde`'s derive feature. This is tracked in
[#79](https://github.com/ga4gh/ga4gh-sdk/issues/79); a passing workspace build
does not establish that the wheel builds.
