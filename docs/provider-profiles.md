# Experimental provider profiles

`tests/interop` records evidence for specific client/provider combinations. It
wraps the existing proTES real-backend integration script and existing SDK
surfaces. It is not a new conformance suite or a compatibility certification.
Profiles are opt-in and do not affect the standard test suite.

Each JSON profile records the API-root convention, tested specification,
implementation and client pins, public fixture identities, authentication,
storage assumptions and features. Reports add the actual SDK Git revision,
source checksums, advertised versions and per-check observations. Container pins
in the local profile describe the recorded local OCI snapshot: an operator must
load those exact images or publish a separately identified profile for a rebuilt
image. A local image digest is not a promise of availability in a public registry.

| Profile | Client and target | Evidence required |
| --- | --- | --- |
| `direct-funnel` | SDK → Funnel 0.12.3 | Real container completion, staged inline input, stdout/stderr, failed executor, cancellation |
| `protes-funnel` | Existing proTES `integration/check.sh` → proTES → Funnel | Existing real-backend suite, pinned by script SHA-256 |
| `nf-ga4gh-protes` | Nextflow 26.10.5 + nf-ga4gh 1.5.0 → proTES → Funnel | Shared work storage, nested directory/glob outputs and selected stdout/stderr |
| `seqera-native-wes` | SDK → native Seqera WES | Separate synthetic multipart fixture, authenticated read-only discovery and bounded opt-in launch/cancel evidence |

The nf-ga4gh documentation source is pinned at
`00c313135a88eca3a4b0de1bfa6475eb5a63f8fe`. The published 1.5.0 artifact comes
from release revision `23bcecd673dffcf5706b324494ab89ac1efd170c`; the profile
records these separately rather than claiming the release was built from the
later documentation commit.

## Evidence classification

Every check retains its outcome in JSON and in JUnit `outcome` properties:

| Outcome | Meaning | JUnit |
| --- | --- | --- |
| `passed` | The named assertion was exercised and satisfied | Passing case |
| `not_run` | No execution opt-in, missing operator setup or deliberately unexercised behavior | Skipped |
| `version_mismatch` | Advertised API/client or pinned suite identity differs | Failure with this type |
| `missing_fixture` | Required seed bytes are absent or differ from their manifest | Failure with this type |
| `harness_error` | Setup, transport, command timeout or report decoding failed | Error |
| `unsupported` | This profile does not expose or exercise the requested guarantee | Skipped |
| `observed_failure` | An executed assertion failed | Failure with this type |

The independent evidence label is `simulated_protocol`, `real_service`,
`real_execution` or `not_run`. A synthetic protocol pass never becomes real
execution evidence. An observed failure has no automatic attribution to a
provider, client or specification. Diagnose the relevant request, fixture and
implementation before reporting a defect upstream.

TES/WES `CANCELED` is an API observation. Physical worker cleanup needs separate
container/process/pod evidence; absent observability remains `unsupported`.
Completed short workflows do not establish cancellation behavior. The existing
Toil no-container test in [SDK PR #142](https://github.com/ga4gh/ga4gh-sdk/pull/142)
is engine evidence, not production container or cloud evidence.

## Run without provider credentials

From the repository root, with stable Rust and Python 3:

```sh
python3 tests/interop/run.py --out tests/interop/reports/not-run
python3 -m unittest discover -s tests/interop -p 'test_*.py'
python3 tests/interop/run.py --profile seqera-native-wes --offline-seqera \
  --out tests/interop/reports/seqera-offline
```

The default command verifies and seeds the public fixtures, then records all
provider executions as `not_run`. The offline Seqera fixture starts a loopback
HTTP server and uses the SDK example to send a real multipart request. It parses
MIME, verifies exact workflow bytes, `NFL`/`DSL2`, JSON engine parameters,
`workspaceId` and `computeEnvId`, then exercises status, run log and cancellation.
The fixture does not execute containers or contact Seqera.

Each invocation writes `report.json` and `junit.xml`. Private seed/work/config
files are retained under ignored `tests/interop/.local/run-work` for diagnosis.
Keep that directory private: it may contain bearer headers in Nextflow config,
provider IDs and raw logs. Public reports use selected observations, never raw
command output or private workspace/compute IDs. `SEQERA_PROFILE_SDK_BIN` can
point to a prebuilt `seqera_wes_profile` example; otherwise the wrapper uses
`cargo run --locked`.

## Disposable real local stack

Use an explicit operator-owned Docker context with Docker Compose. The helper
starts only the `ga4gh-h3-interop` Compose project, publishes its APIs on loopback,
and leaves MongoDB/RabbitMQ internal. It never creates, resizes or removes a VM.
The local snapshot needs the exact images listed in `local_stack.py` already
loaded. The Funnel binary is downloaded from the official v0.12.3 release and
verified against its pinned SHA-256. This reference snapshot uses Linux arm64
Funnel/Python/MongoDB/RabbitMQ with amd64 proTES under emulation.

The custom proTES snapshot can be rebuilt from public source. This is the actual
source/build workflow used for the recorded image (Docker Buildx and amd64
emulation are prerequisites on an arm64 daemon):

```sh
export DOCKER_CONTEXT=your-disposable-context
mkdir -p tests/interop/.local/protes-source
curl -fL https://api.github.com/repos/elixir-cloud-aai/proTES/tarball/55c4d1591cd3be883ad0b4fd2da01d49976414cb \
  -o tests/interop/.local/protes-source.tar.gz
tar -xzf tests/interop/.local/protes-source.tar.gz --strip-components=1 \
  -C tests/interop/.local/protes-source
docker buildx build --platform linux/amd64 --load \
  -t h3-protes:55c4d15 \
  --label org.opencontainers.image.revision=55c4d1591cd3be883ad0b4fd2da01d49976414cb \
  --build-arg SETUPTOOLS_SCM_PRETEND_VERSION=0.0.0+h3.55c4d15 \
  tests/interop/.local/protes-source
docker image inspect h3-protes:55c4d15 --format '{{json .RepoDigests}} {{.Id}}'
```

That source Dockerfile pins FOCA
`elixircloud/foca:0.22.0-py3.13@sha256:bf02db1195d60940dd3b29e53ce4757854155e258f35199e28d778cbf6128f9a`
and uv
`ghcr.io/astral-sh/uv:0.13.7@sha256:96f40d3904aad8d0bd010539001d36c8e2188b981e1334471a90c4a1d93bb490`.
Its clean source directory and pinned `.dockerignore` form the build context;
never add operator credentials. Load/pull the other exact public image digests
listed in `local_stack.py` before starting the helper.

`buildx --load` may produce a local tag with no `RepoDigests`. In that case use
the inspected `.Id` (for example `sha256:<local-image-id>`) as the exact preloaded
identity accepted by `--protes-image`, and record it as a local Docker image ID.
That ID may identify image configuration/engine state; do not call it an OCI
manifest or repository digest. If `RepoDigests` is populated, record its exact
`name@sha256:...` separately. The recorded snapshot above includes an OCI index;
these identity kinds are not interchangeable.

A rebuild may produce a different OCI digest, including build metadata. Do not
claim the recorded digest is reproducible bit for bit. Copy the existing proTES
profile to a new JSON file, assign a new `id`, and set `implementation.image` to
the exact inspected digest. Start with
`local_stack.py up --protes-image <name@sha256:digest>` and the pinned source
checkout; run the wrapper with that new profile ID. Retain the new source/image
identity and observations as a separate combination. The recorded default still
requires its exact original image.

```sh
export DOCKER_CONTEXT=your-disposable-context
export PROTES_SOURCE=/path/to/pinned/proTES
python3 tests/interop/local_stack.py up --context "$DOCKER_CONTEXT" \
  --protes-source "$PROTES_SOURCE"
export PROTES_COMPOSE_FILE="$PWD/tests/interop/.local/compose.yaml"
export TES_PROFILE_API_ROOT=http://127.0.0.1:18910/ga4gh/tes/v1
python3 tests/interop/run.py --profile direct-funnel --execute \
  --out tests/interop/reports/direct
export TES_PROFILE_API_ROOT=http://127.0.0.1:18920/ga4gh/tes/v1
export TES_PROFILE_BACKEND_URL=http://funnel:8000
python3 tests/interop/run.py --profile protes-funnel --execute \
  --out tests/interop/reports/protes
```

The source checkout supplies its existing synthetic identity provider, pinned by
file hash, used only for authentication. Funnel executes the tasks. The proTES
profile runs upstream `tests/integration/check.sh` unchanged: input staging,
successful and failed execution, full/basic log selection, immediate cancel and
cancel during execution. The profile records the script checksum and actual
advertised API/implementation versions. Its label `0.0.0.dev0` is retained when
that is what the deployment advertises, even if OCI labels name another version.
The source and image labels are separate evidence.

For Nextflow, use Java 17 or later supported by the pinned Nextflow release,
pinned Nextflow 26.10.5, and the published `nf-ga4gh@1.5.0` plugin. The executed
snapshot used Temurin 21.0.13+9, whose download checksum is recorded with evidence.
Obtain the synthetic token without printing it:

```sh
mkdir -p tests/interop/.local
(umask 077; docker compose -f "$PROTES_COMPOSE_FILE" exec -T fake-tes \
  python -c 'import json,urllib.request; print(json.load(urllib.request.urlopen("http://localhost:8080/oidc/token?sub=alice"))["access_token"])' \
  > tests/interop/.local/protes-token)
export NEXTFLOW_PROFILE_BIN=/path/to/nextflow-26.10.5
export NEXTFLOW_PROFILE_TOKEN_FILE="$PWD/tests/interop/.local/protes-token"
export NEXTFLOW_PROFILE_ENDPOINT=http://127.0.0.1:18920/ga4gh/tes
python3 tests/interop/run.py --profile nf-ga4gh-protes --execute \
  --out tests/interop/reports/nextflow
```

nf-ga4gh appends `/v1/tasks`; its configured endpoint is the parent
`/ga4gh/tes`, while the SDK/proTES integration uses `/ga4gh/tes/v1` as API root.
Do not add a trailing slash. Nextflow also needs direct access to its work
directory. The helper mounts `.local` at the same absolute path in Funnel;
the Docker VM must share that host path too. A reachable TES endpoint alone
does not provide storage access. The workflow copies a verified seed into a
nested directory and declares both a directory output and a glob output.

Set `INTEROP_DOCKER_CONTEXT` to the same disposable context to add a separate
bounded Docker observation after each local suite. It records the initial count,
waits up to 15 seconds for task containers to stop, and reports the observation
duration. It counts only running
containers with this stack's `ga4gh-h3-interop=task` label. A task container can still be running immediately after API `CANCELED`; that
initial count is retained. This does not turn a TES cancellation acknowledgement
into a whole-worker cleanup guarantee.

After recording operator-level container observations, remove only this stack:

```sh
python3 tests/interop/local_stack.py down --context "$DOCKER_CONTEXT"
```

The nf-ga4gh coverage gaps and regressions belong with existing issues
[#2](https://github.com/nextflow-io/nf-ga4gh/issues/2),
[#6](https://github.com/nextflow-io/nf-ga4gh/issues/6),
[#7](https://github.com/nextflow-io/nf-ga4gh/issues/7),
[#11](https://github.com/nextflow-io/nf-ga4gh/issues/11) and
[#12](https://github.com/nextflow-io/nf-ga4gh/issues/12). This profile does not open
replacement bug scopes.
Coverage is deliberately narrower than those issue scopes: #2 nonshared/object
storage is unsupported by this shared-filesystem fixture; #6 authoritative last
`TesTaskLog` selection across attempts is not run; #12 TES IDs exposed as WES
`TaskLog` is not run. The valid directory/glob fixture inspects the submitted
mapping and actual output bytes relevant to #7/#11; it neither exercises all
null/type/path mismatches nor claims an upstream issue fix.

## Native Seqera WES

Seqera documents native [`POST /ga4gh/wes/v1/runs`](https://docs.seqera.io/platform-api/ga-4-gh-run-create/).
Its organization workspace and compute-environment context are the existing
`workspaceId` and `computeEnvId` keys in `workflow_engine_parameters`. These
remain provider profile data; no generated standard model is changed.

A read-only authenticated probe needs only these settings:

```sh
export SEQERA_PROFILE_API_ROOT=https://api.cloud.seqera.io/ga4gh/wes/v1/
export SEQERA_PROFILE_TOKEN_FILE=/private/operator/seqera-token
python3 tests/interop/run.py --profile seqera-native-wes --execute \
  --out tests/interop/reports/seqera-probe
```

The example injects a sensitive bearer default header with
`Transport::with_client`. The configuration bearer fields are currently ignored
([SDK #31](https://github.com/ga4gh/ga4gh-sdk/issues/31)). Redirects are disabled.
Native WES discovery is validated explicitly; only its checked protocol kind
and spec version are projected into the public SDK WES instance, which allows
the same authenticated transport to perform the existing SDK multipart call.
No second vendor API client or replacement adapter is introduced.

A real launch additionally requires operator approval for one bounded job on an
existing test compute environment, storage/billing context and cleanup. Set
private context through environment variables, never profile JSON:

```sh
export SEQERA_PROFILE_WORKSPACE_ID=operator-owned-workspace
export SEQERA_PROFILE_COMPUTE_ENV_ID=operator-owned-compute-environment
export SEQERA_PROFILE_DESCRIPTOR_VERSION=operator-verified-version
export SEQERA_PROFILE_LEDGER=/private/operator/new-run-ledger.json
export SEQERA_PROFILE_ALLOW_LAUNCH=one-bounded-run
python3 tests/interop/run.py --profile seqera-native-wes --execute --live-launch \
  --out tests/interop/reports/seqera-live
```

In the default attachment mode, the workflow must exactly match the fixed public fixture: one process,
one CPU, 512 MB memory, a two-minute process limit and a 30-second sleep. The
example makes one submission, records the acknowledged ID privately, observes
status/run details, requests cancellation and waits for a terminal API outcome.
Poll budgets are capped at 60 seconds each; bounded HTTP calls keep observation and cancellation within five minutes. These are workflow/protocol bounds,
not a provider-enforced currency or physical-resource cap. The operator's
existing compute environment must enforce any stricter resource limits.

The operator may instead explicitly select `SEQERA_PROFILE_WORKFLOW_MODE=public-hello`
for the preseeded official `https://github.com/nextflow-io/hello` demonstration.
That repository contains four small echo tasks. The profile records expected
repository HEAD `3c2cdc9823c2b4636e5e3e73e223878099ff5dc9`, but does not invent a
native WES revision/configuration engine parameter: revision enforcement and
provider resource overrides remain `unsupported` unless independently observed.
The multipart request still uses the existing SDK and explicit workspace/compute
context. The selected demonstration environment's policy remains operator-owned.

The private ledger is created before submission and must not already exist. A
lost response retains `SUBMISSION_OUTCOME_UNKNOWN` and never retries POST.
An acknowledged unresolved ID remains in the ledger for operator cleanup. A
second invocation cannot overwrite that ledger and start another job. Do not
resolve uncertainty by deleting the ledger and blindly retrying.

For future explicitly approved submission failures, the existing private ledger
also retains a bounded `diagnostic` object. `HttpStatusError` body text is saved
up to 4096 UTF-8 bytes, with original/saved byte counts and a truncation flag.
A body-contained `requestId` or `request_id` (top-level or under `error`) is
retained up to 256 bytes. The current transport error does not expose response
headers, so a header-only request ID remains unavailable. Public JSON/JUnit
continues to expose only the sanitized HTTP status for the failed submission.

The ledger is created with POSIX mode `0600` before POST and updated through the
same file handle, including a flush to disk. Existing-ledger refusal, one-POST
behavior and disabled redirects remain unchanged. If the transport's own body
limit or I/O wraps the response as ambiguous, only its bounded private reason is
retained; missing typed status/body are not reconstructed from prose. This does
not recover the first live attempt's discarded response or resolve its UNKNOWN
intent. A further cloud attempt still requires separate explicit authorization
and a separate new ledger; never delete the original intent to retry.

Run the actual SDK diagnostic regressions entirely on loopback:

```sh
python3 tests/interop/native_diagnostics.py
```

This opt-in check requires Rust (or `SEQERA_PROFILE_SDK_BIN`). It verifies HTTP
400 body/request-ID retention, UTF-8 byte limits, private permissions, one POST
across a second invocation, and absence of synthetic private markers from public
stdout/stderr/JSON/JUnit. It uses no real credentials or cloud services.

The observed native service advertised WES `1.0`, descriptor `NFL` with an empty
version list and Nextflow `20.01.0`. The profile exposes that discovery gap:
a real launch needs an explicit operator-verified descriptor version. Read-only
discovery and offline `DSL2` fixture acceptance are not evidence that a cloud
launch with that version succeeded.

## Upstream evidence handoff

Attach the JSON/JUnit pair and a short command/version/prerequisite summary to
the existing [TES compatibility matrix](https://github.com/ga4gh/task-execution-schemas#compatibility-matrix)
or testbed documentation. Preserve every unsupported/not-run classification and
identify the source revision/dirty flag. Existing
[proTES integrations](https://github.com/elixir-cloud-aai/proTES/blob/55c4d1591cd3be883ad0b4fd2da01d49976414cb/docs/development.md),
[openapi-test-runner](https://github.com/ga4gh/openapi-test-runner/blob/bc3eb911e877521409348e0225852d072d4de196/README.md)
and [DRS compliance suites](https://github.com/ga4gh/drs-compliance-suite) retain
schema/protocol conformance ownership. Seed deployment-owned DRS objects before
those suites; this TES/Nextflow fixture does not invent a DRS write API or infer
DRS coverage from a TES pass.
