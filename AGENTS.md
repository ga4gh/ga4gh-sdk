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

`cargo test --workspace` runs the default unit and documentation tests. The
Funnel and Starter Kit integration suites are opt-in. Stable rustfmt also
formats the generated models; the `ignore` setting in `rustfmt.toml` requires
nightly.

## Opt-in integration suites

- **Funnel/TES:** Install the `funnel` executable and `curl`, then run
  `bash run-integration-tests.sh` from the repository root. The script starts
  Funnel on `localhost:8000` if it does not find a running process, waits for
  `/healthz`, and runs `cargo test --features integration_tests`.
- **Starter Kit WES:** Start a local Docker daemon, then run
  `tests/run-wes-docker-integration.sh` from the repository root. The script
  starts and cleans up a Starter Kit container and runs the WES integration
  test. It needs a working Docker socket and `python3`.

## Python bindings

For Python binding changes, use Python 3.12 or newer and a virtual environment.
From `python/`, install `maturin` and `pytest`, build the extension in development
mode, run its tests, and build a wheel:

```sh
cd python
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.7,<2' pytest
maturin develop
pytest
maturin build
```
