# Engineering principles for SDK changes

This guide directs implementation decisions in the library, CLI, and Python
bindings. It applies to maintainers and coding agents. Protocol specifications,
issue requirements, and the requested behavior define the contract; these
principles help us implement and verify it. Apply them in proportion to the
change. A documentation correction needs less analysis than a new public API or
an operation that processes remote data.

The [reviewer framework](reviewer_framework.md) covers the review and PR
publication workflow. This guide covers properties to preserve while designing
and changing the code. It does not authorize unrelated refactoring or make
every architectural improvement a prerequisite for an unrelated change.

When you edit these principles, follow the
[technical writing guide](technical_writing.md#instructions-for-agents-and-contributors).
For each requirement, state its scope, action, and completion evidence. Mark
decision prompts as guidance rather than implying they are required checks.
Preserve conditions and exceptions.

Use only the relevant sections for the change at hand; the links below point
to starting points, not a requirement to read this whole file:

| Change touches | Start with |
| --- | --- |
| A service endpoint or public interface | [Contracts](#preserve-protocol-and-downstream-contracts); [downstream use](#design-public-apis-for-downstream-use) |
| API versions or provider differences | [Compatibility](#separate-version-selection-capabilities-and-decoding) |
| Client construction, configuration, or shared behavior | [Ownership and boundaries](#give-state-and-policy-a-clear-owner) |
| Models, serialization, or language conversions | [Representations](#preserve-meaning-across-representations) |
| Errors or failure handling | [Actionable failures](#preserve-actionable-failures) |
| Remote URLs, credentials, or untrusted content | [Inputs and effects](#trace-inputs-to-every-output-and-effect) |
| Logs, traces, metrics, or diagnostic context | [Caller-controlled diagnostics](#keep-diagnostics-safe-and-under-caller-control) |
| Pagination, bulk work, or nested references | [Completeness and partial results](#make-completeness-claims-explicit) |
| Submission, concurrency, cancellation, or retry | [Remote effects and lifetimes](#keep-verification-tied-to-the-actual-effect) |
| Generation, dependencies, or build pipelines | [Reproducibility and build evidence](#evaluate-generation-dependencies-and-build-evidence) |
| Tests, examples, or CI coverage | [Boundary evidence](#prove-behavior-at-the-relevant-boundary) |
| User documentation or instructions | [Public contracts](#design-public-apis-for-downstream-use); [technical writing](technical_writing.md) |
| Maintainer edits to an open PR | [Review-time changes](reviewer_framework.md#maintaining-an-existing-pr) |

The external references have different roles. Protocol specifications define
wire requirements. Language and package guidance explain compatibility rules.
Research papers and engineering frameworks provide methods that we adapt to
this SDK. An informational RFC is not automatically a protocol requirement.
Following selected guidance does not establish compliance with an entire
standard.

Adopt a new rule when it addresses a concrete decision or failure mode. State
when it applies, who enforces it, and what evidence checks it. Extend an existing
rule when it already covers the property. A framework name alone does not
justify a new abstraction, dependency, or required check.

Use Git history and PRs to record revisions to this guide. When changing a rule,
explain the decision it improves and check an applicable case and an exception.
Keep dated assessments and experimental results separate from these instructions.
This guide defines expectations for changed code; it does not certify that all
existing code already meets them.

## Apply the principles to a concrete change

1. Identify the affected user contract. Select the applicable sections above.
   Read the implementation, its callers, and existing tests. Do not infer the
   contract from a single function or comment.
2. Trace one normal path from input to observable result. Trace one plausible
   failure path. Locate the enforcement point for each affected invariant.
   Include CLI and Python adapters when the change affects those surfaces.
3. For a suspected weakness, state its trigger, location, observable consequence,
   and evidence. Classify it as a reproduced defect, static finding, design
   question, or intentional limitation. A large file, clone, duplicate, or
   `unwrap` can guide investigation. Its presence alone does not establish a
   defect.
4. Choose the smallest coherent change that restores the contract. Preserve
   unrelated behavior. Before changing public types, identify the compatibility
   effects. Track wider improvements as separate follow-ups.
5. Verify the result at the boundary where the failure was observable. Record
   the checks performed and the remaining uncertainty in the review report.

For a useful finding, connect **trigger → violated property → enforcement point
→ regression evidence**. An architectural suggestion also needs a concrete
caller or maintenance problem; naming a pattern is not a justification.

## Define the contract and its invariants

Before choosing types or modules, state what must remain true across the whole
operation. Identify the inputs, the effects, the last point where failure is
safe, and evidence that could disprove the claim. For example, if an operation
checks content before sending it, the sent bytes must be the checked bytes; if
preparation fails, remote execution must not start.

For a consequential design choice, use a measurable quality scenario: name the
trigger and operating conditions, expected response, and a measure or test.
Record the scenario and its test or measure in the deciding issue or PR.
Consider reliability, security, interoperability, performance, usability, and
maintainability when they bear on the decision. Record a tradeoff when
improving one property materially weakens another.

For a complex change, a short design record can make the contract reviewable:

```text
Invariant: ...
Quality property and tradeoff, if relevant: ...
Trigger, input, and operating conditions: ...
Trust boundaries and protected effect: ...
Expected observable response: ...
Enforcement point and measure or test: ...
Affected callers and compatibility: ...
Unsupported cases: ...
```

For decisions that will constrain later changes, retain a short architecture
decision record in the deciding issue or PR. Record the context, alternatives,
decision, status, and consequences, including compatibility costs. Link the
record from the affected guidance so later changes can find it. When replacing
a decision, link its successor and mark the old decision as superseded. Routine
implementation choices need no separate record. Do not claim “fully validated”
unless every supported path enforces the stated invariant.

## Preserve protocol and downstream contracts

For a service change, identify the applicable GA4GH specification and version.
Check methods and paths, request and response shapes, optional fields, status
codes, error bodies, pagination, and authentication where relevant. Compare
observed wire behavior with the contract; a generated model or a successful
compile does not prove protocol conformance. [OpenAPI descriptions](https://spec.openapis.org/oas/)
can support generation and testing, while the governing service specification
defines the required behavior.

Distinguish permitted extension data from malformed or ambiguous messages.
Apply the protocol's extension rules; do not infer missing required values to
make a response appear valid. When interoperability requires a provider-specific
exception, isolate its scope, document the evidence and tradeoff, and test both
the exception and normal protocol behavior. Identify when to reconsider the
exception, such as a provider correction or specification update. This adapts
[RFC 9413's active protocol maintenance guidance](https://www.rfc-editor.org/rfc/rfc9413.html):
resolve disagreements explicitly instead of allowing silent repairs to become
an undocumented protocol. Record TRS provider deviations and unverified
interpretations under [Selection and capabilities](api_compatibility.md#selection-and-capabilities);
use the corresponding compatibility guide for another service.

For a public interface change, examine callers in all affected surfaces:

- For Rust, use the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
  for naming, documentation, useful traits, type safety, and error behavior.
  Check the [Cargo SemVer compatibility guide](https://doc.rust-lang.org/cargo/reference/semver.html)
  for changes to exported types, signatures, traits, features, public
  dependency types, and toolchain requirements.
- For the CLI, consider command names, flags, output, exit codes, and scripts
  that consume them. For Python, consider call signatures, returned types,
  exceptions, and examples. A Rust-compatible change may still alter these
  user contracts.

Document intended incompatibilities and migration paths. Test a representative
downstream call or script, and use deprecation when it provides a practical
transition. Apply compatibility guidance to the project's stated support
policy; the external guidelines are not automatic release rules.

## Separate version selection, capabilities, and decoding

For compatibility work, distinguish the pinned source schema, the selected
wire contract, the server's declared version, and the public SDK API. Compare
the actual operations and representations before claiming compatibility. A
larger version number or an additive wire field does not establish Rust source
compatibility. Record supported operations and evidence in the
[compatibility guide](api_compatibility.md), including provider deviations and
unverified interpretations.

When behavior depends on API version, select a protocol profile for each
connection and retain how it was selected. Keep it consistent with the transport
and discovery snapshot. Define what happens when the caller changes the endpoint
or refreshes discovery. A failed decode must not select a more permissive decoder
or replay an operation automatically. Shared behavior needs no extra version
layer solely to conform to this guide.

Treat capabilities separately from versions. A feature absent from an older
specification may be a provider extension; a declared newer version does not
prove that a deployment implements it. Distinguish specified, demonstrated,
disabled, and unknown support where callers need that distinction. Reject
unsupported requested semantics before the affected request. For unestablished
support, require the documented caller policy; never silently discard a filter
or constraint to make the request succeed.

Reuse a model where comparison establishes that it preserves the required
meaning. Generate another versioned model set or add an adapter when an actual
contract difference requires it. Keep public import paths stable where practical.
Do not duplicate a generated tree or decode every response twice solely to
anticipate a future difference. Exercise retained profiles through the public
client, including nested responses, pagination, and simultaneous connections.

Separate version selection from response-validation policy. A response with
malformed optional descriptive metadata can remain usable if a documented rule
preserves the raw value and exposes the unavailable typed value explicitly.
Verify that the operation's result and every returned item's identity remain
meaningful. Test valid, absent, null, and nonconforming values where applicable.
This permission does not cover required execution fields, authorization decisions,
or silently removing failed items from a supposedly complete list. Malformed
syntax and missing data required by the operation remain errors. Tolerance is
an interoperability decision, not a claim of schema conformance.

## Give state and policy a clear owner

Keep protocol behavior in service clients and shared transport facilities;
keep argument parsing, process I/O, and language-specific conversion at their
adapters. A lower-level operation must not require CLI initialization or
decide when a host application exits. Share wire rules such as path construction,
query encoding, and semantic validation when they implement the same contract.
Keep genuine presentation differences in the adapters.

Give each invariant one authoritative owner. If a client holds configuration,
a transport, and discovered service metadata, define which determines its
identity and what happens when that identity changes. Prevent independent
mutation from leaving derived state or cached capabilities inconsistent.
Similarly, a typed view and its raw response need a controlled update path if
both are retained. Cloning immutable values is not itself a consistency defect.

For shared mutable caches or refreshable state, define which operation may
replace or invalidate each value. Associate delayed responses with the state
that produced their requests. An old credential rejection must not invalidate
a newer credential. Test competing completions in both orders with controlled
synchronization; a sequential success case does not exercise that invariant.

Use private fields and validated constructors or update methods for types that
maintain relationships between fields. Public fields remain useful for passive
data records. See the [Rust API Guidelines on private fields](https://rust-lang.github.io/api-guidelines/future-proofing.html#structs-have-private-fields-c-struct-private).
Represent lifecycle states deliberately: construction, discovery, and readiness
need not be the same operation. Do not retain an impossible failure state after
a constructor has already rejected it unless another supported path needs it.

Allow callers to supply transport or operation policy where a supported use case
requires it, and carry that policy through discovery and subsequent requests.
Avoid silently constructing a default client halfway through the operation.
If supplied and SDK-owned policies can conflict, make the resolution explicit.
Reject an unsupported combination before its effect rather than falling back to
an anonymous request or replacing a caller's transport policy. Offline preparation
must not require credentials or discovery unless its documented contract does.

When configuration has multiple sources, define their precedence or mutual
exclusion at one resolution boundary. State what absent, null, and empty values
mean. Reject invalid selected values instead of silently replacing them with
defaults. Test both a supported override and a prohibited conflict where those
cases exist. Resolving configuration does not authorize logging its values.

Contain decisions likely to change behind stable interfaces. For a proposed
module split, name a likely change and the callers that would need edits.
For example, a shared query policy should be changeable without reimplementing
it in each language adapter. Review the affected caller paths to check that
claim. Splitting a file into steps of the same operation does not necessarily
reduce coupling. Prefer composition and narrow helpers; introduce traits,
builders, or layers when they protect a real invariant or enable an actual
caller.

Before publishing a shared abstraction, identify its present consumers and the
common invariant it owns. With one consumer, prefer a local interface or document
why a shared public API is already needed and which assumptions remain
provisional. A provisional label does not remove compatibility costs.

For a new command family, example, or service, define its user, owner, dependencies,
and release lifecycle. Keep GA4GH protocol implementation in the SDK. Higher-level
or provider-specific features need an explicit placement decision, based on
reuse and coupling rather than their ability to call the SDK. An optional feature
still adds maintenance and distribution costs. State whether a component is a
proposal, experimental example, or supported product surface; a runnable prototype
does not establish a production integration contract.

## Preserve meaning across representations

For every conversion, decide what may be lost. Unknown extension fields,
absent versus `null`, enum values, repeated parameters, opaque identifiers, and
exact bytes can matter to different contracts. A typed projection can be useful
without being suitable for forwarding the original payload. Do not silently
turn a pass-through operation into a lossy deserialize/serialize round trip.
Expose raw or extensible representations where preservation is promised;
document intentional normalization elsewhere.

Separate decoding from validation for the requested operation. A partial
response model may legitimately omit fields that creation requires. Successful
deserialization does not establish a valid creation request, and a valid request
model need not represent every response view. Enforce operation requirements at
an entry point used by all relevant callers, before the protected effect.

Constructors must honor supplied values; defaults and convenience conversions
must not silently discard them. Test representative values through construction,
conversion, and the actual serialized request. For extensible payloads, include
an unknown nested field; for partial responses, include omitted and explicit
`null` fields where supported. Compare JSON semantics unless exact byte identity
is part of the contract.

## Preserve actionable failures

Let callers distinguish invalid input, HTTP failure, transport failure,
cancellation, and an uncertain remote outcome without matching message text.

When a public SDK method returns an HTTP failure as an error:

- Preserve the HTTP status in a form the caller can inspect.
- Return the original structured error, or retain it through
  [Error::source](https://doc.rust-lang.org/std/error/trait.Error.html#method.source).
- Keep the headers and bounded response data promised for caller recovery.

Retaining a cause or raw response does not authorize displaying it. Separate
explicit access from default diagnostics. Server bodies and callback errors can
echo secrets that the SDK cannot recognize; bounded text is not necessarily safe
text. Default `Display`, `Debug`, CLI output, and Python exceptions must use safe
classification and context. Do not include arbitrary bodies or format a retained
source chain by default. Test an echoing server or callback through the public
surfaces that can display the error. See the
[OWASP logging guidance on excluded data](https://cheatsheetseries.owasp.org/cheatsheets/Logging_Cheat_Sheet.html#data-to-exclude).

A string containing the original message does not preserve the error's type or
cause. Methods that deliberately return raw HTTP responses have a different
contract. Preserve their response metadata without converting every non-success
status into an SDK error.

When you change an HTTP error path, exercise the public method against a local
server that returns an error response. HTTP 503 is one useful case. Assert that
the caller can inspect the status without parsing the message. If the error is
wrapped, also verify that the original cause remains accessible. Reuse existing
tests when they cover the changed path.

Choose a stable error boundary for each public surface. Map SDK errors to CLI
exit status and Python exceptions intentionally; format human messages at that
boundary. If an error must cross a thread or runtime boundary, use an owned
representation that retains the required classification. Check `Send`/`Sync`
where promised; do not claim them from the result type alone. The
[Rust API Guidelines on errors and thread safety](https://rust-lang.github.io/api-guidelines/interoperability.html)
are relevant here.

Malformed external input and ordinary I/O failures belong in the error contract.
Reserve panics for violated internal assumptions that callers cannot normally
trigger. Do not turn failure into an empty success, fabricated default, or a
secondary parsing error that hides the original cause. Assert the
classification and recovery information, not only a fragment of the message.

When an operation promises a usable task or run handle, validate the required
acknowledgement before returning it. A successful HTTP status alone is insufficient.
If the acknowledgement is unusable after submission, report that the remote
operation may have started. Test a malformed acknowledgement without replaying
the submission; do not classify it as a failure known to precede sending.

## Trace inputs to every output and effect

For a change that crosses a trust boundary, sketch the data flow: actor,
input, boundary, destination, and possible misuse. A short table or diagram is
enough when it makes the [OWASP threat-modeling questions](https://cheatsheetseries.owasp.org/cheatsheets/Threat_Modeling_Cheat_Sheet.html)
answerable. Focus on paths the changed code can actually take.

- Treat user input, configuration, remote responses, fetched documents, and
  acknowledgements as separate inputs. Validate each at the boundary where
  its assumptions become relevant.
- Follow data to outgoing requests, local files, returned values, manifests,
  errors, logs, and examples. Sensitive data can escape through `Display` or
  `Debug` even when the successful result omits it.
- Make URL and credential policy explicit. Validate the complete allowlist
  entry; narrowing a path-scoped entry to its origin broadens access. Check
  redirects and failure paths as well as the first request. Keep authenticated
  service calls within their configured API root.
- Apply configured authentication at its shared request boundary, including
  discovery and custom request builders. Keep service credentials separate from
  object-store or artifact credentials. When a caller supplies a client, document
  which redirect, TLS, timeout, and credential decisions the SDK still owns.
- Preserve structured status and useful context in errors while redacting
  credentials, tokens, and sensitive URL components from diagnostics.

Encode for the destination. A URL path segment, query value, JSON string, log
entry, and terminal table have different rules. Preserve an opaque identifier
in the SDK while preventing server-controlled escape sequences from becoming
terminal instructions; [CWE-150](https://cwe.mitre.org/data/definitions/150.html)
describes this class of weakness. Define human-readable and machine-readable
output contracts separately. Escaping presentation must not silently change the
identifier used in later requests, and redacting logs must not corrupt an
explicitly returned access URL.

For URLs supplied by a user or another service, decide which schemes, hosts,
network ranges, redirects, and credential forwarding the operation permits.
The [OWASP SSRF guidance](https://cheatsheetseries.owasp.org/cheatsheets/Server_Side_Request_Forgery_Prevention_Cheat_Sheet.html)
is a useful reference for destination checks. Local and private services can
also be legitimate SDK targets, so express that choice in the operation's
policy instead of applying an unexplained global rule.

Test the observable sinks, including error text and logs where exposure is
possible. Sanitizing one stored field does not establish that every path is
safe.

## Keep diagnostics safe and under caller control

When adding or changing logs, traces, or metrics, let the host application choose
initialization, exporters, formatting, and filtering. Emit through an
instrumentation interface without installing process-wide handlers.
This applies [OpenTelemetry's library instrumentation guidance](https://opentelemetry.io/docs/concepts/instrumentation/libraries/)
as a design rule; it does not require adding OpenTelemetry to this repository.
Keep ordinary SDK behavior usable when diagnostics are disabled. Instrumentation
failures must not turn a completed remote effect into an apparent SDK failure.

Define what each diagnostic represents: one logical operation, one HTTP attempt,
or a final outcome. Record only the fields needed to diagnose it. Use stable
operation names and bounded categories for metric labels, rather than task IDs,
full URLs, or arbitrary error messages. The
[OpenTelemetry HTTP conventions](https://opentelemetry.io/docs/specs/semconv/http/http-spans/)
provide useful distinctions for attempts and error classification. Apply the
[input-to-output policy](#trace-inputs-to-every-output-and-effect) to diagnostic
fields, including signed URLs and server-provided values. A diagnostic-only
redaction must not alter the value returned to an authorized caller.

Capture diagnostics in tests of the changed success and failure paths. Verify
useful classification, secret removal, and operation/attempt counts where those
are reported. Use an in-process collector or test logger; a deployed telemetry
backend is unnecessary for these checks.

## Make completeness claims explicit

Distinguish complete, partial, unknown, unsupported, stale, and failed states
where callers act differently on them. A missing capability is not necessarily
an unsupported capability; exhausting a request budget is not necessarily the
end of a collection. Preserve successful items and per-item failures in bulk
operations when partial results are part of the contract, with unambiguous
correlation to the original inputs.

For pagination and resumable work, define checkpoint scope: service identity,
query, position within a page, and any version assumptions. Test resumption after
partial consumption and rejection of incompatible checkpoints. Do not promise
snapshot consistency or exactly-once delivery without support from the protocol.

When an API promises a complete set of files, references, or dependencies,
traverse every supported form, including nested and imported content. Parse
according to the format's semantics: the same field name can mean different
things in different contexts. Track that context rather than inferring it from
text matches or a single global flag.

Decide how to handle missing, external, dynamic, cyclic, and unsafe references.
Reject unsupported cases before an irreversible effect. A successful result
must mean the supported graph was closed, rather than that the parser found no
more references. Test both a valid nested case and an unsupported nested case
that must fail.

## Keep verification tied to the actual effect

- Validate or hash the bytes that will actually be sent. Preserve an immutable
  snapshot after verification; reopening a mutable path later can invalidate
  the result.
- Separate preparation from submission when callers need to inspect a plan or
  manifest. Make remote effects explicit in names and documentation. If repeat
  calls can duplicate an effect, say so.
- Distinguish a failure before any attempt, a known response, and an ambiguous
  outcome once sending may have begun. Cancellation before polling a send future
  is not evidence of a remote submission. Conversely, a timeout or lost
  acknowledgement does not prove that the server did nothing. Treat HTTP
  responses according to the service contract; an error status alone need not
  prove absence of side effects.
- State resource limits precisely. A bound on staged bytes does not bound
  response bodies, parsing work, or total transfer. Bounded concurrency does
  not bound queued inputs or accumulated results. Enforce limits while reading
  or producing data, rather than trusting remote length metadata.

Classify each remote operation by its effect and retry semantics. Follow
[RFC 9110's guidance on idempotency](https://www.rfc-editor.org/rfc/rfc9110.html#section-9.2.2):
do not automatically repeat a non-idempotent request. Repeat it only if the
service provides an idempotency mechanism or the client establishes that the
first attempt had no effect. Test when the server receives the request but the
response is lost. If the outcome cannot be resolved, expose that uncertainty
to the caller.

When retries are enabled, choose the layer that owns the attempt policy and
account for retries in lower-level clients. Independent retry loops can multiply
requests and worsen overload. Bound attempts and delays within the operation
budget; use backoff and jitter where appropriate for the service contract.
This adapts the [AWS Builders' Library retry guidance](https://aws.amazon.com/builders-library/timeouts-retries-and-backoff-with-jitter/).
Verify the actual request count through the composed call, including exhaustion
and cancellation. A retry delay or a refreshed credential does not make a non-replayable
operation safe.

Give a composed operation one explicit lifetime and resource budget when the
API promises an overall deadline. Carry the remaining budget through discovery,
retries, body reads, polling, and sleeps instead of restarting it at each step.
Decide what happens to spawned work, open streams, temporary files, and partial
results when a caller cancels or drops the future. Define when local cancellation
takes effect and which background work can continue. Requesting remote
cancellation is a separate action; its acknowledgement need not mean the remote
task has already stopped.
The [Tokio cancellation-safety documentation](https://docs.rs/tokio/latest/tokio/macro.select.html#cancellation-safety)
explains why dropping a future must be considered at each awaited boundary.

Test cancellation before the first request, during I/O, and after remote
acceptance. Test deadline exhaustion during a body read or retry delay, and
cleanup after partial preparation. These cases need different outcomes, even
if each begins with a timeout or cancellation signal.

## Design public APIs for downstream use

Use types and composition to express real states and policies. Document inputs,
side effects, error cases, and whether returned values may contain sensitive
data. Keep configuration and output under caller control where the library is
intended for embedding. A convenience API may choose defaults; make material
policies discoverable rather than requiring downstream code to duplicate the
implementation to change them.

When an extension or implementation claims to substitute for an existing API,
check behavioral substitutability. It must accept inputs admitted by the
shared contract and preserve promised results, invariants, failure semantics,
and state transitions. Matching Rust
trait signatures or Python method names is insufficient. Exercise the same
contract cases through the original and replacement implementations. A language
adapter may intentionally expose a different contract; document that adaptation
instead of claiming interchangeability. This rule does not require inheritance
or a trait for every concrete type.

Check concurrency properties such as `Send` and `Sync` with a downstream
compile probe when they are part of the supported use case. Verify the entire
type path, including shared transports, before promising those properties.

Treat bindings as a maintained product surface. For each SDK API change, decide
whether the CLI or Python surface exposes it, adapts it, or intentionally omits
it. Keep signatures, type stubs, exceptions, supported-surface documentation,
installation steps, and runnable examples aligned. Matching member names in a
stub does not verify returned values or behavioral parity. Follow the
[Python agent guide](../python/AGENTS.md) for runtime, GIL, build, and test rules;
Rust changes do not automatically propagate into hand-written bindings.

When supported behavior changes, update the affected reference text, examples,
and migration instructions in the same change. Identify the authoritative page
for each procedure and link to it instead of maintaining competing copies.
Check the documented behavior against the current implementation and intended
contract; do not rewrite a promise solely to conceal an implementation defect.
Apply the [technical writing guide](technical_writing.md) to the affected prose.
Clear wording and verified technical instructions are separate requirements;
[Google's documentation review guidance](https://google.github.io/eng-practices/review/reviewer/looking-for.html#documentation)
also treats documentation updates as part of a behavior change.

## Evaluate generation, dependencies, and build evidence

When you change a generated model:

1. Identify its source specification, generator configuration, and maintained
   patch.
2. Make the correction in that source or patch.
3. Regenerate the model with the procedure in the
   [generation guide](autogenerate_OAI_models.md).
4. Verify that regeneration preserves the intended change.
5. Verify the affected behavior independently of regeneration.

Do not repair only the generated output. A clean regeneration establishes
reproducibility. It does not establish behavioral correctness. If a constructor
or conversion changes, test its output with representative input values. If
partial-response handling changes, test the supported response views.

For a new parser, transport component, or generator, check maintenance status,
toolchain compatibility, licensing, and relevant advisories. Update the
relevant lockfile when one exists, and test the input shapes this repository
actually uses. Prefer a maintained implementation when two compatible options
meet the same contract. Do not treat compilation alone as proof that a
dependency handles the required formats or failure modes.

Verify the build as it will be consumed. A workspace build can hide missing
features through [dependency feature unification](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification).
Use standalone or downstream
builds for supported feature combinations, toolchains, and extension use cases
affected by the change. Do not infer platform support from a single operating
system, or declare an untested platform broken without evidence.

For a release artifact or build-pipeline change, identify the source revision,
build inputs, toolchain, and actual provenance produced. Record only guarantees
the pipeline provides, with links to the workflow or artifact that proves each
claim. Keep executable release procedures in the CI or release guidance.

Test the artifact through the supported installation path, outside the source
checkout. Inspect its metadata, licenses, included files, and usable imports or
commands. When publishing behavior changes, exercise a test publication where
available and distinguish it from a production release. Build success, upload
success, installation, and runtime behavior establish different properties.
Record package ownership, publisher identity, and approval boundaries separately.
Track temporary ownership or release exceptions with an owner and exit condition;
a temporary arrangement must not silently become the distribution contract.

## Prove behavior at the relevant boundary

Choose tests that could falsify the invariant. Inspect the bytes and metadata
received by a mock server for an upload, or the method, path, status, and
serialized fields for a protocol operation. For rejected input, assert that no
submission request was made. Compile a representative downstream Rust call,
run a CLI subprocess, or exercise a Python binding for the public surface
changed. Use focused unit tests for internal decisions and boundary tests for
externally observable behavior.

For conversions with many input combinations, consider generated property tests,
as in [Proptest](https://proptest-rs.github.io/proptest/intro.html). State the law
independently of the implementation: an accepted identifier stays within one URL
segment, or preserved extension data survives a round trip. Include independent
expected outputs; two incorrect inverse functions can agree with each other.
Retain minimized failures as regression cases. Generated cases supplement known
examples and do not prove the property for every input.

For behavior dependent on a sequence of actions, consider a small reference
state machine, as illustrated by [Hypothesis stateful testing](https://hypothesis.readthedocs.io/en/latest/stateful.html).
Examples include consuming a page, saving a checkpoint, and resuming, or
cancelling before and after submission. Check outcomes and allowed effects after
each transition. Use controlled clocks and server synchronization for timing
cases. Choose these techniques when their additional coverage justifies the
maintenance cost; they do not mandate new test dependencies for ordinary edits.

Mock collaborators or the remote service, not the operation the test claims to
verify. Ask whether breaking the production behavior would make the test fail;
asserting a value returned by a replacement of that same method proves little.
Prefer deterministic local fixtures for repeatable failure cases. Use live
services for interoperability evidence that mocks cannot supply, with explicit
prerequisites and bounded side effects. Do not add brittle live dependencies to
default tests solely to increase apparent coverage.

State the limits of each kind of evidence:

| Evidence | What it can establish | What it does not establish alone |
| --- | --- | --- |
| Static trace | A reachable path and its enforced assumptions | Runtime or deployment behavior |
| Compile or stub check | Tested signatures, traits, features, or declared members | Correct requests, return values, or complete binding coverage |
| Behavioral unit test | The decision exercised with the chosen inputs | Correct integration across an unexercised adapter |
| Property or state-machine test | A stated law or transition checked over generated cases | A formal proof, or correctness of an incomplete reference model |
| Mock-backed boundary test | Actual requests, conversions, output, and failures exercised | Conformance of a real server |
| Live integration run | Interoperability with the named server, revision, and environment | All deployments, protocol versions, or failure modes |
| Regeneration or artifact check | Reproducible output or inspected package contents | Correct semantics of generated code |

Keep enforcement claims accurate. Guidance, a local check, and a required CI job
provide different assurance. Inspect commands, feature flags, path filters,
skips, and the final gate before saying CI protects a property. Trigger an
expensive suite when it exercises the affected behavior; adding a broad path
filter does not create missing test coverage. Keep suite commands and setup in
the [integration guide](../tests/README.md), rather than duplicating them here.

For CI scheduling changes, define the affected inputs and event classes. If
classification is uncertain, run the broader applicable checks. Keep
fast contract checks on the feedback path and place broader checks where their
risk coverage justifies the cost. A required gate must distinguish intentional
skips from missing, failed, or cancelled applicable jobs. Verify both success
and failure combinations, including manual and main-branch runs when used.
Document how to request expensive checks before merge for relevant changes.
Measure job count or runner time when cost is the reason for the change.

For rechecks after integration, CI inspection, and review conclusions, follow
the [reviewer framework](reviewer_framework.md#maintaining-an-existing-pr).
Earlier evidence applies only while the behavior and assumptions it exercised
remain unchanged.

For a runnable example or fixture collection, provide a discoverable entry point
with its purpose, prerequisites, working directory, inputs, command, and expected
result. Identify external sources and pin revisions or digests where reproducible
behavior depends on them. Check links and files in the distributed package as
well as the repository when both are supported. Keep historical logs tied to
their source revision; they do not validate later code.

## Further reading

The rules above adapt selected methods. These sources can help with a design
decision; reading every source is not a required step for each change.

- Quality scenarios: [SEI method](https://www.sei.cmu.edu/documents/704/2003_005_001_14213.pdf#page=55)
  and [ISO/IEC 25010](https://www.iso.org/standard/78176.html).
- Decision records and module boundaries:
  [Nygard](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions),
  [Parnas](https://doi.org/10.1145/361598.361623), and
  [Google's review guidance](https://google.github.io/eng-practices/review/reviewer/looking-for.html).
- Substitutability: [Liskov and Wing](https://www.cs.cmu.edu/~wing/publications/LiskovWing94.pdf).
- Development controls and artifact provenance:
  [NIST SSDF](https://csrc.nist.gov/pubs/sp/800/218/final) and
  [SLSA build track](https://slsa.dev/spec/v1.2/build-track-basics).
