# SDK engineering assessment — 2026-11-10

This is a read-only assessment used to refine the
[engineering principles](engineering_principles.md).
It records concrete follow-up candidates separately from the general guidance.
[Tracking issue #176](https://github.com/ga4gh/ga4gh-sdk/issues/176) owns remediation status; this document preserves the
assessment evidence rather than duplicating a live task list.
It is not a claim that the repository has passed an exhaustive audit, or a list
of prerequisites for an unrelated PR. No source fixes were made in the original
assessment pass. Findings 1–9 below describe that snapshot, not current defects.
The [publication reconciliation](#publication-reconciliation--2026-11-12)
records later outcomes separately.

## Scope and evidence

- Main reference: commit
  [`8b8030f`](https://github.com/ga4gh/ga4gh-sdk/commit/8b8030f0c9e2576b68f46bb46db10a6dae6c444a).
  Source links below are pinned to this assessment snapshot.
- Additional reference: commit
  [`103d773`](https://github.com/ga4gh/ga4gh-sdk/commit/103d7737f9ac665bf76b3b73c4f43150f306e376),
  for TRS/WES bundle preparation and submission. Findings below refer to the
  main snapshot unless explicitly marked otherwise.
- After [#142](https://github.com/ga4gh/ga4gh-sdk/pull/142) merged, the documentation
  branch was based on main commit
  [`4115af9`](https://github.com/ga4gh/ga4gh-sdk/commit/4115af9bb785b7f8441e058d65101dcc106835d0).
  The diff from the assessment snapshot leaves the source locations for findings
  1–9 unchanged. The probes were not rerun after that merge; reproduce the relevant
  case on the implementation branch before fixing it.
- Structural survey: all service-client families, shared transport and operation
  utilities, configuration, CLI commands, Python wrappers and runtime, model
  generation, manifests, CI, and integration runners. Generated models and test
  bodies were inspected selectively after inventorying them; this was not a
  line-by-line audit of every generated file or test.
- Targeted runtime probes: a temporary downstream Rust crate exercised the
  model constructor, WES/TES errors, cancellation, and the CLI dispatcher against
  loopback HTTP servers. It did not contact real GA4GH services.
- Not run: the complete repository test suites, model regeneration, Docker
  suites, live-service integration, downstream PyO3 builds, or other platforms.
  Their configuration and existing test code were inspected where relevant.

A temporary downstream Rust crate was built with `cargo build --offline` and
executed with local sockets allowed. Its dependencies were the assessed library
and CLI via path dependencies, `serde_json`, and Tokio. The CLI probe invokes the
public dispatcher and inspects its returned stdout; it is not a CLI subprocess
or terminal-emulator test. The triggering inputs and observations are recorded
below; turn them into repository regression tests when implementing the fixes.

Observed probe output:

```text
constructor_keeps_executor=false
typed_roundtrip_keeps_extension=false
wes_status_retains_http_error=false
wes_status_has_error_source=false
tes_status_retains_http_error=true
cancel_before_send_reported_ambiguous=true
cli_create_keeps_extension=false
cli_stdout_contains_raw_escape=true
```

## Reproduced behavior

### 1. The TES task constructor discards its executors

**Location:** [TesTask::new](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/tes/models/tes_task.rs#L57)
and the maintained [generation patch](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/utils/patches/tes.patch#L31).

Passing a nonempty executor vector produces `executors: None`. The patch makes
the field optional for partial response views, but also replaces the constructor
assignment with `None`. The probe reports `constructor_keeps_executor=false`.
This is a concrete constructor defect, independent of remote service behavior.

**Direction:** correct the maintained patch and regenerate, retaining the
supplied vector while continuing to decode partial responses. Add a behavioral
test of constructor output and request serialization alongside a partial-response
case. A no-diff regeneration check alone cannot catch this defect.

**Principles:** representation fidelity; separate response decoding from request
validation; generated semantics require independent evidence.

### 2. WES wrappers erase the original HTTP error

**Location:** [Run::status](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/wes/mod.rs#L43),
with related conversions in `Run::cancel`, service initialization, and
`WES::list_runs` in the same module.

For a mocked HTTP 503, `Run::status` returns an error that neither downcasts to
`HttpError` nor exposes an error source. The corresponding TES status call
retains `HttpError`. Formatting the original error into a new string loses the
machine-readable status and cause, complicating caller recovery decisions.
The runtime probe covers status; the other conversions were inspected statically.

**Direction:** propagate the original error or use a contextual wrapper with an
inspectable cause and classification. Test status/cause retention through each
affected public wrapper. Review compatibility where callers depend on the old
error type or wording; avoid replacing every SDK error type in one sweep.

**Principles:** actionable errors; consistent semantics across adapters.

### 3. TES CLI creation silently drops extension fields

**Location:** [CLI create path](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/cli/src/tes.rs#L111).

A valid task JSON containing an additional nested `extension_field` reaches the
server without that field. The CLI deserializes into `TesTask` and calls typed
`TES::create`; the probe checks the actual POST body. SDK `create_value` and the
Python create wrapper retain the raw payload instead.

This demonstrates loss and a difference between surfaces. It does not by itself
establish that the CLI currently promises arbitrary extension passthrough.

**Direction:** decide the intended CLI contract explicitly. If supplied task
JSON is to be forwarded with extensions, use the existing raw-value path with
shared request validation and assert the received JSON. If the CLI deliberately
accepts only a projection, make the restriction visible rather than silently
implying preservation. Keep Python and Rust differences documented.

**Principles:** explicit conversion loss; shared protocol policy; compatibility.

### 4. TES CLI output passes a server-provided escape sequence through

**Location:** [creation output](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/cli/src/tes.rs#L119)
and [task table formatting](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/cli/src/tes.rs#L282).
Compare the existing [registry output helpers](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/cli/src/registry.rs#L101).

A server acknowledgement with an ID containing an ESC sequence results in that
raw ESC in `CommandOutput.stdout`. The probe establishes byte propagation,
not exploitation of a specific terminal. This can affect terminal presentation;
it is not evidence of arbitrary code execution. Table and diagnostic sinks need
the same policy review, but were not all exercised by this probe.

**Direction:** specify safe terminal rendering and any explicit raw or machine
output mode without changing the underlying opaque SDK identifier. Reuse a
presentation policy across CLI commands where appropriate. Test actual output
and confirm that subsequent HTTP paths still use the original identifier.

**Principles:** destination-specific encoding; distinct data and presentation
contracts; observable sink tests.

### 5. Cancellation before sending is classified as an ambiguous submission

**Location:** [Transport::send_once](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/utils/transport.rs#L359)
and [Operation::run](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/utils/operation.rs#L34).

With an already-cancelled operation, `send_once` returns
`AmbiguousSubmissionError`. `Operation::run` checks cancellation before polling
the supplied send future, so this path has made no submission attempt. Wrapping
all operation failures as ambiguous overstates uncertainty and can force callers
to reconcile an execution that could not have started through this call.

**Direction:** distinguish failures known to precede an attempt from interruption
after sending may have begun. Preserve ambiguity in the latter case. Test both
zero-request cancellation and a server accepting a request before its response
is lost; do not infer safe retry from cancellation alone.

**Principles:** explicit operation transitions; actionable failures; retry safety.

## Static findings and design decisions

### 6. Query serialization has multiple owners with different rules

**Locations:** [transport query encoding](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/utils/transport.rs#L207),
[Python conversion](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/python/src/utils/convert.rs#L99),
and [pagination encoding](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/pagination.rs#L193).

The transport rejects `null` inside an array; Python and pagination skip it.
Pagination stringifies objects where the other encoders reject them. The Python
TES list wrapper also retains a comment saying the SDK cannot encode repeated
tag parameters, although the transport now has that support.

**Direction:** decide which differences are intentional, then consolidate the
shared wire policy or explicitly adapt it. Preserve each caller's raw-response
requirements when changing delegation. A parameterized set of null, scalar,
repeated, nested, and reserved-character cases should exercise the relevant
public paths. The finding is policy drift, not a rule to remove all duplication.

**Principles:** one policy owner; intentional adapter differences; evidence at
the transmitted request.

### 7. Public duplicated state can lose internal consistency

**Locations:** client fields in
[TES](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/tes/mod.rs#L175),
[DRS](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/drs/mod.rs#L16),
WES, and TRS; [DRS reference construction](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/drs/resolution.rs#L107);
[Python typed/raw task fields](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/python/src/clients/tes.rs#L165).

Client configuration, transport configuration, and discovered service metadata
are separately mutable. DRS reference construction uses the client's config,
while network requests use the transport's config; changing only one can make
the reference describe a different root. Python extension authors can likewise
mutate the public Rust `inner` and `raw` task fields independently, even though
different accessors consult different representations.

Constructors also perform discovery with a separately created transport.
[ServiceInfo](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/lib/src/clients/serviceinfo/mod.rs#L22)
does not expose transport injection, making custom policy harder to carry
through discovery. This is an extensibility concern, not proof that default
construction sends incorrect requests.

**Direction:** identify the authoritative state and supported mutation lifecycle;
introduce controlled construction/update paths where required. Carry supplied
transport policy through discovery. Plan a compatibility migration before making
public fields private. Avoid a wholesale client hierarchy rewrite or treating
every clone as a defect.

**Principles:** invariant ownership; explicit lifecycle; caller-controlled policy.

### 8. Some Python tests replace the operation they claim to verify

**Location:** [test_all.py](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/python/tests/test_all.py#L17).

The service-info and HTTP-method tests patch the public method being invoked and
assert its configured mocked return. They can pass when the real method is
broken. This does not apply to the whole Python suite: other tests execute
wrappers against a mock HTTP service, and the configuration test in this file
does call production code.

**Direction:** replace redundant mock-return tests with actual wrapper calls to
the local test server, or remove them when existing behavior tests already cover
the contract. Assert method, path, data, and observable exception behavior.

**Principles:** mock collaborators; tests must be capable of detecting the defect.

### 9. Documented checks and actual CI coverage are distinct

The [generation guide](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/docs/autogenerate_OAI_models.md#L29)
explicitly says regeneration is not checked in CI. The
[workflow](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/.github/workflows/ci.yml)
runs on Ubuntu; it does not exercise every downstream feature combination.
[Stub tests](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/python/tests/test_stubs.py)
compare names and parameters and check package contents, but cannot establish
runtime return semantics or that a newly added Rust API should be bound.

These are limits of the evidence, not newly demonstrated build failures. The
[Python agent guide](https://github.com/ga4gh/ga4gh-sdk/blob/8b8030f0c9e2576b68f46bb46db10a6dae6c444a/python/AGENTS.md)
already makes the supported-surface decision and downstream PyO3 check explicit.

**Direction:** add automation for stable, promised properties where justified,
and label the remaining manual checks accurately. Route integration suites by
behavior they actually exercise; changing a path filter does not add tests for
a new capability. Do not require a platform matrix without first establishing
the support policy.

**Principles:** claim only actual assurance; binding lifecycle; proportional CI.

## Existing designs worth preserving

| Design | General property to retain |
| --- | --- |
| Bounded response reads with structured status and headers | Enforce limits on consumed data and retain recovery information. |
| Shared absolute operation deadlines | Include retries, sleeps, and body I/O in the same promised budget. |
| DRS bulk correlation and partial outcomes | Preserve identity and successful work when other items fail. |
| Scoped pagination checkpoints | Validate resume context and avoid unsupported consistency claims. |
| Capability states distinguishing uncertainty | Do not turn missing or stale information into a false definitive answer. |
| Immutable prepared bytes in the additional bundle reference | Submit the bytes that were verified; separate preparation from execution. |
| Registry rendering helpers | Encode untrusted values at presentation sinks. |
| Python shared runtime, GIL release, and package stub checks | Treat runtime behavior and installed-package usability as binding contracts. |

## Follow-up ownership

The documentation delivery is tracked in
[#178](https://github.com/ga4gh/ga4gh-sdk/issues/178). The table maps findings to
their issue owners; it does not duplicate completion status.

| Finding | Issue | Relationship |
| --- | --- | --- |
| 1 | [#95](https://github.com/ga4gh/ga4gh-sdk/issues/95) | Already tracks the ignored constructor argument and model extensibility. |
| 2 | [#100](https://github.com/ga4gh/ga4gh-sdk/issues/100) | Coordinate narrow WES error propagation with typed errors and thread-safety; a partial fix does not close the whole issue. |
| 3 | [#179](https://github.com/ga4gh/ga4gh-sdk/issues/179) | Define and preserve the CLI task-input contract, independently of the generated-model changes in #95. |
| 4 | [#180](https://github.com/ga4gh/ga4gh-sdk/issues/180) | Define safe output rendering and machine consumption without changing opaque identifiers. |
| 5 | [#181](https://github.com/ga4gh/ga4gh-sdk/issues/181) | Classify pre-attempt outcomes; coordinate with the configurable budgets in [#173](https://github.com/ga4gh/ga4gh-sdk/issues/173). |
| 6 | [#182](https://github.com/ga4gh/ga4gh-sdk/issues/182) | Resolve query-policy differences and consolidate the shared semantics. |
| 7 | [#183](https://github.com/ga4gh/ga4gh-sdk/issues/183) | Decide state ownership and compatibility, then track accepted implementation changes separately; reuse #100 and #173 for their existing scope. |
| 8 | [#184](https://github.com/ga4gh/ga4gh-sdk/issues/184) | Replace tests that mock the tested method, or remove them where real coverage already exists. |
| 9 | [#94](https://github.com/ga4gh/ga4gh-sdk/issues/94), [#116](https://github.com/ga4gh/ga4gh-sdk/issues/116) | Downstream CI and lint enforcement; triage any remaining gaps in #178. Neither alone proves generated semantics or binding parity. |

These findings are assessment evidence, not a claim that all of them are newly
discovered. Reuse the existing issues when their acceptance criteria fit, and
create focused follow-ups for gaps rather than duplicating the same work.

## Publication reconciliation — 2026-11-12

This reconciliation uses main commit
[`fc94305`](https://github.com/ga4gh/ga4gh-sdk/commit/fc94305b358c66f82d53c5352845a10bb17ee56d).
It compares the merged changes, current implementation, and regression-test
sources with the original findings. The original downstream probes were not
rerun unchanged. Their output above remains evidence for the original snapshot.
This table is dated evidence, not a second live checklist; linked issues own
subsequent status.

| Finding | Outcome at publication base | Evidence and remaining boundary |
| --- | --- | --- |
| 1. TES constructor | Corrected by [#192](https://github.com/ga4gh/ga4gh-sdk/pull/192); #95 is closed. | `lib/tests/model_unknown_values.rs` checks supplied executors and unknown nested fields. |
| 2. WES error propagation | Status, cancellation, and listing retain `HttpError`. The wider #100 remains open. | `test_run_and_list_errors_keep_the_http_error` in `lib/src/clients/wes/mod.rs` checks classification and raw body access. `WES::with_transport` still converts discovery errors through `access_error`; this is not a completed SDK-wide error migration. |
| 3. CLI task JSON | [#204](https://github.com/ga4gh/ga4gh-sdk/pull/204) closed #179 after model extension support landed. | `cli/tests/output.rs` checks actual file/inline/stdin request JSON, including extensions, nulls, and numeric representation. |
| 4. Terminal output | [#195](https://github.com/ga4gh/ga4gh-sdk/pull/195) closed #180. | `cli/tests/output.rs` checks safe rendering and opaque-ID handling. |
| 5. Pre-send cancellation | [#186](https://github.com/ga4gh/ga4gh-sdk/pull/186) closed #181. | `lib/tests/operation_policy.rs` distinguishes pre-attempt failure from an uncertain submission. |
| 6. Query policy | [#199](https://github.com/ga4gh/ga4gh-sdk/pull/199) closed #182. | `lib/tests/query_encoding.rs` and `python/tests/test_query_encoding.py` cover shared semantics. |
| 7. State ownership | Partially addressed; #183 remains open. | [#213](https://github.com/ga4gh/ga4gh-sdk/pull/213) controls TRS connection state; [#202](https://github.com/ga4gh/ga4gh-sdk/pull/202) and [#220](https://github.com/ga4gh/ga4gh-sdk/pull/220) carry injected transport policy. TES/WES/DRS public state and Python typed/raw mutation still need the scoped decisions in #183. |
| 8. Self-mocked Python tests | [#193](https://github.com/ga4gh/ga4gh-sdk/pull/193) closed #184. | `python/tests/test_all.py` exercises the real wrappers against the local fixture. |
| 9. Build evidence | Coverage, package builds, and CI scheduling have advanced; this was an evidence inventory, not one defect. | [#124](https://github.com/ga4gh/ga4gh-sdk/pull/124), [#187](https://github.com/ga4gh/ga4gh-sdk/pull/187), and [#229](https://github.com/ga4gh/ga4gh-sdk/pull/229) changed checks. [#219](https://github.com/ga4gh/ga4gh-sdk/pull/219) added Codecov upload and the badge; #87 is closed. [#94](https://github.com/ga4gh/ga4gh-sdk/issues/94) and [#32](https://github.com/ga4gh/ga4gh-sdk/issues/32) retain broader scope. Reconcile their checklists against merged evidence when taking the next task. |

The paths in this table refer to the
[pinned publication tree](https://github.com/ga4gh/ga4gh-sdk/tree/fc94305b358c66f82d53c5352845a10bb17ee56d).
Read their current equivalents when starting new work. A merged partial fix
or a closed neighboring issue does not establish completion of a broader contract.

The documentation PR can close #178. State-ownership decisions (#183), the wider
error redesign (#100), and comparative instruction evaluation (#185) keep their
own delivery boundaries under the rollout. Publication does not close #176.
