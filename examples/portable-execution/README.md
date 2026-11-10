# Portable application checkpoint and admission experiment

This opt-in Python 3.10+ example executes real CPU subprocesses through two
loopback TES HTTP targets. It requires only the Python standard library. The two
logical execution/storage domains share one host and one durable coordinator.
They are not cloud regions. The `ga4gh-h3-cpu:v1` image string labels an
allowlisted recipe; it does **not** mean Docker or container isolation.

From the repository root:

```sh
python3 -m unittest discover -s examples/portable-execution -p test_provider.py -v
python3 examples/portable-execution/demo.py --output /tmp/h3-provider-demo.json
```

The second command runs the public integer-sequence workload, preempts one
attempt after a committed checkpoint, restores on the other target, and asserts
byte-equivalent typed metrics against an uninterrupted run. It then races two
continuations of a live source, observes physical overlap, and proves that the
fenced source cannot publish a conflicting result. Its JSON report retains
checkpoint bytes, restore progress, work repeated, observed child status,
synthetic estimate traces and actual managed staging rejection.

The public input is the 1,024-byte sequence of integers 0..255 encoded as big
endian unsigned 32-bit words. A seeded, explicit integer LCG supplies RNG state;
300 SHA-256 iterations are performed per step. Checkpoints contain completed
step, RNG state and accumulator. The illustrative response score
`-(parameter - 3) ** 2` supports a small deterministic campaign; it is not a
scientific performance benchmark. Each subprocess reads and verifies the bytes
written by managed staging. Execution is bounded to 10..1,000 steps, parameter
-100..100 and delay 0..100 ms. Arbitrary commands, images, environment, volumes,
input URLs and output URLs are unsupported.

For an interactive controller, start the private backend pair:

```sh
python3 examples/portable-execution/provider.py serve --root /tmp/h3-provider-state
```

The printed manifest contains two `http://127.0.0.1:<port>` base URLs. Existing SDK
TES clients use a base ending in `/v1`; the optional controls use
`/control/v1/...` on the same origin. The pair is intentionally loopback only:
it is a private backend experiment, not a user-facing website. Shut down with
Ctrl-C. `Cluster(Path(...))` supports embedding in the adaptive campaign. Use a
new/dedicated directory for each demo run; the standalone service retains task,
intent, plan and receipt metadata across restarts.

Read [CONTRACT.md](CONTRACT.md) for endpoints and
[the provider RFC](../../docs/horizon3-provider-contracts.md) for assumptions,
field meaning, prior art and enforcement boundaries. Run artifacts belong in a
user-selected directory, not in Git.

## Guarantees exercised

| Behavior | Evidence |
| --- | --- |
| Real asynchronous task create/get/list/cancel | Separate PID, loopback HTTP, terminal protocol state and observed child exit |
| Durable app checkpoints without a connected client | Client closes, worker continues and publishes committed receipts |
| Correct continuation | Final checksum and all typed metrics equal the uninterrupted run; restored progress > 0 |
| Competing requests | Atomic coordinator ownership CAS; one new writer; loser 409 |
| Stale live source | Still physically running after transfer; final result commit rejected; PREEMPTED |
| Invalid restore data | Truncated bytes, wrong digest, input/executable mismatch, format/runtime incompatibility and parameter change rejected before new task |
| Access and retention | Other principal denied; expired checkpoint 410; expired plan 409 |
| Submission binding | Other principal/task/backend, changed policy and expired plan rejected |
| Placement tradeoff | More expensive alpha compute wins on total for short tasks; cheap beta total wins for long tasks until no-export excludes it |
| Managed transfer enforcement | HTTP staging request to forbidden domain rejected before directory/data creation |
| Policy during execution | Changed storage policy prevents result publication |
| Unknown data metadata | Strict plan rejects unknown size/location; non-strict plan returns null cost and explicit uncertainty |
| Failure/fallback | Unavailable allowed site does not authorize forbidden fallback |
| Lost acknowledgement | Alpha unique durable intent mapping supports adoption; beta reports 501 and the client leaves the outcome unresolved |

## Boundaries

`X-H3-Principal` is a trusted-loopback test identity, not an authentication
system. Production adapters need authenticated principal derivation and storage
ACLs; possession of a digest does not confer access. The operator endpoints for
policy/availability exist only for this test. This program does not execute
untrusted code and is not a production TES implementation.

The shared SQLite coordinator serializes continuation and result publication.
It proves exclusive authorized result commit in this cooperating deployment,
not exactly-once physical computation or distributed high availability. The
live-source demonstration intentionally permits overlapping CPU work. A target
launch failure after ownership transfer remains a failed linked attempt; the
service never rolls ownership back and blindly retries. On a supervisor restart,
formerly active child status becomes `UNKNOWN_AFTER_RESTART`; persisted intent
records can identify a task, but PID reuse is never used as termination proof.

Managed input copies and result publication are rechecked under the current
server policy. These controls do not constrain arbitrary application network
traffic, establish physical/geographic residency, reserve capacity, enforce a
memory/currency quota, or cap an invoice. Supporting arbitrary applications with
a no-export guarantee requires operator network and storage isolation. Estimate
rates and runtime assumptions are explicitly synthetic; actual observed bytes,
work and child exits are recorded separately. Expiry blocks access/restore; it
is not proof of secure deletion of expired local bytes.
