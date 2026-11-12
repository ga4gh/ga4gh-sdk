# Repository guide

This is a public Rust workspace with a library (`lib`), CLI (`cli`), and Python
bindings (`python`). Run the commands below from the repository root unless a
different directory is shown.

## Changes and pull requests

- Open pull requests against `main`.
- Before work in a fork, compare its `main` with upstream `main` and bring in
  upstream changes. Check again immediately before a release dry run. Preserve
  fork-only commits and tags; do not force-reset a fork used for testing.
- Use Conventional Commits for commit subjects, for example
  `docs: add contributor checks` or `fix(cli): handle missing config`.
- Never put secrets or private details in commits, issues, or pull requests.
- Treat every `models/` directory produced by `utils/build_models_wrapper.sh`
  as generated code, including versioned directories under `wire/`. Do not
  edit models by hand. Regenerate them with the wrapper and put hand
  edits into `utils/patches/<api>.patch`; see
  [the generation guide](docs/autogenerate_OAI_models.md). A clean run of the
  wrapper must not change `lib/src/clients`.
- When changing a public Rust, CLI, or Python API, consider the downstream
  extension use cases in [#78](https://github.com/ga4gh/ga4gh-sdk/issues/78).
  The example crate in `examples/downstream-sdk/` uses these APIs and has
  its own workspace, so `make check` doesn't build it: run `make downstream`,
  and update the example and [its guide](docs/downstream.md) with the API.
- For implementation work, use the routing table in the
  [engineering principles](docs/engineering_principles.md). Read only the
  sections that apply to the affected contract. Keep unrelated refactoring
  outside the task.
- Before reviewing or updating an existing PR, read the
  [reviewer framework](docs/reviewer_framework.md). For updates, follow its
  procedure for maintaining an existing PR.
- For documentation or instruction changes, review the affected prose using
  the [technical writing guide](docs/technical_writing.md#review-and-verification).
  Preserve technical meaning, conditions, exceptions, and API identifiers.
  This is a scoped editorial review, not a new CI gate. Leave unrelated prose
  alone; [#185](https://github.com/ga4gh/ga4gh-sdk/issues/185) evaluates the pilot.

## Core engineering checks

For a code change, use these reminders and read the applicable sections of the
[engineering principles](docs/engineering_principles.md) for the full rules:

- Identify the affected contract; trace a normal path and a failure path.
- Keep credentials and authenticated requests within the intended service boundary.
- Preserve HTTP status and cause for callers; keep secrets out of default diagnostics.
- Preserve data across parsing, models, and language conversions.
- Reject invalid configuration instead of silently choosing another value.
- Do not replay a non-idempotent request without proof that replay is safe.
- Test at the observable boundary, including downstream and Python paths when affected.
- Update affected documentation and examples with the behavior change.

## Before opening a pull request

Before creating a PR, including a draft, review the complete diff against its
intended base using [the reviewer framework](docs/reviewer_framework.md). Check
the linked issue, affected contracts, tests, documentation, and the actual CI
coverage. Run the relevant local checks first; CI results arrive after the PR
is opened.

Report the pre-PR review result in the chat. If it finds required work, explain
the findings, fix them, and repeat the affected checks and review. Do not open
the PR while a blocking defect, required verification, or material design
question remains unresolved. If the work cannot be resolved, report what is
left and stop before creating the PR. Nonblocking follow-ups do not prevent
publication; identify them separately.

Keep PR descriptions focused on the change and evidence specific to the issue.
Report routine check results in the chat review; list them in the PR only when
they add context that CI does not show. Do not list unrelated opt-in suites as
limitations. When updating someone else's PR, preserve accurate validation
evidence they supplied. Correct stale claims without replacing that evidence
with a generic list of workspace commands.

## Standard Rust checks

Install a stable Rust toolchain with `rustfmt` and `clippy`. Run all four checks
from the repository root before opening a pull request:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo build --workspace
cargo test --workspace
```

`make check` runs these four checks, with `--locked`. See
[the development guide](docs/development.md) for the other `make` targets,
the pre-commit hooks, coverage, lints, and the CI jobs.

`cargo test --workspace` runs the default unit and documentation tests.
Integration suites are opt-in. Stable rustfmt also formats the generated models.
Some default tests start mock HTTP servers on loopback, so the test environment
must allow local sockets.

## Opt-in integration suites

See [the integration test guide](tests/README.md) for the suite list,
prerequisites, and commands. Add new suites there.

## Python bindings

For changes under `python/`, and for SDK client or model changes that may affect
Python users, read [the Python agent guide](python/AGENTS.md). It routes to the
[Python bindings guide](python/README.md) for setup, supported API, build, and
test commands. Update those guides when the Python workflow or API changes.
