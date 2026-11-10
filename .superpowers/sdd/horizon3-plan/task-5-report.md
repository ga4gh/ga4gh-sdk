# Task 5 — H3-06 provider profiles

Branch: `feat/horizon3-provider-profiles`. Core implementation through `a694463`; public rebuild/repinning docs through
`b6af042` and requested latest phase-two dependency merged;
all changes are in the assigned worktree. Controller owns PR/upstream reporting.

Implemented machine-readable direct Funnel, proTES/Funnel, nf-ga4gh/proTES and
native Seqera WES profiles; SHA-256 fixture seeding; distinct JSON/JUnit outcome
and evidence levels; unchanged upstream proTES suite wrapper; disposable pinned
local-stack templates; dedicated opt-in SDK TES test; bounded native WES SDK
example through public `Transport::with_client` and existing multipart calls.
No generated models, standard API fields, tests/README or unrelated worktrees
were changed. Configuration bearer fields are not relied upon (#31).

Final artifacts: `docs/review/horizon3-interop/index.json`, per-profile JSON/JUnit
pairs and README. Every selected report records its actual clean source revision
and checksums; earlier valid passing observations are labelled separately.

## Observed acceptance

- Direct SDK/Funnel staged input, success/failure, logs and API cancel pass.
  Physical container count 1→0 was separately observed after 1.044 seconds.
- Exact upstream proTES real integration suite passes on real Funnel.
- Final Nextflow protocol/storage checks pass: shared work path, actual nested
  directory/glob output bytes, separate selected stdout/stderr, inspected glob
  prefix and directory type, known RUNNING task interruption→CANCELED.
- Final independent Nextflow physical check fails: one owned task container was
  still running after 15.149 seconds while proTES/Funnel both reported CANCELED.
  Later private task-bound observation found zero containers. Preserve this
  failure; no stable immediate physical cancellation guarantee or defect
  attribution is claimed.
- Native offline attachment and hello multipart/status/log/cancel checks pass.
  Actual native discovery is authenticated and advertises WES 1.0, NFL with no
  descriptor versions, and Nextflow 20.01.0.
- Exactly one freshly human-approved live hello POST returned HTTP400 without
  run acknowledgement. Evidence is real_service. Intent remains UNKNOWN,
  no retry/guessed cancel. Live accepted execution/status/log/cancel criteria
  remain unfulfilled; no paid-resource or non-execution claim is inferred.
- Acceptance/QUEUED/cancel alone stays real_service; observed RUNNING/COMPLETE
  has a separate assertion. Six unit tests include this regression. Independent
  reviewer verified lost-response no-retry/0600 ledger, existing-ledger refusal,
  redirect-stop and queued-cancel behavior against a loopback fixture.

## Exact scope limits

nf-ga4gh #2 nonshared/object storage unsupported; #6 last TesTaskLog across
attempts not run; #12 TES IDs in WES TaskLog not run. #7/#11 cover only the
actual valid fixture mapping/bytes; no issue fix is claimed. No DRS compliance,
production container/cloud compatibility, whole-worker shutdown or generic
certification is claimed. Native hello revision/resource overrides are not
invented; expected repository HEAD and unverified enforcement are explicit.

## Validation and lifecycle

Workspace fmt/check, Clippy, build and tests passed; final affected example built
and passed focused Clippy, six Python tests and both offline protocol variants.
Baseline generated-code/Python warnings remain. A deleted external phase-two VM
forced a new isolated `colima-ga4gh-horizon3` stack; old unknown submission
context was not replayed. Built exact source pins and used separate database,
loopback forwards and one Celery worker; never changed default VM or other apps.
Own stack containers/volumes and both explicit forwards are removed; parent
scenario API/VM/shared SSH mux remain. See exported `cleanup.json`. Private helper/context/run
ledger/raw diagnostics stay ignored under `tests/interop/.local`; no values or
private account/workspace/run IDs are exported.
