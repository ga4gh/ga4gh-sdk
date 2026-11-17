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
| `make clippy` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
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
`cargo clippy --workspace --all-targets --locked -- -D warnings`. When a
hook changes files, the commit stops: review the changes, stage them, and
commit again.

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

`make clippy` checks all workspace targets and fails on any Clippy warning
(`-- -D warnings`). The CI lint job runs the same command and checks
formatting. Fix a new warning in the code where you can. Otherwise allow the
lint as narrowly as possible, with a comment that says why. Allows in
hand-written code sit on single items; two wider allows cover code that isn't
hand-written:

- Generated models: `clippy::derivable_impls` (the generator writes
  `Default` for enums by hand) is allowed on the `models` module of the DRS,
  TES, TRS and WES clients, and `clippy::too_many_arguments` (`new` takes
  one argument per required field) on that of WES, so that the models stay
  as the generator writes them (see
  [Autogenerate OpenAPI models](autogenerate_OAI_models.md)). A new client
  whose models trigger these lints needs the same attribute.

`ga4gh-lib` and `ga4gh-cli` deny public items without a doc comment
(`#![deny(missing_docs)]`), so every build fails on them, not only
`make clippy` and CI. Document every new public item. Items behind the
`workflow-contracts` feature are checked only in builds that enable it,
such as workspace builds (the CLI enables it). The generated `models`
modules are exempt, as the generator writes their docs from the OpenAPI
specifications: the `models` module of every client carries
`#[allow(missing_docs)]`, together with the Clippy allows above where
those apply.

## Documentation

The guides are Markdown files next to the code they describe: the
[README](../README.md), the guides in `docs/`, the
[CLI README](../cli/README.md), the [Python README](../python/README.md), the
[integration test guide](../tests/README.md) and the READMEs of the
examples. [Read the Docs](https://ga4gh-sdk.readthedocs.io/) publishes them
as one site, built with [MkDocs](https://www.mkdocs.org/) and the
[Material theme](https://squidfunk.github.io/mkdocs-material/), together with
the Rust API reference of `cargo doc` for the `ga4gh_sdk` and `ga4gh_cli`
crates and the [Python API reference](../python/api.md), which
[mkdocstrings](https://mkdocstrings.github.io/python/) renders from the
docstrings of the type stubs (`python/ga4gh_sdk.pyi`). Once the Read the
Docs project is configured as described below, it serves `latest` from
`main`, a version per tag, and pull request previews.

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

`make site` runs two strict builds:

- `cargo doc --no-deps --locked -p ga4gh-lib -p ga4gh-cli` with
  `RUSTDOCFLAGS="-D warnings"`, so that any unsuppressed rustdoc warning fails it, such as
  a bare URL (write `<https://...>` or `[text](https://...)`) or an
  unresolved link. `make docs` builds the whole workspace and stays lenient.
  Fixes in the docs of generated models go into their patch in
  `utils/patches/` (see
  [Autogenerate OpenAPI models](autogenerate_OAI_models.md)).
- `mkdocs build --strict`, which fails on warnings such as links to missing
  files or anchors. It also fails if the Python API reference can't be
  generated or Griffe warns about a docstring of the stubs, for example one
  that documents a parameter that the signature lacks. The stubs' docstrings
  use the Google style (`Args:`, `Returns:`, `Raises:`). The hooks copy
  `python/ga4gh_sdk.pyi` into the stub-only package
  `target/python-stubs/ga4gh_sdk/`, where Griffe reads it, so the build
  needs no compiled bindings.

`python/tests/test_stubs.py`, which the `Python tests` job runs, checks that
every class and member of the stubs has a docstring and that the `Args:`
section of each method names all of its parameters.

The Griffe extension `docs/griffe_overloads.py` renders methods that the
stubs declare only with `@overload`, such as `Task.wait`.
`python3 -m pytest docs` tests the hooks and checks that the reference loads
every class and member of the stubs; CI runs both. After the first
`make site`, `mkdocs serve` shows the site at <http://127.0.0.1:8000/> and
rebuilds it when a guide changes.

### Read the Docs project

The project on Read the Docs is set up once, by a maintainer:

1. Import `ga4gh/ga4gh-sdk` on [readthedocs.org](https://readthedocs.org/)
   (project slug `ga4gh-sdk`) and keep `main` as the default branch, so that
   `latest` follows it. Prefer a connected repository through the
   [Read the Docs GitHub App](https://docs.readthedocs.com/platform/stable/reference/git-integration.html#github-app).
   If the project was imported manually, create a **GitHub incoming webhook**
   under **Settings > Integrations**. In the GitHub repository's
   **Settings > Webhooks**, add the displayed URL and secret with the
   **Pushes**, **Pull requests**, **Branch or tag creation**, and
   **Branch or tag deletion** events. Without a connected repository or this
   webhook, Read the Docs receives no changes from GitHub.
2. Under **Settings > Pull request builds**, turn on builds of pull
   requests, for the previews. Push a new commit to any PR that was already
   open when previews were enabled to trigger its first build.
3. Under **Automation rules**, add a rule that activates new versions of
   tags (for example, matching SemVer versions), so that every release gets
   its version.

After a release, check that its version was built.

## CI

The [`CI` workflow](../.github/workflows/ci.yml) runs on every pull request,
on pushes to `main` and on manual dispatch:

| Job | Name | What it does |
| --- | --- | --- |
| `lint` | Lint | `cargo fmt --all --check`, `cargo clippy` with `-D warnings` |
| `test` | Test | `cargo test --workspace --locked` |
| `build` | Build | Lockfile check, standalone library and CLI builds |
| `python` | Python tests | Builds and installs the wheel, runs `pytest` in `python/` |
| `wheels` | Python wheels | Full platform wheel and sdist matrix on `main` pushes and manual runs; skipped on PRs |
| `binaries` | CLI binaries | Builds and runs `ga4gh-cli` on the six release platforms on `main` pushes and manual runs; skipped on PRs |
| `image` | CLI image | Builds the [CLI image](container-image.md) for `linux/amd64` and `linux/arm64` and runs `tests/run-image-tests.sh` (with Funnel on `linux/amd64`); runs on PRs too |
| `integration` | Integration (*suite*) | Integration suites when their files change |
| `coverage` | Coverage | `make coverage`; summary in the job summary, LCOV report as the `coverage` artifact |
| `codecov` | Upload coverage | Uploads the `coverage` artifact to Codecov; upload errors don't fail it |
| `pre-commit` | Pre-commit | All hooks except the Rust ones, which `lint` runs |
| `docs` | Docs | `make site`: the Rust API reference, without rustdoc warnings, and the strict MkDocs build |
| `downstream` | Downstream example | `make downstream`: builds and tests [`examples/downstream-sdk/`](downstream.md) |
| `passed` | CI passed | Requires all applicable jobs to succeed; accepts the intentionally skipped wheel and CLI binary matrices on PRs |

`CI passed` is the check to require for merges. A new job must be added to
its `needs:` and to the list of expected jobs in its script. Ordinary PRs run
the Python binding tests, including a local wheel build, but do not run the
five-platform wheel and sdist matrix or the six-platform CLI binary matrix.
Every pull request also builds and tests the CLI image on both supported
platforms. Every push to `main` runs the wheel, binary and image matrices;
version tags build them as part of the [release workflow](releasing.md).
For a PR that changes packaging, native dependencies, supported platforms or the wheel, binary or release
workflows, open **Actions → CI → Run workflow**, select the PR branch and run
the full matrices before merging. Check the wheel, sdist and CLI binary jobs.
The manual run builds artifacts but does not publish them.

## Issue and pull request templates

New issues offer forms for bug reports and feature requests
([`.github/ISSUE_TEMPLATE/`](../.github/ISSUE_TEMPLATE/)), with links for
questions, chat and reporting vulnerabilities. New pull requests start from
[the pull request template](../.github/pull_request_template.md): a summary,
related issues when applicable, issue-specific verification, and optional
review notes. Use `Closes #N` only when merging the PR will fully satisfy the
issue; otherwise use `Refs #N`.
