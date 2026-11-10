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
  cargo install cargo-llvm-cov --locked
  ```

- For the hooks: [pre-commit](https://pre-commit.com/) 4, for example with
  `python3 -m pip install 'pre-commit>=4,<5'` or `pipx install pre-commit`.

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
| `make models` | Runs `utils/build_models_wrapper.sh` |
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
[typos](https://github.com/crate-ci/typos), and run `cargo fmt --all` and
`cargo clippy --workspace --all-targets --locked`. When a hook changes
files, the commit stops: review the changes, stage them, and commit again.

The generated models in `lib/src/clients/*/models/` are excluded from the
hooks, because their text comes from the upstream OpenAPI specifications. A
commit that only changes models therefore doesn't run the Rust hooks; run
`make lint` after regenerating models.

Run all hooks on all files with `make pre-commit`, and skip single hooks with
`SKIP`, for example `SKIP=cargo-clippy git commit`. Dependabot bumps the
pinned hook revisions.

## Coverage

`make coverage` runs the default tests with
[cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) and writes:

- `coverage/lcov.info`: the LCOV report, for editors and coverage services
- `coverage/summary.txt`: the summary table, which it also prints

The report leaves out the generated models. The opt-in integration suites are not
included. For an HTML report, run `cargo llvm-cov --workspace --html --open`.

## Lints

All workspace members share the lints in the `[workspace.lints]` table of
[`Cargo.toml`](../Cargo.toml) (`[lints] workspace = true` in each member):

- `missing_docs`: public items need documentation comments. It only warns
  for now, and CI does not fail on Clippy warnings: about 350 items lack
  documentation, 250 of them in the generated models. Document the items you
  add or change.

`cargo clippy --workspace --all-targets` shows the warnings.

## CI

The [`CI` workflow](../.github/workflows/ci.yml) runs on every pull request,
on pushes to `main` and on manual dispatch:

| Job | Name | What it does |
| --- | --- | --- |
| `lint` | Lint | `cargo fmt --all --check`, `cargo clippy` |
| `test` | Test | `cargo test --workspace --locked` |
| `build` | Build | Lockfile check, builds of library, CLI and Python wheel |
| `integration` | Integration (*suite*) | Integration suites when their files change |
| `coverage` | Coverage | `make coverage`; summary in the job summary, LCOV report as the `coverage` artifact |
| `pre-commit` | Pre-commit | All hooks except the Rust ones, which `lint` runs |
| `passed` | CI passed | Fails unless all jobs above succeeded |

`CI passed` is the check to require for merges. A new job must be added to
its `needs:` and to the list of expected jobs in its script.

## Issue and pull request templates

New issues offer forms for bug reports and feature requests
([`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE/)), with links for
questions, chat and reporting vulnerabilities. New pull requests start from
[the pull request template](../.github/pull_request_template.md): a summary
with the issues it closes, the verification, declined review findings, open
questions and a checklist.
