# Development

This guide describes the tools for working on the SDK: the `Makefile`,
pre-commit hooks, coverage, lints, the CI checks, and the issue and pull
request templates. The rules for changes and the standard checks are in
[AGENTS.md](../AGENTS.md); run all commands from the repository root.

## Engineering and review guidance

[AGENTS.md](../AGENTS.md) routes contributors and coding agents to the
engineering principles, reviewer framework, and technical writing guide.
Read the sections relevant to the current task. The
[dated assessment](assessments/2026-11-10-engineering.md) is archived evidence;
linked issues own current remediation status.

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
| `make downstream` | Formatting, Clippy and tests of the [downstream example](downstream.md#checks) |
| `make docs` | `cargo doc --workspace --no-deps --locked` (in `target/doc/`) |
| `make site` | The documentation site in `site/` (see [Documentation](#documentation)) |
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
  --ignore-filename-regex 'lib/src/clients/[^/]+/(wire/[^/]+/)?models/'
```

In CI, the `codecov` job uploads `coverage/lcov.info` from the `coverage`
job to [Codecov](https://codecov.io/gh/ga4gh/ga4gh-sdk), which shows the
coverage of `main` in the README badge and, on pull requests, the change in
coverage as a comment and as the `codecov/project` and `codecov/patch`
statuses. [`codecov.yml`](../codecov.yml) makes these statuses
informational: they never fail, and `CI passed` remains the only required
check.

The upload authenticates with an OIDC token of the workflow run, so the
repository needs no Codecov secret; uploads from forks need no token. Uploads
need the Codecov GitHub app, which the ga4gh organization has installed for
this repository. Dependabot pull requests may get no OIDC token, and then no
Codecov report.
A failed upload doesn't fail the job: the job summary and the `coverage`
artifact of the `coverage` job are then still there, and the job log of
`codecov` shows the reason.

## Lints

`make clippy` checks all workspace targets. The CI lint job runs the same
command and checks formatting. Documentation lint enforcement remains in
[#87](https://github.com/ga4gh/ga4gh-sdk/issues/87); it needs a policy for
generated models and the existing undocumented public API before it can become
a useful gate.

## Documentation

The guides are Markdown files next to the code they describe: the
[README](../README.md), the guides in `docs/`, the
[CLI README](../cli/README.md), the [Python README](../python/README.md), the
[integration test guide](../tests/README.md) and the READMEs of the
examples. [Read the Docs](https://ga4gh-sdk.readthedocs.io/) publishes them
as one site, built with [MkDocs](https://www.mkdocs.org/) and the
[Material theme](https://squidfunk.github.io/mkdocs-material/), together with
the Rust API reference of `cargo doc` for the `ga4gh_sdk` and `ga4gh_cli`
crates. Once the Read the Docs project is configured as described below, it
serves `latest` from `main`, a version per tag, and pull request previews.

- [`mkdocs.yml`](../mkdocs.yml) configures the site. Its `nav` lists the
  pages by their paths in the repository; Markdown files that it doesn't
  list are not on the site.
- [`docs/hooks.py`](hooks.py) builds the site from those files where they
  are, so that relative links between them work on GitHub and on the site
  alike, and points links to other files that Git tracks, such as source
  files, to GitHub. Links to untracked files, such as build output, fail the
  build.
- [`.readthedocs.yaml`](../.readthedocs.yaml) configures the build on Read the
  Docs: the dependency group `docs` of
  [`python/pyproject.toml`](../python/pyproject.toml) (exact versions;
  Dependabot doesn't bump them, so update them by hand), rustup for the
  pinned toolchain, `cargo doc`, then MkDocs.

A new guide needs an entry in `nav`; link to it with its relative path, as
on GitHub. To build the site locally into `site/` as CI does, install the
tools (pip 25.1 or later) and run `make site`:

```sh
python3 -m pip install --group python/pyproject.toml:docs
make site
```

`make site` runs `cargo doc --no-deps --locked -p ga4gh-lib -p ga4gh-cli` and
`mkdocs build --strict`, which fails on warnings such as links to missing
files or anchors. `python3 -m pytest docs/test_hooks.py` tests the hooks; CI
runs both. After the first `make site`, `mkdocs serve` shows the site
at <http://127.0.0.1:8000/> and rebuilds it when a guide changes.

### Read the Docs project

The project on Read the Docs is set up once, by a maintainer:

1. Import `ga4gh/ga4gh-sdk` on [readthedocs.org](https://readthedocs.org/)
   (project slug `ga4gh-sdk`) and keep `main` as the default branch, so that
   `latest` follows it.
2. Under **Settings > Pull request builds**, turn on builds of pull
   requests, for the previews.
3. Under **Automation rules**, add a rule that activates new versions of
   tags (for example, matching SemVer versions), so that every release gets
   its version.

After a release, check that its version was built.

## CI

The [`CI` workflow](../.github/workflows/ci.yml) runs on every pull request,
on pushes to `main` and on manual dispatch:

| Job | Name | What it does |
| --- | --- | --- |
| `lint` | Lint | `cargo fmt --all --check`, `cargo clippy` |
| `test` | Test | `cargo test --workspace --locked` |
| `build` | Build | Lockfile check, standalone library and CLI builds |
| `python` | Python tests | Builds and installs the wheel, runs `pytest` in `python/` |
| `wheels` | Python wheels | Full platform wheel and sdist matrix on `main` pushes and manual runs; skipped on PRs |
| `integration` | Integration (*suite*) | Integration suites when their files change |
| `coverage` | Coverage | `make coverage`; summary in the job summary, LCOV report as the `coverage` artifact |
| `codecov` | Upload coverage | Uploads the `coverage` artifact to Codecov; upload errors don't fail it |
| `pre-commit` | Pre-commit | All hooks except the Rust ones, which `lint` runs |
| `docs` | Docs | `make site`: the Rust API reference and the strict MkDocs build |
| `downstream` | Downstream example | `make downstream`: builds and tests [`examples/downstream-sdk/`](downstream.md) |
| `passed` | CI passed | Requires all applicable jobs to succeed; accepts the intentionally skipped wheel matrix on PRs |

`CI passed` is the check to require for merges. A new job must be added to
its `needs:` and to the list of expected jobs in its script. Ordinary PRs run
the Python binding tests, including a local wheel build, but do not run the
five-platform wheel and sdist matrix. Every push to `main` runs that matrix;
version tags also build it as part of the [release workflow](releasing.md).
For a PR that changes packaging, native dependencies, supported platforms or
the wheel/release workflows, open **Actions → CI → Run workflow**, select the
PR branch and run the full matrix before merging. Check the wheel and sdist
jobs. The manual run builds artifacts but does not publish them.

## Issue and pull request templates

New issues offer forms for bug reports and feature requests
([`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE/)), with links for
questions, chat and reporting vulnerabilities. New pull requests start from
[the pull request template](../.github/pull_request_template.md): a summary,
related issues when applicable, issue-specific verification, and optional
review notes. Use `Closes #N` only when merging the PR will fully satisfy the
issue; otherwise use `Refs #N`.
