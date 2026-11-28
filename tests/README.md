# Integration suites and test fixtures

Run these commands from the repository root. The configuration and JSON
persistence suites run by default with `cargo test --workspace`. The service
integration suites require explicit commands or features, local HTTP access,
and the services described in their sections.

## Configuration and JSON persistence

These suites need the repository's Rust toolchain and temporary filesystem
access; they do not use external services:

- [`configuration_sources.rs`](../lib/tests/configuration_sources.rs) checks
  source precedence, profile selection, nulls and array replacement, typed
  additional settings, credential conflicts, and secret-safe diagnostics.
- [`json_store.rs`](../lib/tests/json_store.rs) checks atomic persistence,
  failed updates, malformed input, preserved I/O causes, permissions of private
  data and lock files, exclusive locking during an update, and
  preservation of changes from concurrent writers. A unit test in
  [`json_store.rs`](../lib/src/utils/json_store.rs) injects a directory-sync
  failure after real replacement and checks its machine-readable status.

Run both suites on their own with:

```sh
cargo test --locked -p ga4gh-lib --test configuration_sources --test json_store
```

## Native extension manager

The SDK's [`extensions.rs`](../lib/tests/extensions.rs) suite covers installation
failures, registry locking across concurrent managers, settings from composed
configuration sources, and the lifetime of a real shared library. It needs local
HTTP sockets, temporary filesystem access, the repository's Rust toolchain, and
a C compiler for the host target. The fixture compiles against the checked-in
C ABI header. Tests cover buffer ownership, instance isolation and concurrency,
admission during disable/removal, process version pins, and immutable updates.
Python's `tests/test_extensions.py` compiles and calls the same C fixture through
the bindings; it also requires a host C compiler. The CLI's
[`extensions.rs`](../cli/tests/extensions.rs) checks caller-owned paths and command
output. On Unix, `native_commands.rs` also compiles a command provider with
`cc` and checks lazy loading, selection, collisions, and recovery. These suites
run with the default workspace tests and need no external
services:

```sh
cargo test --locked -p ga4gh-lib --test extensions
cargo test --locked -p ga4gh-cli --test extensions --test native_commands
```

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
and of the CLI. It waits up to 60 seconds by default; set
`FUNNEL_STARTUP_TIMEOUT_SECONDS` to change the limit. It does not stop a Funnel
process that it starts.

The CLI suite, [`funnel_cli.rs`](../cli/tests/funnel_cli.rs), runs the
`ga4gh-cli` binary with a temporary home directory whose config points to
Funnel. It runs a task without inputs or outputs to completion with
`tes create`, `tes status`, `tes get` and `tes list`. The list checks cover
matching and nonmatching tag filters. The suite also cancels a running task
with `tes cancel` and checks that an unknown task returns a non-zero exit
status, empty stdout, and HTTP 404 on stderr.

The [`funnel_tes_files.rs`](../cli/tests/funnel_tes_files.rs) CLI suite runs a
task and checks its stdout through `tes files`, `tes log` and `tes save`.
Downloads of output files from HTTP and S3 storage are checked against mock
servers in the default CLI tests.

Funnel needs network access to pull `alpine:3.22` for these tasks. Each state
polling loop has a 180-second deadline; an in-flight CLI request can extend
that wait by its configured timeout. The other `tes` tests of the CLI run
against mock servers in the default `cargo test`.

## proTES/TES

[proTES](https://github.com/elixir-cloud-aai/proTES) is a TES gateway: it
serves the TES API itself, requires access tokens for tasks, and relays tasks
to other TES instances. This suite runs the TES client against proTES 0.6.0
relaying to Funnel, so the client meets a second TES implementation, with
authentication, while tasks still run for real.

Prerequisites: Docker with Compose, `curl`, network access (to pull images
and the identity provider's file), and a Linux Funnel 0.11.14 executable
matching the Funnel container's architecture. This suite runs Funnel inside
a Linux container, so the host-native macOS executable used by the Funnel/TES
suite above cannot run here.

On Linux, with a compatible `funnel` on `PATH`, run:

```sh
tests/run-protes-integration.sh
```

On macOS, download the `funnel-linux-arm64` or `funnel-linux-amd64` archive
from the Funnel release linked above to match the container, extract it, and
point `FUNNEL_BIN` to that executable:

```sh
FUNNEL_BIN=/absolute/path/to/linux/funnel tests/run-protes-integration.sh
```

The script starts the services of [`protes/compose.yaml`](protes/compose.yaml):
proTES and its worker, MongoDB, RabbitMQ, Funnel (the host's executable, in a
container that uses the host's Docker daemon), and the fake identity provider
of the proTES smoke test, whose file it downloads from a pinned proTES commit
and checks against a checksum. It gets an access token of a test user from
the identity provider and runs [`protes_tes.rs`](../lib/tests/protes_tes.rs):

- a task without inputs or outputs runs to completion through the gateway
  (state, logs, the backend tag of proTES and the list filter), within 180
  seconds;
- a long task is canceled;
- requests without a token, or with an invalid one, get HTTP 401.

The library test passes the token with
`Configuration::with_oauth_access_token`.

It then runs the CLI suite, [`protes_cli.rs`](../cli/tests/protes_cli.rs).
It runs the `ga4gh-cli` binary with a temporary home directory whose config
points to proTES and holds the access token as `oauth_access_token`, so the
token reaches proTES from the CLI configuration only:

- a task runs to completion with `tes create`, `tes status`, `tes get` and
  `tes list` (with matching and nonmatching tag filters);
- a running task is canceled with `tes cancel`;
- with a config without a token, `tes create`, `tes status`, `tes get`,
  `tes list` and `tes cancel` (`status`, `get` and `cancel` on an existing
  task of the test user) fail with a non-zero exit status, empty stdout,
  and HTTP 401 on stderr.

The script builds both test binaries before it starts the services, because
the access token expires 10 minutes after it is issued. On exit, it prints
the service logs if a step failed and removes the services and its temporary
files.

| Variable | Default | Meaning |
| --- | --- | --- |
| `FUNNEL_BIN` | `funnel` on `PATH` | Linux Funnel executable for the container |
| `DOCKER_SOCKET` | from the Docker context | Docker socket for Funnel |
| `PROTES_PORT` | `8090` | Port of proTES on `127.0.0.1`; must be free |
| `PROTES_STARTUP_TIMEOUT_SECONDS` | `300` | Time limit for the services to become healthy |

All images are pinned by digest. To update proTES, bump its image digest in
the Compose file together with `protes_commit` and `fake_tes_sha256` in the
script; the Docker CLI image of Funnel is `DOCKER_CLI_IMAGE` in the script.
The [CI workflow](../.github/workflows/ci.yml) installs Funnel as for the
Funnel suite and runs this suite (`Integration (protes)`) when TES, CLI,
shared library, dependency or its own files change.

## CLI image

This suite needs bash 4.4 or later, a running Docker daemon with
[Buildx](https://docs.docker.com/build/concepts/overview/#buildx), and for
its last part `curl` and the `funnel` executable (see above;
`tests/install-funnel.sh DIRECTORY` installs the pinned version on Linux
x86-64).

```sh
tests/run-image-tests.sh
```

The script builds the [CLI image](../docs/container-image.md) from the
working tree and checks that:

1. the build context holds every tracked file of the workspace manifests and
   the `lib/`, `cli/` and `python/` crates, and leaves out probe files that
   look like environment files, keys, credential stores, build output and
   other directories, while keeping `lib/.env.example` (the probe files are
   created in the working tree and removed again, together with directories
   created for them); the Rust image of the `Dockerfile` matches
   `rust-toolchain.toml`;
2. the image's entrypoint is `ga4gh-cli`, `--help` and the default command
   print the help, `--version` prints the workspace version, it runs as UID
   10001 with `HOME=/home/ga4gh` and contains no sources;
3. if `funnel` is on `PATH`, the
   [site check script](../examples/kubernetes/tes-site-check.sh) runs in the
   image against Funnel on `localhost:8000`, with the configuration mounted
   read-only and a read-only root file system. It exits 0 with
   `result=COMPLETE` for the example task; 1 with `result=EXECUTOR_ERROR` for
   a task whose command fails; 1 with `result=TIMEOUT` for a task that runs
   too long, which it cancels; and 1 without submitting a task for an
   unreachable TES and for an invalid `TIMEOUT_SECONDS`.

The script starts Funnel if nothing answers at `localhost:8000/healthz`, and
stops it at the end; otherwise it uses the service already there. Set
`IMAGE` to choose the tag of the built image (default `ga4gh-cli:test`),
`PLATFORM` to build for another platform, such as `linux/arm64`, and
`FUNNEL_CHECK` to `true` or `false` to require or skip the third part
(default `auto`). The [`CLI image` workflow](../.github/workflows/image.yml)
runs it for `linux/amd64` with Funnel and for `linux/arm64` without.

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
It runs all integration suites on manual dispatch. The `CI passed` job checks
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
MongoDB 7.0, both pinned by tag and digest, on a private Docker network, and
publishes TRS-Filer on a free loopback port. It needs Docker and curl 7.76 or
later (for `--fail-with-body`). It waits up to 120 seconds for `/ga4gh/trs/v2/service-info`
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

## Keycloak/CLI login

This suite runs `ga4gh-cli login` with the OpenID Connect device flow against
a real provider: [Keycloak](https://www.keycloak.org/) 26.8.2, pinned by tag
and digest. It needs a running Docker daemon, `curl` and network access to
pull the image.

```sh
tests/run-keycloak-integration.sh
```

The script builds the test, starts Keycloak in development mode (plain HTTP,
which the CLI accepts on loopback addresses only) on a free loopback port,
imports the test realm [`keycloak/realm.json`](keycloak/realm.json), waits up
to 180 seconds for the realm's discovery document (set
`KEYCLOAK_STARTUP_TIMEOUT_SECONDS` to change the limit), runs
[`keycloak_login.rs`](../cli/tests/keycloak_login.rs) with the
`keycloak_integration_tests` feature and the issuer in `KEYCLOAK_ISSUER`, and
removes the container. On failure it prints the Keycloak logs.

The realm is for this test only: it has a public client `ga4gh-cli-test`
with the device grant enabled, and a user `test-user` with the password
`test-only-password` and the `offline_access` role, which the CLI's default
scope `openid offline_access` needs. No admin user is created. Access tokens
are valid for 40 seconds, and refresh tokens are rotated: each refresh
returns a new one, and Keycloak ends the session when an old one is used
again. Keycloak has no API to approve a device code, so
[`keycloak/approve-device.sh`](keycloak/approve-device.sh) does what the user
does in the browser: it opens the verification URL with the code, signs in
through the login form and answers the consent page, which Keycloak always
shows for the device grant. It relies on the HTML forms of Keycloak's default
login theme, so a Keycloak update may need changes there.

The tests run the `ga4gh-cli` binary with a temporary home directory whose
configuration has a `REGISTRY` section with `oidc` settings for the realm,
pointing to a local service that records the `Authorization` header:

- `login registry` shows the verification URL; after the scripted approval
  it stores the tokens with mode `0600` and prints none of them;
- `registry list` sends the stored access token, and Keycloak's userinfo
  endpoint accepts it for the test user;
- once the token expires within 30 seconds, `registry list` refreshes it
  first and stores the new access and refresh tokens; a second refresh, which
  succeeds only with the stored rotated refresh token, follows. The test
  waits about 20 seconds for both;
- `logout registry` deletes the login, and `registry list` then fails with
  "Not logged in" without a request to the service;
- a login that the user denies on the consent page fails with "The login was
  denied" and stores no tokens.

The refresh after a `401` and the provider's `slow_down` answer are checked
only against the fake provider of the default CLI tests.

The [CI workflow](../.github/workflows/ci.yml) runs this suite as the job
`Integration (keycloak)` when the CLI, the Registry client used by the
test, the shared library code, the dependencies or the suite's own files
change (see the path lists in the workflow), and on manual dispatch.

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
make workflow-extension
python3 tests/run-workflow-contracts-qc.py
```

`make workflow-extension` requires Python 3 and the workspace Rust toolchain. It
builds the provider in its own Cargo workspace, packages it, installs it in
temporary registries, and exercises offline Draft 2020-12 validation, pinned upstream
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

The Python bindings have the same smoke tests. They are skipped unless
`GA4GH_TRS_PUBLIC_TESTS=1` is set; run them from `python/` after building the
bindings as described in the [Python guide](../python/README.md):

```sh
GA4GH_TRS_PUBLIC_TESTS=1 python -m pytest tests/test_trs_public.py
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
- [`fixtures/trs-filer/tool.json`](fixtures/trs-filer/tool.json) is the CWL
  workflow that
  [`run-trs-filer-integration.sh`](run-trs-filer-integration.sh) registers in
  TRS-Filer for [`trs_filer.rs`](../lib/tests/trs_filer.rs) and
  [`cli/tests/trs_filer.rs`](../cli/tests/trs_filer.rs).
