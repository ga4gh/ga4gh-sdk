# Optional durable adaptive TES campaign

This example composes the existing GA4GH Python binding with **Temporal**, an
existing durable execution runtime. The runtime is optional: the Rust workspace
and normal SDK installation acquire no Temporal dependency. This is a
multi-trial ownership adapter, not a workflow engine or optimizer library.

The runnable benchmark uses the [portable CPU providers](../portable-execution/CONTRACT.md).
The public input is the integer sequence 0..255. Each native CPU subprocess runs
a deterministic SHA-256/LCG workload and reports the shifted quadratic objective
`score = -(parameter - 3)**2`, plus a checksum. It is a small public reproducible
benchmark, not a validated performance or scientific comparison suite. The image
name is an allowlisted recipe label; this demo executes host processes, not Docker.

Two trusted loopback TES endpoints run on one host and share a cooperating
provider coordinator. They demonstrate distinct endpoint ownership contracts,
not independent cloud trust domains. TES create/get/cancel go through the existing
Rust binding. Result references, process evidence, and intent receipts use the
explicit optional provider contract outside TES fields. No WES adapter is added.

## Install and run

Use Python 3.12+ and the repository's [Python binding setup](../../python/README.md).
For example, from the repository root:

```sh
python3 -m venv /tmp/ga4gh-campaign-env
. /tmp/ga4gh-campaign-env/bin/activate
python -m pip install -r examples/adaptive-campaign/requirements.txt 'maturin>=1.7,<2'
cd python
maturin develop
cd ..
```

Use this checkout's binding, rather than assuming the separately published Python
package has these synchronous I/O methods. Never fork a process after starting
its threaded native runtime; this example starts fresh worker subprocesses.

The dispatch adapter passes a task mapping to `TES.create()`. In the current
bindings a string is a file path, so serialized JSON text is not a task argument.

Install the [official Temporal CLI](https://docs.temporal.io/cli), then start the
native development service in one terminal with SQLite persistence:

```sh
temporal server start-dev --ip 127.0.0.1 --db-filename /tmp/ga4gh-campaign-temporal.sqlite
```

Start the CPU targets in a second terminal and retain the printed manifest:

```sh
python examples/portable-execution/provider.py serve --root /tmp/ga4gh-campaign-providers
```

Create `/tmp/campaign.json` using the two actual URLs from that manifest:

```json
{
  "providers": [
    {"name": "alpha", "url": "http://127.0.0.1:ALPHA_PORT"},
    {"name": "beta", "url": "http://127.0.0.1:BETA_PORT"}
  ],
  "max_trials": 6,
  "max_concurrency": 2,
  "deadline_seconds": 60,
  "steps": 80,
  "delay_ms": 20
}
```

Start a worker in a third terminal, then submit and inspect from a fourth:

```sh
python examples/adaptive-campaign/controller.py worker
python examples/adaptive-campaign/controller.py start --id public-search-1 --config /tmp/campaign.json
python examples/adaptive-campaign/controller.py status --id public-search-1
python examples/adaptive-campaign/controller.py result --id public-search-1
python examples/adaptive-campaign/controller.py history --id public-search-1 > /tmp/public-search-1.history.json
```

If the worker/controller exits, restart the same `worker` command against the
same Temporal service/database and task queue. The workflow resumes from its
history; do not start a new campaign to recover the old one. Duplicate workflow
IDs are rejected. The provider processes must remain reachable. The demo
supervisor is not a production worker recovery service.

## Pinned decisions and ownership

The workflow type `ga4gh-adaptive-campaign-v1`, task queue, metric schema, and
`integer-neighbors/v1` policy are versioned. Keep their implementation stable for
open histories; introduce a new version/task queue for changed decision semantics.
The policy starts at 0 and 6. Later trials select the closest untested integer to
the best **observed valid** score, breaking ties by earlier results and lower
parameter. Its decision evidence records the consumed result-event IDs, winning
score/event, and chosen parameter. Missing or failed results take a documented
deterministic untested fallback; they never supply a fabricated zero score.

Each trial retains its campaign/trial identity, recipe digest, provider, exact
request fingerprint, reservation, dispatch intent, acknowledgement when known,
protocol state, result reference, typed outcome, and decision evidence. The
recipe digest identifies the public input, recipe label, metric schema and pinned
policy. It is not an OCI image digest or a certification of the execution host.

The state sequence is:

```text
RESERVED -> DISPATCHING -> ACKNOWLEDGED -> TERMINAL -> RESULT_RECORDED
                        \
                         -> SUBMISSION_OUTCOME_UNKNOWN (reservation retained)
```

Temporal persists `ActivityTaskScheduled`, including the trial, request
fingerprint, and reserved dispatch intent, before executing the POST. Remote I/O
occurs only in activities. Creation has **one attempt** at both the SDK and
Temporal activity layers. A worker dying after acceptance can cause an activity
timeout without an acknowledgement; recovery enters
`SUBMISSION_OUTCOME_UNKNOWN`. A local ledger or Temporal history cannot make an
external non-idempotent POST exactly once.

Alpha's documented, durable provider receipt binds the intent, normalized whole
request fingerprint, provider, and remote ID. All must match before adoption.
Beta has no such mechanism: HTTP 501, HTTP 404, an empty list, or a failed
observation cannot prove non-execution. Beta's uncertain task remains reserved
and unscored, even if a person can see a completed task on that provider. No
second create is attempted for that intent.

While a workflow is still open, an operator may explicitly retire that unknown
reservation with a reason:

```sh
python examples/adaptive-campaign/controller.py retire-unknown --id public-search-1 \
  --trial-id public-search-1:trial:3 --reason 'Operator accepts unresolved external worker risk'
```

This records `OPERATOR_RETIRED_UNKNOWN` and `cleanup=UNKNOWN`; it is **not**
provider-backed adoption or proof of worker absence. It never retries the retired
intent. At the campaign deadline a final report may retain an unresolved
reservation and return `NEEDS_EXPLICIT_RESOLUTION`; that completed report requires
provider/operator follow-up outside the closed workflow. Do not hide it by
starting another campaign. The example intentionally provides no automatic
"job absent" override.

## Limits, cancellation, and results

`max_trials` is 1..8 and `max_concurrency` is 1..2. Unknown submissions consume a
slot alongside acknowledged active runs. An observation error or `CANCELING`
never releases a slot. Only a recognized terminal TES state, definite dispatch
skipped before POST, or explicit operator retirement can release one. The
activity checks the absolute deadline again before calling create, so queued
activities cannot initiate a late POST.

After the deadline the workflow stops dispatching, requests cancellation of known
active IDs, and keeps observing until a terminal protocol outcome. Cancellation
acknowledgement is not completion. An unreachable provider can therefore leave a
workflow pending beyond its deadline: ownership is not discarded to make the
controller appear finished. If a controller is offline at the deadline, local
cancellation cannot occur until it returns.

Terminal protocol state and physical cleanup evidence are separate. The optional
local provider evidence can confirm an observed child exit; missing/failed
physical evidence stays `UNKNOWN`. These local campaign bounds are **not physical
resource, spend, worker-residency, or invoice caps**. Those guarantees require a
separate provider enforcement contract, reliable provider clocks, and authority
beyond this controller.

Metric outcomes distinguish `VALID_METRICS`, `MISSING_METRICS`, `INVALID_METRICS`,
`METRICS_UNAVAILABLE`, and remote terminal failures/cancellation. The typed metric
must match its parameter and step count; booleans and floats cannot masquerade
as integer scores. Result-event identity is trial plus remote ID. Replay cannot
score a completed event twice. Temporary result access failure remains a visible
unavailable outcome; this example does not retry metrics forever.

## Reproducible interruption evidence

The fast contracts need no Temporal service; adapter contracts run when the
optional Temporal package is installed:

```sh
python tests/run-h3-campaign.py
```

When the native GA4GH binding is also installed, the adapter checks include one
actual dispatch and cancellation through the local CPU/TES provider. This guards
the task argument contract against changes to the Python API.

The opt-in demonstration starts the official native Temporal development service,
real CPU provider processes, and fresh controller worker processes. On first use
Temporal's testing helper downloads its official CLI binary. No Docker is used.
Give a **new** evidence directory for each run:

```sh
python tests/run-h3-campaign.py --integration --evidence /tmp/ga4gh-campaign-evidence-1
```

It kills the controller during acknowledged runs, restarts it, then exits a new
worker immediately after beta accepts trial 3 but before the activity completion
is recorded. The test marker contains no remote ID and is never used for recovery.
Independent provider evidence proves acceptance. The demo checks six unique
accepted intents, five unique scored results across both endpoints, preserved
decision/result prefixes, at most two reservations, and one explicit unresolved
beta reservation at the deadline. A separate alpha crash proves receipt-based
adoption. A third campaign exercises actual deadline cancellation. Each saved
Temporal history is independently replayed with the SDK `Replayer`.

The directory retains snapshots, all Temporal histories, the SQLite service
history database, provider records/results, worker logs, and `summary.json`.
The harness closes and reaps its own CPU children at teardown; that test cleanup
is not a production campaign cleanup guarantee. No cloud execution, billing,
exactly-once physical execution, or production durability claim follows from it.

Temporal references: [workflow durability](https://docs.temporal.io/workflows),
[Python SDK](https://docs.temporal.io/develop/python),
[activity retry policies](https://docs.temporal.io/encyclopedia/retry-policies).
