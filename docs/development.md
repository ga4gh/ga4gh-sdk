# Development

This guide describes the tools for working on the SDK: the `Makefile`,
pre-commit hooks, coverage, lints, the CI checks, and the issue and pull
request templates. The rules for changes and the standard checks are in
[AGENTS.md](../AGENTS.md); run all commands from the repository root.

## Prerequisites

- [rustup](https://rustup.rs/), for the toolchain pinned in
  [`rust-toolchain.toml`](../rust-toolchain.toml) with `rustfmt` and
  `clippy`; `rustup toolchain install` installs it.
- GNU Make, to use the targets below.
- For coverage: the LLVM tools and
  [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov):

  ```sh
  rustup component add llvm-tools-preview
  cargo install cargo-llvm-cov --locked --version 0.9.1
  ```

- For the hooks: [pre-commit](https://pre-commit.com/), for example with
  `python3 -m pip install 'pre-commit==4.7.0'` or `pipx install pre-commit`.

CI uses the versions above (cargo-llvm-cov 0.9.1, pre-commit 4.7.0). They
are pinned in [`ci.yml`](../.github/workflows/ci.yml) and Dependabot does not
bump them, so update this guide when you change them.

## Make targets

`make` (or `make help`) lists the targets:

| Target | Command |
| --- | --- |
| `make build` | `cargo build --workspace --locked` |
| `make test` | `cargo test --workspace --locked` (unit and documentation tests) |
| `make fmt` | `cargo fmt --all` |
| `make fmt-check` | `cargo fmt --all --check` |
| `make clippy` | `cargo clippy --workspace --all-targets --locked` |
| `make lint` | `fmt-check` and `clippy` |
| `make check` | `lint`, `build` and `test`: the standard checks |
| `make coverage` | Coverage report in `coverage/` (see below) |
| `make clean-coverage` | Removes coverage data and `coverage/` |
| `make docs` | `cargo doc --workspace --no-deps --locked` (in `target/doc/`) |
| `make models` | Runs `utils/build_models_wrapper.sh`; run `make fmt` afterwards |
| `make pre-commit` | `pre-commit run --all-files` |

`make models` regenerates the OpenAPI models as described in
[the generation guide](autogenerate_OAI_models.md). The integration suites
and the Python bindings have their own guides:
[integration tests](../tests/README.md) and
[Python bindings](../python/README.md).

## Pre-commit hooks

[`.pre-commit-config.yaml`](../.pre-commit-config.yaml) defines checks that
run on every commit once you install them:

```sh
pre-commit install
```

The hooks check and fix file hygiene (trailing whitespace, final newlines,
merge conflict markers, large files, executable bits), validate YAML and
TOML, check the GitHub issue forms, workflows and Dependabot configuration
against their schemas, lint the workflows with
[actionlint](https://github.com/rhysd/actionlint), fix spelling with
[typos](https://github.com/crate-ci/typos) (intended words that it flags go
into [`_typos.toml`](../_typos.toml)), and run `cargo fmt --all` and
`cargo clippy --workspace --all-targets --locked`. When a hook changes
files, the commit stops: review the changes, stage them, and commit again.

The formatting and spelling hooks skip generated models in
`lib/src/clients/*/models/`, because their text comes from the upstream
OpenAPI specifications, and their patches in `utils/patches/`, whose
whitespace must match the generated files exactly. Structural checks still
scan those files for merge markers and oversized files. A change to generated
Rust models triggers the local Rust hooks, which format and lint the whole
workspace. After regenerating models, run `make fmt` and `make lint` before
committing.

Run all hooks on all files with `make pre-commit`, and skip single hooks with
`SKIP`, for example `SKIP=cargo-clippy git commit`. Dependabot bumps the
pinned hook revisions.

## Coverage

`make coverage` runs the unit tests with
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) and writes:

- `coverage/lcov.info`: the LCOV report, for editors and coverage services
- `coverage/summary.txt`: the summary table, which it also prints

The report leaves out the generated models. Documentation tests (which
cargo-llvm-cov only measures on nightly Rust) and the opt-in integration
suites are not included. For an HTML report of the same data, run this after
`make coverage`:

```sh
cargo llvm-cov report --html --open \
  --ignore-filename-regex 'lib/src/clients/[^/]+/models/'
```

In CI, the `codecov` job uploads `coverage/lcov.info` from the `coverage`
job to [Codecov](https://codecov.io/gh/ga4gh/ga4gh-sdk), which shows the
coverage of `main` in the README badge and, on pull requests, the change in
coverage as a comment and as the `codecov/project` and `codecov/patch`
statuses. [`codecov.yml`](../codecov.yml) makes these statuses
informational: they never fail, and `CI passed` remains the only required
check.

The upload authenticates with an OIDC token of the workflow run, so the
repository needs no Codecov secret; uploads from forks need no token. This
needs a one-time setup by an organization admin: the Codecov GitHub app
installed for the repository, and the repository enabled on Codecov.
Dependabot pull requests may get no OIDC token, and then no Codecov report.
A failed upload doesn't fail the job: the job summary and the `coverage`
artifact of the `coverage` job are then still there, and the job log of
`codecov` shows the reason.

## Lints

`make clippy` checks all workspace targets. The CI lint job runs the same
command and checks formatting. Documentation lint enforcement remains in
[#87](https://github.com/ga4gh/ga4gh-sdk/issues/87); it needs a policy for
generated models and the existing undocumented public API before it can become
a useful gate.

## CI

The [`CI` workflow](../.github/workflows/ci.yml) runs on every pull request,
on pushes to `main` and on manual dispatch:

| Job | Name | What it does |
| --- | --- | --- |
| `lint` | Lint | `cargo fmt --all --check`, `cargo clippy` |
| `test` | Test | `cargo test --workspace --locked` |
| `build` | Build | Lockfile check, builds of library, CLI and Python wheel |
| `python` | Python tests | Builds and installs the wheel, runs `pytest` in `python/` |
| `integration` | Integration (*suite*) | Integration suites when their files change |
| `coverage` | Coverage | `make coverage`; summary in the job summary, LCOV report as the `coverage` artifact |
| `codecov` | Upload coverage | Uploads the `coverage` artifact to Codecov; upload errors don't fail it |
| `pre-commit` | Pre-commit | All hooks except the Rust ones, which `lint` runs |
| `passed` | CI passed | Fails unless all jobs above succeeded |

`CI passed` is the check to require for merges. A new job must be added to
its `needs:` and to the list of expected jobs in its script.

## Issue and pull request templates

New issues offer forms for bug reports and feature requests
([`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE/)), with links for
questions, chat and reporting vulnerabilities. New pull requests start from
[the pull request template](../.github/pull_request_template.md): a summary,
related issues when applicable, issue-specific verification, and optional
review notes. Use `Closes #N` only when merging the PR will fully satisfy the
issue; otherwise use `Refs #N`.
