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

## TRS public registries

These smoke tests call WorkflowHub and Dockstore, so they need network
access. They are marked `#[ignore]` rather than gated by a Cargo feature, so
they build with the other tests but only run on request:

```sh
cargo test -p ga4gh-lib --test trs_public_registries -- --ignored
```

## Public Service Registry

The first ignored smoke test reads all four Service Registry 1.0 endpoints
from the [official GA4GH Registry](https://ga4gh.github.io/ga4gh-registry/).
It looks up the registry itself by ID. It needs network access and anonymous
reads.

```sh
cargo test -p ga4gh-lib --test registry_public official_registry_reads_without_a_token -- --ignored
```

For a stricter check, set `GA4GH_REGISTRY_URL` to a compatible registry root
that permits anonymous reads. This second test gets **every** listed service
by its ID and confirms its type is in `/services/types`:

```sh
GA4GH_REGISTRY_URL='https://your-registry.example.org/ga4gh/registry/v1' \
  cargo test -p ga4gh-lib --test registry_public configured_registry_supports_all_read_endpoints -- --ignored
```

As checked on 2026-11-10, the official GA4GH deployment returns HTTP 400
when looking up a listed service whose ID contains `/`; the SDK encodes that
slash as `%2F` to keep the ID in one URL path segment. The strict check is
therefore unsuitable for that deployment until its lookup accepts such IDs.

## Test fixtures

- [`sample.tes`](sample.tes) comes from the Funnel task examples and is used by
  [`funnel_tes.rs`](../lib/tests/funnel_tes.rs).
- [`grape.tes`](grape.tes) is an alternative TES task document. It is not used
  by the current tests and contains nonstandard placeholders such as
  `${AWS_ACCESS_KEY_ID}` that would need substitution before submission.

## Downstream proTES consumer

`examples/protes-companion` is a standalone crate using main SDK transport/auth.
Run `cargo test --manifest-path examples/protes-companion/Cargo.toml` for provenance
retention, ordered attempts, rerun sanitization and 403/404 semantics. Its README
provides actual offline CLI examples and describes the outstanding live pipeline
acceptance. Fixture tests do not demonstrate remote gateway execution.
