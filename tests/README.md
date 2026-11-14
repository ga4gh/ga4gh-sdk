# Integration suites and test fixtures

The default `cargo test` does not enable the integration suites below. Run
their commands from the repository root. They need local HTTP access and the
services described in each section.

## Funnel/TES

On Linux or macOS, download the matching archive from the
[Funnel 0.11.14 release](https://github.com/calypr/funnel/releases/tag/v0.11.14).
Extract the `funnel` executable and add its directory to `PATH`. CI uses this release.
Make sure `curl` and a Docker daemon are available. The suite uses `localhost:8000`.

```sh
bash run-integration-tests.sh
```

The script starts Funnel if its `/healthz` endpoint is not healthy, waits for
that endpoint, then runs the `integration_tests` feature suites of the library
and of the CLI.
It waits up to 60 seconds by default; set `FUNNEL_STARTUP_TIMEOUT_SECONDS`
to change the limit. It does not stop a Funnel process that it starts.

The CLI suite, [`funnel_cli.rs`](../cli/tests/funnel_cli.rs), runs the
`ga4gh-cli` binary with a temporary home directory whose config points to
Funnel. It runs a task without inputs or outputs to completion with
`tes create`, `tes status`, `tes get` and `tes list`. The list checks cover
matching and nonmatching tag filters. The suite also cancels a running task
with `tes cancel` and checks that an unknown task returns a non-zero exit
status, empty stdout, and HTTP 404 on stderr.

Funnel needs network access to pull `alpine:3.22` for these tasks. Each state
polling loop has a 180-second deadline; an in-flight CLI request can extend
that wait by its configured timeout. The other `tes` tests of the CLI run
against mock servers in the default `cargo test`.

## Starter Kit WES

This suite needs a running Docker daemon with a usable Unix socket, plus
`python3`, `git`, and `curl`.

```sh
tests/run-wes-docker-integration.sh
```

The script starts `ga4gh/ga4gh-starter-kit-wes:0.2.0-nextflow` with temporary
configuration, an initialized SQLite database, a shared work directory, and
the Docker socket. It waits up to 180 seconds for
`/ga4gh/wes/v1/service-info`, runs the SDK test and then a CLI test that
submits a run with `ga4gh-cli wes run` and tracks it to completion with
`wes status` and `wes get`, and cleans up the container and temporary files.
On failure it prints server logs. Increase `WES_STARTUP_TIMEOUT_SECONDS` or
`WES_RUN_TIMEOUT_SECONDS` on slower hosts. Because of the server limitations
below, `wes cancel`, `wes run --attachment` and the paging of `wes list` are
checked only against a mock server, in the default CLI tests.
The [CI workflow](../.github/workflows/ci.yml) runs this suite on an Ubuntu
runner when WES, CLI, shared library, dependency, or integration files change.
It runs both integration suites on manual dispatch. The `CI passed` job checks
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

## Workflow contracts and two local QC executions

```sh
cargo test -p ga4gh-lib --features workflow-contracts --test workflow_contracts
cargo test -p ga4gh-cli --test workflow
python3 tests/run-workflow-contracts-qc.py
```

The first two commands exercise offline Draft 2020-12 validation, pinned upstream
fixtures, actual CLI JSON/errors, partial output delivery and comparison policies.
The [fixture README](fixtures/io-profile/README.md) records their upstream revision
and checksums; the [example README](../examples/sequence-qc/README.md) maps the
workflow, request, profile, policy and viewer adapter. The optional Python
harness requires Python 3.12+, Rust on PATH and a working Docker context with
this checkout shared into its VM. It installs Toil 9.3.0 and IPython 9.6.0,
checking those versions when reusing an environment. Their transitive Python
dependencies are not locked. The harness starts two independent loopback-only
WES processes, submits the same sequence-QC CWL descriptor in its pinned Python
container, verifies metrics and golden report bytes, compares their results and
supplies both reports to IPython. Evidence is written to
`target/workflow-contracts-qc/evidence.json`; servers stop on exit.
`GA4GH_TOIL_ENV_DIR` can reuse an environment. The optional
`GA4GH_QC_NO_CONTAINER=1` mode executes on the host and must be reported separately.
This suite uses native WES discovery; it makes no external-node/federation claim.

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

## Gen3 DRS (CanineDC)

This ignored smoke test reads CanineDC, a public Gen3 data commons, without
credentials. It needs network access to `caninedc.org`:

```sh
cargo test -p ga4gh-lib --test drs_gen3_public -- --ignored
```

It sends one service-info request and one bulk `POST /objects` request for a
documented public object and one unknown ID, allows one bulk attempt within a
30-second budget, and checks that the reply uses the Gen3 dialect, that the
object resolves by its ID, and that the unknown ID gets 404. It requests no
access URLs and downloads no bytes. Access URLs need credentials on this
service (`POST /objects/access` returned 401 anonymously on 2026-11-10), so
Gen3 bulk access behavior is unverified.

## Test fixtures

- [`sample.tes`](sample.tes) comes from the Funnel task examples and is used by
  [`funnel_tes.rs`](../lib/tests/funnel_tes.rs).
- [`grape.tes`](grape.tes) is an alternative TES task document. It is not used
  by the current tests and contains nonstandard placeholders such as
  `${AWS_ACCESS_KEY_ID}` that would need substitution before submission.
- [`fixtures/drs/gen3-caninedc-bulk-objects.json`](fixtures/drs/gen3-caninedc-bulk-objects.json)
  is the anonymous CanineDC reply to `POST /ga4gh/drs/v1/objects` for one
  public object and one unknown ID, recorded on 2026-11-10 and reformatted
  with sorted keys. It contains public metadata only and is used by
  [`drs_bulk.rs`](../lib/tests/drs_bulk.rs).

## Third-horizon optional experiments

These downstream prototypes keep normative TES/WES models unchanged. Their
runtime dependencies and evidence boundaries are documented alongside each
experiment. None needs cloud credentials for its local checks.

| Suite | Command from the repository root | Prerequisites and evidence |
| --- | --- | --- |
| Durable adaptive campaigns | `python3 tests/run-h3-campaign.py` | Fast contract checks; optional Temporal Python package enables adapter checks |
| Actual campaign interruption/recovery | `python3 tests/run-h3-campaign.py --integration --evidence /tmp/h3-campaign-new-run` | Python binding from this checkout, pinned Temporal dependency and native Temporal development service; two real loopback CPU targets, saved histories and independent replay |
| Portable checkpoints and admission | `python3 -m unittest discover -s examples/portable-execution -p test_provider.py -v` | Python standard library, loopback sockets; concurrent writer, expiry, cancellation and managed-staging regressions |
| Actual checkpoint continuation | `python3 examples/portable-execution/demo.py --output /tmp/h3-provider-demo.json` | Real bounded host CPU subprocesses and cooperating durable coordinator; no container/cloud claim |
| Exact deterministic reuse | `cargo test -p ga4gh-lib --features workflow-contracts --test result_reuse --locked` | Offline contract/trust/access/integrity checks |
| Actual container reuse | `python3 examples/result-reuse/run.py` | Docker with the pinned image cached and checkout shared into its VM; explicit `GA4GH_REUSE_DOCKER_CONTEXT` where needed, one A execution and zero B submissions |
| Shared scenario API | `python3 tests/run-h3-scenarios.py` | Rust, Python, Node 22+ and npm; HTTP isolation/replay tests plus actual SDK and maintained RestWesProvider histories |
| Deployed scenario API | `python3 tests/run-h3-scenarios.py --url http://127.0.0.1:8815` | An already running source or imported OCI deployment; exercises the supplied URL rather than starting another service |
| Direct SDK real TES profile | `cargo test -p ga4gh-lib --test provider_profiles -- --ignored` | Explicit `TES_PROFILE_API_ROOT` for an operator-owned disposable TES; preferably invoke through the profile wrapper below |
| Provider profile classifications | `python3 -m unittest discover -s tests/interop -p 'test_*.py'` | Python; verifies distinct passed/not-run/unsupported/failure classifications |
| Native private diagnostics | `python3 tests/interop/native_diagnostics.py` | Rust or `SEQERA_PROFILE_SDK_BIN`; actual SDK on loopback, private error bounds/0600/no-public-leak and one-POST ledger guard |
| Seqera multipart fixture | `python3 tests/interop/run.py --profile seqera-native-wes --offline-seqera --out tests/interop/reports/seqera-offline` | Rust and Python, real MIME parsing on loopback; simulated protocol only |

Read [campaign instructions](../examples/adaptive-campaign/README.md),
[provider contracts](../docs/horizon3-provider-contracts.md),
[result reuse](../docs/result-reuse.md), [scenario API](../docs/scenario-api.md)
and [provider profiles](../docs/provider-profiles.md) before opt-in execution.
The profiles wrap the existing pinned proTES suite, direct Funnel, and
nf-ga4gh/Nextflow with shared storage, exact fixture manifests and separate
JSON/JUnit evidence. Native Seqera discovery and a separately approved single
live job use private environment configuration. Missing operator access,
physical cleanup observation or hosted Reference Cloud deployment remains a
coverage gap; protocol fixtures never establish real execution or certification.
