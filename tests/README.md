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

The script starts Funnel if it does not find a running `funnel server run`
process, waits for `/healthz`, then runs the `integration_tests` feature suite.
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
The [Starter Kit workflow](../.github/workflows/wes-starter-kit.yml) runs this
suite on an Ubuntu runner for matching pull requests and manual dispatches.

Starter Kit WES 0.2.0 only accepts a GitHub-shaped Nextflow project URL; its
`workflow_attachment` argument is disabled. The script places the checked-in
[Nextflow fixture](fixtures/wes-nextflow/main.nf) in Nextflow's local Git
project cache at a deterministic commit, so the workflow runs without fetching
a remote project. This server returns a fixed run list instead of recently
submitted runs, and its cancellation endpoint does not stop a run. The test
accounts for both limitations and does not claim that a run was canceled.

## Test fixtures

- [`sample.tes`](sample.tes) comes from the Funnel task examples and is used by
  [`funnel_tes.rs`](../lib/tests/funnel_tes.rs).
- [`grape.tes`](grape.tes) is an alternative TES task document. It is not used
  by the current tests and contains nonstandard placeholders such as
  `${AWS_ACCESS_KEY_ID}` that would need substitution before submission.
