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

## TRS-Filer/TRS

This suite needs a running Docker daemon and `curl`.

```sh
tests/run-trs-filer-integration.sh
```

The script starts [TRS-Filer](https://github.com/elixir-cloud-aai/trs-filer)
(the image of a `dev` commit that serves TRS 2.0.2, for `linux/amd64`) and
MongoDB 7.0, both pinned by tag and digest, on a private Docker network, and publishes TRS-Filer on a free
loopback port. It waits up to 120 seconds for `/ga4gh/trs/v2/service-info`
(set `TRS_FILER_STARTUP_TIMEOUT_SECONDS` to change the limit), runs the
library and CLI tests with the `trs_filer_integration_tests` feature, and
removes the containers, their volumes and the network. On failure it prints
the TRS-Filer logs.

The script then registers the CWL workflow in
[`fixtures/trs-filer/tool.json`](fixtures/trs-filer/tool.json) (`POST /tools`,
an extension of TRS-Filer), with a primary and a secondary descriptor, a file
of type `OTHER` and two tags, and passes its ID to the tests in
`TRS_FILER_TOOL_ID`. The tests compare the responses with the fixture. The
library test lists tools by tags, reads the version with its `description`,
fetches the descriptor, the file list with all three file types, the `OTHER`
file by relative path and the ZIP archive of all files. The CLI test does the
same through `trs tools --tag`, `versions`, `version`, `descriptor --path`,
`files` and `files --zip`.

TRS-Filer doesn't serve the `PLAIN_` descriptor types (`--plain`) yet, and it
doesn't find files in subdirectories by a relative path sent as one encoded
path segment (`tools%2Fecho.cwl`), as the SDK sends it. The suite therefore
fetches a top-level file by relative path; `PLAIN_` types and nested relative
paths are checked only against mock servers, in the default tests.

The [CI workflow](../.github/workflows/ci.yml) runs this suite as the job
`Integration (trs-filer)` when the TRS, Service Info or shared library code,
the CLI, the dependencies or the suite's own files change (see the path lists
in the workflow), and on manual dispatch.

To update TRS-Filer, pin `trs_filer_image` in the script to a release or a
`sha-<commit>` tag of `elixircloud/trs-filer`, not to `dev`, together with its
digest, e.g. from `docker buildx imagetools inspect
elixircloud/trs-filer:sha-<commit>`.

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
