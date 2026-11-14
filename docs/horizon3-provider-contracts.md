# Experimental cooperating-provider checkpoint and admission contracts

Status: bounded RFC and executable CPU experiment for H3-02/H3-03. The
[example](../examples/portable-execution/README.md) contains two real loopback
HTTP execution targets and independent host CPU subprocesses. Its
[contract](../examples/portable-execution/CONTRACT.md) is versioned optional
provider functionality, not a proposed addition to required TES/WES fields.
There is no cloud deployment, geographic enforcement or container execution.

## Why a provider contract is needed

Resuming observation of an existing task ([SDK #131](https://github.com/ga4gh/ga4gh-sdk/issues/131))
does not recover application state at another site. [WES #203](https://github.com/ga4gh/workflow-execution-service-schemas/issues/203)
already discusses engine-owned rerun/resume. This experiment concerns the
checkpoint identity, compatibility and authority required for an explicitly
checkpoint-aware application to continue across cooperating execution targets.
It does not define another workflow scheduler or a general resume standard.

[Seqera Fusion snapshots](https://docs.seqera.io/fusion/guide/snapshots/) show the
value of surviving preemption using native process snapshots. Their platform
and hardware compatibility conditions do not establish portability between
arbitrary providers. This experiment uses application state, not CRIU, GPU
state, transparent process migration or a Fusion implementation.

TES already defines asynchronous execution, ordinary task states and provider
responsibility for authorization. Cancellation/preemption re-read state and
conditionally transition under the completion lock; a late stop request cannot
reverse an already committed terminal outcome. Those states remain intact here; checkpoint
availability, preemption reason, linked attempt and physical process evidence
are separate records. See the [TES specification](https://ga4gh.github.io/task-execution-schemas/docs/).
[TES #121](https://github.com/ga4gh/task-execution-schemas/issues/121) and
[WES #133](https://github.com/ga4gh/workflow-execution-service-schemas/issues/133)
remain the separate callback discussions.

## H3-02: committed application checkpoints

The application processes the public integer sequence 0..255. Its explicit LCG
state, SHA-256 accumulator and next step make restoration deterministic using
integer operations and a declared runtime contract. Every ten steps, the worker
writes checkpoint bytes to a unique partial file, fsyncs it, renames it and
fsyncs the directory. Only then does a FULL-synchronous SQLite transaction
publish the receipt. A crash can leave orphan bytes; an uncommitted or partial
artifact is never listed as a usable checkpoint. The SDK need not remain
connected for checkpoint production or continuation.

A receipt contains:

| Identity / condition | Experimental representation |
| --- | --- |
| Source and ordering | source execution/provider, lineage, source ownership epoch, sequence and completed step |
| Committed bytes | managed artifact reference, SHA-256 digest and size |
| Application identity | application name, checkpoint format/version, immutable executable digest |
| Input identity | content digest, validated against the submitted recipe's managed input |
| Compatibility | explicit integer/SHA-256 Python 3 runtime identifier; same parameters and total steps |
| Retention and access | principal, creation/expiry times and same-principal managed access condition |

A continuation first validates target admission, receipt access/expiry,
format/runtime, immutable recipe/input identity, application parameters and
current artifact bytes. It creates no execution attempt when those checks fail.
The example rejects truncated artifacts, wrong digests, changed input or
executable, incompatible format/runtime and altered parameters before restore.
The receipt becomes inaccessible after expiry; retained local bytes are not
claimed to have been securely deleted.

The optional capability document distinguishes application checkpoints from
process snapshots and engine resume. Requesting unsupported modes fails
explicitly. No checkpoint credentials or secret state are included in this
public-data recipe. Real adapters must apply existing authenticated identity,
secret and placement mechanisms to checkpoint storage; a receipt digest is not
an authorization grant.

### Continuation authority and uncertainty

The cooperating pair shares a durable ownership row
`(lineage, writer_attempt, epoch, result)`. A SQLite `BEGIN IMMEDIATE`
transaction verifies that the receipt source still owns the unfinished lineage,
inserts the successor, advances its epoch and changes the authorized writer.
Concurrent continuation requests from that receipt cannot both win. Each result
publication checks the same writer/epoch under that coordinator's transaction.
The output file is not served without the authoritative committed result
pointer. A source that has already committed a result prevents continuation;
a live source that loses ownership can continue spending CPU but cannot publish
its result. Its eventual TES state is PREEMPTED, and physical exit is reported
separately.

This is atomic **result-writer fencing** in one cooperating deployment. It is
not atomic physical migration or exactly-once computation. The live-source
experiment intentionally observes two running child processes after transfer.
A second demonstration stops a source after a checkpoint and observes child
exit before continuation. `result_fencing: false` reports the unsupported
exclusive-writer guarantee and rejects continuation; it never silently weakens
that guarantee. A termination-only production adapter must advertise that
narrower mode and confirm the source stopped before granting successor commit.
A shared SQLite file is not a production distributed consensus service.

Submission outcome can still be uncertain after target acceptance and before
the client persists its ID. Alpha opts into a unique durable
`(provider, principal, intent) -> (dispatch binding, task ID)` mapping. The binding
includes operation kind, task fingerprint and admission plan ID. Continuation
also binds receipt ID, source lineage/execution/epoch and checkpoint digest.
Identical acknowledged retries adopt the bound task even after admission expiry,
without staging or launching; a changed plan/receipt/task or operation conflicts.
Legacy mappings without a complete binding require explicit resolution. The fingerprint
ignores optional JSON nulls and TES server-owned fields so the existing SDK's
serialization preserves identity. Beta deliberately lacks that contract and
returns 501 for intent reconciliation. On beta, tags, a local dispatch ledger
or an empty list response cannot prove absence of execution. The caller must
retain an unresolved reservation and must not blindly launch a second task.
The [adaptive campaign](../examples/adaptive-campaign/) composes this provider
evidence with an existing durable runtime rather than building a second
scheduler. A target process launch failure after ownership transfer is a failed
linked attempt; ownership is not silently rolled back for a blind retry.

### Observed CPU result

The reproducible demo's 100-step recipe restored at step 20 and matched every
metric, including checksum
`3264f1d2c64c151312b1be0d0199b1762e5ca71598b3b13703d8459673c183c9`.
In the recorded run, the source performed 21 steps, so one step was repeated;
the successor performed only 80 steps. It read 380 checkpoint bytes. Source and
successor checkpoint writes were 759 and 3,035 bytes. Timing is reported by each
run and is not a portable performance claim. The separate live-source race had
one successful continuation and one HTTP 409; the source performed all 100 steps
but its result commit was rejected. These measurements distinguish saved
restart work from duplicated physical work.

## H3-03: reviewable admission and managed staging

A capability lookup can describe support and a router can rank sites; neither
alone binds a submission to a caller's data movement policy. The useful layer
is a server-validated decision envelope. Native [Seqera Intelligent Compute](https://docs.seqera.io/platform-cloud/compute-envs/intelligent-compute/)
combines resource predictions with provisioning policy; [DNAnexus regions](https://documentation.dnanexus.com/developer/api/regions)
apply platform region and permitted-region constraints. This RFC exposes only
a bounded portable admission contract and does not replace those schedulers.

A plan binds the principal, canonical task fingerprint, explicit input identity,
immutable executable digest, selected backend, server policy version and expiry.
It includes a resource profile, allowed data movements and a decision trace for
every candidate. Compute, storage and transfer estimates include their source,
source timestamp, synthetic currency and uncertainty. Size/location uncertainty
is explicit; unknown cost is JSON null, not zero. The unknown-size/location
catalog entries in the example are non-runnable metadata probes. Strict policy
rejects them; a non-strict advisory plan can retain uncertainty but cannot stage
an unknown object.

The provider checks the envelope again at submission. Reusing it for a different
principal/task/backend, after expiry or after a policy change fails before
execution. Managed staging validates the plan after acquiring the same write
transaction that authorizes the copy, so queued requests cannot use a plan that
expired or changed while waiting. Submission stages and persists dispatch under
that lock. Expiry is an admission bound, not a running task lease: current
storage policy and writer ownership, rather than elapsed admission TTL, govern
checkpoint/result publication under their commit transaction. A requested policy can tighten the server's policy, never loosen it.
A failed selected backend produces an explicit error; fallback requires a new
eligible decision under the same constraints. Ordinary TES submission also
applies the server's current policy before managed staging and dispatch. It does
not create a way around the optional admission contract.

[DRS](https://ga4gh.github.io/data-repository-service-schemas/docs/) can supply
content identity and access metadata, but a retrieval method does not prove
current authorization or trustworthy physical location. IP geolocation is not
used as residency evidence. Publication uses the provider's existing managed
storage; this RFC does not invent a required DRS write endpoint.

### Advertise each enforcement boundary

| Constraint | This implementation's enforcement |
| --- | --- |
| Approved execution domain | Checked at admission; no silent reroute |
| Managed input/storage domain | Checked on every staging action under the policy transaction; arbitrary URL/path requests rejected |
| Prohibited managed transfers | Actual copy to forbidden domain rejected before destination data creation |
| Managed checkpoint/output publication | Current storage policy rechecked at checkpoint and result commits |
| Checkpoint continuation domain | Target plan validated before ownership transfer/restoration |
| Supported resource request | One CPU recipe, bounded steps/parameter/delay, small declared memory request; no OS memory quota |
| Reservation / task-count quota | Not implemented; plan reports null, not an implicit reservation |
| Arbitrary application traffic / physical residency | Unsupported; needs operator network/storage isolation and trustworthy domain evidence |
| Guaranteed invoice ceiling | Unsupported; synthetic estimates are advisory |

Policy updates cannot undo an input transfer that was authorized when it
occurred. Staging and policy updates are serialized so new managed transfers
cannot race a committed prohibition. The example runs only its fixed
allowlisted program; it does not sandbox arbitrary applications. Its no-export
claim is restricted to approved selection and managed staging/publication. The
two domain directories and loopback endpoints provide real controlled file
movement but do not establish separate geographic locations.

### Observed placement tradeoffs

All rate metadata is deliberately synthetic. For 80 steps, alpha compute costs
0.016 units and beta 0.008. Beta's 1,024-byte transfer adds 0.0512 units; alpha
therefore wins with total 0.0161024 versus 0.0593024. For 1,000 steps beta's total
is cheaper, but a no-export policy excludes it and selects alpha. After alpha is
marked unavailable, planning fails instead of routing to forbidden beta. Direct
HTTP attempts to stage to beta and to submit there receive 403. Predicted
transfer bytes/costs and actual staged bytes/CPU work are separate report fields.

### Composition with existing work

Consume [SDK #130](https://github.com/ga4gh/ga4gh-sdk/issues/130) capability
preflight and DRS access planning as inputs; the example does not replace them
with inferred authorization. Reuse
[foca-plugins #8](https://github.com/elixir-cloud-aai/foca-plugins/issues/8)
routing and [#21](https://github.com/elixir-cloud-aai/foca-plugins/issues/21)
lookup caching in a real adapter. Observed quantities can extend
[proTES #459](https://github.com/elixir-cloud-aai/proTES/issues/459) accounting;
this example creates no new usage ledger. Preserve
[#285](https://github.com/elixir-cloud-aai/proTES/issues/285) task-count quotas
and coordinate future recorder/thin-gateway integration with undecided
[#391](https://github.com/elixir-cloud-aai/proTES/issues/391). No proTES/FOCA
adapter, broker, pricing crawler, carbon optimizer or GPU fleet is implemented.

## Reproduction and acceptance

Run the commands in the example README. The automated suite exercises actual
HTTP calls and native CPU child processes for successful continuation, live
fencing, corrupt/incompatible/expired/inaccessible checkpoints, plan
principal/task/backend/version/expiry binding, managed staging rejection,
publication policy updates, unknown metadata and unresolved lost responses.
The standalone demo writes a machine-readable evidence report. The existing
GA4GH Python binding and Temporal campaign also consume these endpoints;
optional controls remain separate from standard TES models. Rust generated
models and protocol states are unchanged.
