# Cooperating CPU providers: experimental contract v1

This is a loopback-only, host CPU demonstrator, not a container runtime or a
production TES deployment. `provider.py` exposes two real TES HTTP endpoints;
each task runs a separate allowlisted Python CPU subprocess. No arbitrary image
or command is executed. No TES/WES schema extensions are required.

## Starting and embedding

`python3 examples/portable-execution/provider.py serve --root /tmp/h3-providers`
prints a JSON manifest. Ports default to ephemeral loopback ports. Import
`provider.py` with `importlib.util.spec_from_file_location`, then use
`cluster = Cluster(Path(root)).start()`; `cluster.providers` is a list of
`Provider` objects with `name` (`alpha`, `beta`) and `url`. `cluster.close()` stops
and waits for every owned child. A context manager is also supported. The
initial default is provider-backed intent reconciliation on alpha and explicitly
unsupported reconciliation on beta. Pass `reconcile=(True, True)` to opt in on
both. Storage and ownership are shared by this cooperating pair.

## Standard TES surface

The base URL is `http://127.0.0.1:<port>`; use `/v1/tasks`, `/v1/tasks/{id}`,
`/v1/tasks/{id}:cancel`, `/v1/service-info`. `/ga4gh/tes/v1/...` aliases also work.
Create returns `{"id":"..."}`. Get accepts MINIMAL/BASIC/FULL, list accepts
name_prefix, state, tag_key/tag_value, page_size/page_token. Cancel returns `{}`;
observe the terminal CANCELED state before releasing ownership. Physical process
cleanup is separate optional evidence.

```json
{
  "name": "public-cpu-trial",
  "executors": [{"image": "ga4gh-h3-cpu:v1", "command": ["h3-cpu", "--steps", "80", "--parameter", "2", "--delay-ms", "10"]}],
  "tags": {"h3.intent": "unique-dispatch-intent"}
}
```

Use `make_task(parameter=2, steps=80, delay_ms=10, intent="...")` for this
recipe. The application repeatedly hashes the fixed public integer sequence
0..255 with an explicit deterministic LCG seed. Parameters are bounded. Output
`/outputs/metrics.json` is typed JSON: `schema`, integer `parameter`, integer
`steps`, integer `score`, string `checksum`. It is available through
`GET /control/v1/tasks/{id}/metrics` after COMPLETE, and through the standard
FULL TES task `logs[0].outputs[0].url`. No credentials occur in output.

## Optional provider evidence (not TES standard)

`GET /control/v1/capabilities` declares the actual capabilities.
`GET /control/v1/tasks/{id}/evidence` reports task state, PID, physical process
status, attempt/lineage, ownership epoch, restore progress, checkpoint bytes and
performed work. `GET /control/v1/intents/{intent}` returns a provider-backed
binding `{intent, task_fingerprint, task_id, provider}` on alpha; beta responds
501. It is scoped to the demo principal and fingerprint and is a durable unique
intent mapping. Duplicate alpha creates with the same intent and task return the
original ID; changed task or dispatch context conflicts. The durable binding
includes operation kind and admission plan ID; continuation also binds receipt,
source lineage/execution/epoch and checkpoint digest. Evidence includes this
`dispatch_binding`. An identical acknowledged dispatch may be adopted after plan
expiry without staging or relaunch. Reusing the intent with another plan,
receipt or operation is HTTP 409. Legacy records lacking this complete binding
are not silently adopted. Tags on beta are descriptive only: repeated
POSTs can start duplicate tasks. A lost response must remain unresolved there;
an empty list is not proof of non-execution. A campaign may inject acceptance /
client-persistence failure itself after a successful create to exercise recovery.

## Optional admission and checkpoints

`X-H3-Principal` defaults to `demo`; it is a trusted-loopback test identity,
not authentication. `POST /control/v1/plans` accepts `{task, policy, ttl_seconds}`;
`POST /control/v1/submit` accepts `{task, plan_id}` on the selected backend.
Plans bind the canonical TES request, principal, policy version, backend, input
identity, executable, expiry and managed staging constraints. Staging acquires its
write lock before validating plan principal, object identity, task fingerprint
(when supplied), destination, current policy version and expiry; the same
transaction authorizes the actual copy. Plan expiry governs new admission and
staging, not already admitted execution. Current storage policy and writer
ownership are rechecked atomically at checkpoint/result commits. Ordinary TES
create uses server-owned default policy and evaluates it before staging.
`POST /control/v1/stage` accepts `{input_id, destination}` and rechecks current
server policy; arbitrary URLs/file paths are rejected. The one public object is
`public-numbers-v1`. `POST /control/v1/policy` changes test policy version.
`POST /control/v1/availability` with `{available:false}` injects target failure.

`GET /control/v1/tasks/{id}/checkpoints` lists committed receipts only.
`POST /control/v1/continue` takes `{receipt_id, task, plan_id, intent}` on the
target. Receipt, retained artifact, compatibility, principal, expiry and target
admission are validated before an atomic SQLite writer compare-and-swap. A
single continuation wins; replay of the same durable intent adopts its task.
Each result commit rechecks writer ID and epoch in that same coordinator.
`POST /control/v1/tasks/{id}/preempt` stops a source with TES PREEMPTED after a
checkpoint; live ownership transfer also fences a still-running source result.
No promise of exactly-once physical execution is made. Unsupported process
snapshots / engine resume return 422 when requested as a continuation `mode`.

Details, error reasons and evidence examples are in README.md and the RFC.
