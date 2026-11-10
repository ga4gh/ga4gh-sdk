# H3-06 interoperability evidence — 2026-11-10

These are target-specific observations, not a compatibility badge. The
[index](index.json) links exact source revisions and outcome counts; each run
has JSON and JUnit. All recorded SDK source states are clean. Source checksums,
fixture digests and configured/resolved runtime image identities remain in the
individual reports.

- **Direct SDK/Funnel:** real staged input, stdout/stderr, successful and failed
  executors, and API cancellation pass. An independent container check observed
  one container initially, then zero after 1.044 seconds. Whole-worker cleanup
  remains unsupported.
- **proTES/Funnel:** the unchanged pinned upstream real-backend integration
  script passes, including input staging, logs, failure and cancellation.
- **Nextflow/nf-ga4gh/proTES/Funnel:** the final run passes seven protocol/storage
  checks: fixture identity, plugin artifact, real execution, shared nested
  directory/glob bytes, stdout/stderr selection, inspected path prefix, and
  interruption of an acknowledged running task to `CANCELED`. Its independent
  physical-container check **fails**: one task container still ran after the
  15-second observation budget. Both proTES and Funnel subsequently reported
  `CANCELED`; a later bound-to-task operator probe found zero running containers.
  The earlier passing snapshot is retained separately and does not supersede
  this final failure.
- **Native WES offline:** actual SDK multipart/status/log/cancel behavior passes
  for both the fixed attachment and public-hello request shape. Simulation does
  not prove Seqera execution.
- **Native Seqera live:** authenticated discovery succeeds. Exactly one freshly
  approved public-hello submission returns **HTTP 400**, without an acknowledged
  run ID. Evidence is `real_service`, not `real_execution`. The private intent
  ledger remains `SUBMISSION_OUTCOME_UNKNOWN`; no POST retry or guessed
  cancellation occurred. Status/log/cancel of a real accepted run remain
  unexercised. No provider/client defect or absence of execution is inferred
  from that response alone.

The local combination is Funnel 0.12.3 / binary revision `738ab75a`, proTES
source `55c4d1591cd3be883ad0b4fd2da01d49976414cb` built on its pinned FOCA base,
Nextflow 26.10.5, Temurin 21.0.13+9, and published nf-ga4gh 1.5.0. The plugin
release revision is `23bcecd673dffcf5706b324494ab89ac1efd170c`; the referenced
later documentation source is `00c313135a88eca3a4b0de1bfa6475eb5a63f8fe`.
The final Nextflow report explicitly inherits TES discovery from the same pinned
proTES instance/report, which advertises TES 1.1.0 and implementation
`0.0.0+h3.55c4d15`, not the older phase-two `0.0.0.dev0` deployment.

Existing nf-ga4gh #2 (object/nonshared storage) is unsupported; #6 (last-attempt
log selection) and #12 (TES IDs in WES TaskLog) are not run. The valid fixture
only characterizes the inspected directory/glob mapping and bytes relevant to
#7/#11. No upstream issue fix is claimed.

Commands and prerequisites are in [provider-profiles.md](../../provider-profiles.md).
Synthetic credentials expire: generate a fresh fixture token before a new
Nextflow run. The observed stale-token HTTP 401 was a harness prerequisite
failure and is not provider-defect evidence. The initial bad endpoint prefix,
unshared work path, unspecified directory type and optional metrics request
were corrected before the selected final execution evidence.

Standard workspace fmt/Clippy/build/tests passed during implementation; final
affected Rust example compilation/Clippy, six profile unit tests and both SDK
multipart fixture variants passed. Existing repository warnings remain.

Forward these artifacts to the existing TES compatibility matrix/testbed and
linked issue scopes. No upstream publication or replacement conformance suite
was created by this task.
