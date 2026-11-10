# Integration suites and test fixtures

The default `cargo test` does not enable the integration suites below. Run
their commands from the repository root. They need local HTTP access and the
services described in each section.

## Funnel/TES

Install the `funnel` executable using the [Funnel developer guide](https://ohsu-comp-bio.github.io/funnel/docs/development/developers/)
and make sure `curl` is available. The suite uses `localhost:8000`.

```sh
bash run-integration-tests.sh
```

The script starts Funnel if its `/healthz` endpoint is not healthy, waits for
that endpoint, then runs the `integration_tests` feature suite.
It waits up to 60 seconds by default; set `FUNNEL_STARTUP_TIMEOUT_SECONDS`
to change the limit. It does not stop a Funnel process that it starts.

## Starter Kit WES

This suite needs a running Docker daemon with a usable Unix socket, plus
`python3`, `git`, and `curl`.

```sh
tests/run-wes-docker-integration.sh
```

The script starts `ga4gh/ga4gh-starter-kit-wes:0.2.0-nextflow` with temporary
configuration, an initialized SQLite database, a shared work directory, and
the Docker socket. It waits up to 180 seconds for
`/ga4gh/wes/v1/service-info`, runs the SDK test, and cleans up the container
and temporary files. On failure it prints server logs. Increase
`WES_STARTUP_TIMEOUT_SECONDS` or `WES_RUN_TIMEOUT_SECONDS` on slower hosts.
The [CI workflow](../.github/workflows/ci.yml) runs this suite on an Ubuntu
runner when WES, shared library, dependency, or integration files change. It
runs both integration suites on manual dispatch. The `CI passed` job checks
their results alongside lint, unit tests, and builds.

Starter Kit WES 0.2.0 only accepts a GitHub-shaped Nextflow project URL; its
`workflow_attachment` argument is disabled. The script places the checked-in
[Nextflow fixture](fixtures/wes-nextflow/main.nf) in Nextflow's local Git
project cache at a deterministic commit, so the workflow runs without fetching
a remote project. This server returns a fixed run list instead of recently
submitted runs, and its cancellation endpoint does not stop a run. The test
accounts for both limitations and does not claim that a run was canceled.

## Verified TRS CWL bundle with Toil WES

The bundle unit tests use a local TRS fixture and a capturing mock WES to compare
every staged/uploaded byte and nested relative filename. They also cover missing
imports, external/dynamic dependencies, unsafe/colliding paths, checksum mismatch,
explicit URL fetch policy and content changing behind the same TRS version.

For a real execution, use Python 3.12+ and Rust on PATH, with network access to
install the pinned Toil 9.3.0 test environment:

```sh
tests/run-toil-bundle-integration.sh
```

The script creates/reuses `target/toil-bundle-venv`, installs
`toil[server,cwl,aws]==9.3.0` (the server imports its AWS helpers even for local
execution), and starts a loopback-only WES with `--bypass_celery --no-container`.
It prepares the checked-in CWL workflow and nested echo tool through the TRS
fixture, submits their verified staged bytes to the real WES/Toil engine, waits
for COMPLETE and verifies the output file contains `verified hello\n`. It records
the run/manifest in `target/toil-bundle-evidence.json` and stops its server on exit.
Set `GA4GH_TOIL_ENV_DIR` to reuse another prepared virtual environment.

This is a named local backend check, not a claim of compatibility with every
WES deployment. Toil 9.3.0 advertises WES 1.0.0 and omits generic GA4GH identity
fields from service-info; the test uses explicitly supplied backend identity
instead of the SDK's generic discovery constructor. TRS is a controlled fixture;
WES, the CWL engine and output verification are real. Starter Kit's disabled
attachment handling remains unsupported for this bundle scenario.

## Test fixtures

- [`sample.tes`](sample.tes) comes from the Funnel task examples and is used by
  [`funnel_tes.rs`](../lib/tests/funnel_tes.rs).
- [`grape.tes`](grape.tes) is an alternative TES task document. It is not used
  by the current tests and contains nonstandard placeholders such as
  `${AWS_ACCESS_KEY_ID}` that would need substitution before submission.
