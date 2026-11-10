# Phase two: reviewer handoff

## Upstream reconciliation: 2026-11-10

The workflow branch now includes upstream main `a1876a9`, including merged
#113/#117/#123/#137–#140. The current DRS CLI/raw-response and resume-root fixes
are retained alongside workflow commands; Python string inputs retain Rust
ownership before releasing the GIL. The missing #138 prerequisite described in
the historical handoff below is now included.

All four workspace Rust checks passed on the reconciled source: 220 tests
passed, none failed, four opt-in tests ignored. Workflow-contract feature tests
also passed. Two real Toil WES 9.3.0 container executions were repeated on this
source: both reached COMPLETE, golden report bytes and metrics were verified,
comparison passed, and IPython consumed both HTML reports. The harness permits
up to three read-only status attempts within its existing deadline; it never
replays a submission. This completes the local integrated QC check requested
below. It does not establish external-node or production operator acceptance.

The remaining sections preserve the original source/evidence identities and
historical execution results. Their old pre-merge synchronization instructions
are superseded by this reconciliation note; operator gates remain open.


Review entry point for the six phase-two issues. This packet contains the review
route, acceptance boundaries, reproducible commands and preserved local results.
It does not record an independent review or declare deployment acceptance.

## Scope and fixed code snapshots

| Work item | Delivery | Code snapshot | Remaining acceptance |
| --- | --- | --- | --- |
| S2-01, [SDK #143](https://github.com/ga4gh/ga4gh-sdk/issues/143) | [PR #149](https://github.com/ga4gh/ga4gh-sdk/pull/149): pinned invocation preparation | `00ce96e869940ffdfa5f07a66052d69a0af9670e` | API review, prerequisite integration, external-node acceptance |
| S2-02, [SDK #144](https://github.com/ga4gh/ga4gh-sdk/issues/144) | PR #149: typed results and selected artifact evidence | same SDK snapshot | Output/viewer review and actual deployment evidence |
| S2-03, [ecosystem #98](https://github.com/elixir-cloud-aai/generic-issues/issues/98) | [PR #150](https://github.com/ga4gh/ga4gh-sdk/pull/150): standalone proTES consumer | `2eeaefd938e04c763623bdd9ab7592b9cdd3b7f2` | Downstream/API review; controlled executor boundary remains |
| S2-04, [SDK #145](https://github.com/ga4gh/ga4gh-sdk/issues/145) | PR #149: explicit comparison policy | same SDK snapshot | Scientific policy review and external-node acceptance |
| S2-05, [ecosystem #99](https://github.com/elixir-cloud-aai/generic-issues/issues/99) | [PR #151](https://github.com/ga4gh/ga4gh-sdk/pull/151): leased-session RFC and executable reference | `7e489d59b7a8a0f8461fc7dca4dc77af3131e2c5` | Joint operator/controller/ingress contract and deployment gates |
| S2-06, [ecosystem #100](https://github.com/elixir-cloud-aai/generic-issues/issues/100) | PR #151: workload-secret RFC and real OpenBao feasibility | same RFC snapshot | Broker/runner identity, authorization and deployment gates |

The SDK adds an optional `workflow-contracts` feature; the CLI enables it.
The companion is a separate crate against main. The RFC prototypes establish
specific local properties; they add no portable session/secret API to the SDK.
All six issues remain open. No production scientific data or credentials are
needed for the supplied synthetic examples.

## Dependency and integration order

Status and dependency ancestry checked on 2026-11-10 UTC.
At handoff, upstream `main` is `c24cc01b37b1be4bc809addaa3a37a04a3f8ef0f`.
All three PRs target `main` and are **ready for review**. They reported
`MERGEABLE` at the status check; this is not a promise about a future merge.
The manifest preserves the initial draft/CI observations as historical evidence;
current review readiness is recorded on the live PRs. Outstanding integration or
deployment gates do not prevent review.

1. Review #150 and #151 independently against main. They do not depend on #149
   or on each other. Approval of an RFC records agreement with a proposal, not
   completion of its deployment issue.
2. For #149, review/integrate upstream [#113](https://github.com/ga4gh/ga4gh-sdk/pull/113)
   (TRS), [#117](https://github.com/ga4gh/ga4gh-sdk/pull/117) (WES 1.1/models) and
   [#123](https://github.com/ga4gh/ga4gh-sdk/pull/123) (CLI TES), and the phase-one
   [#137](https://github.com/ga4gh/ga4gh-sdk/pull/137)–[#142](https://github.com/ga4gh/ga4gh-sdk/pull/142)
   stack following its own dependency order. #149 contains composed prerequisite
   snapshots, so its full main-to-head diff is intentionally larger than phase two.
3. Before merging #149, synchronize reviewed prerequisite tips and main, resolve
   overlap, inspect the resulting diff, and validate the resulting tree. One known
   outstanding dependency update is #138 commit
   [`aff99993498d09fb5ec9d9e084dfa56add97d94d`](https://github.com/ga4gh/ga4gh-sdk/commit/aff99993498d09fb5ec9d9e084dfa56add97d94d)
   (`fix(python): own string inputs before releasing the GIL`), which is absent
   from the SDK snapshot. The current tips of #113/#117/#123/#137/#139–#142 are
   ancestors of that snapshot. This packet does not silently change the code
   under review to incorporate the later Python fix.
4. Merging #150 and #149 will require combining their additions to
   `tests/README.md`. Model/API evolution from #117 also warrants checking the
   companion against the integrated tree, despite its independent current build.

Generated model changes in #149 come from its prerequisites, especially #117.
The phase-two commits below do not change `lib/src/clients/*/models/`.
A full clean model-regeneration run was not recorded for phase two; do not infer
that from formatting or unit-test success. Apply the repository's wrapper/patch
rule when validating or changing the integrated generated tree.

### Read the phase-two delta without rereviewing every prerequisite

Start with these three authored changes, in order; review the merge resolutions
and final files as well. These commit diffs isolate authored phase-two work but
are not a replacement for integration review:

- [`5977d8a`](https://github.com/ga4gh/ga4gh-sdk/commit/5977d8a5dd78c1889579c49834952a46b59cd781): optional layer, CLI, fixtures, QC harness.
- [`87ea442`](https://github.com/ga4gh/ga4gh-sdk/commit/87ea44255a97d0c30b9c86d6dd506f48e968d32e): WES auth/discovery and schema/comparison fixes.
- [`00ce96e`](https://github.com/ga4gh/ga4gh-sdk/commit/00ce96e869940ffdfa5f07a66052d69a0af9670e): retain the root contract for referenced output annotations.

For #150 and #151 the complete diff against the fixed main snapshot is their
own scope. [The manifest](phase2-evidence.json) fingerprints the final authored
files in each code snapshot, including the independent companion and RFC trees.
Later changes to this review packet do not imply new implementation validation.

## Review route and questions

### SDK API and scientific semantics (#149)

Start with [the user/API guide](../workflow-contracts.md), then
[`workflow/mod.rs`](../../lib/src/workflow/mod.rs),
[`results.rs`](../../lib/src/workflow/results.rs),
[`compare.rs`](../../lib/src/workflow/compare.rs) and
[`cli/src/workflow.rs`](../../cli/src/workflow.rs).
Tests are in [`workflow_contracts.rs`](../../lib/tests/workflow_contracts.rs),
[`cli/tests/workflow.rs`](../../cli/tests/workflow.rs) and
[`wes_native_discovery.rs`](../../lib/tests/wes_native_discovery.rs).

- [ ] The public extension shape fits downstream [#78](https://github.com/ga4gh/ga4gh-sdk/issues/78): optional feature, caller-owned transport, serializable evidence, no UI or proprietary provider dependency in standard clients.
- [ ] Pins and exact schema-byte digest are explicit. Draft 2020-12 validation retains root `$ref` scope, rejects unresolved resources/required unknown vocabularies, performs no URI fetch or default insertion, and distinguishes schema-valued keywords from instance data.
- [ ] `PreparedInvocation` cannot be deserialized/edited into a validated submission object. Parameters remain exact; primary uploaded bytes can be digest-bound. Caller-supplied workflow identity is not attestation of bytes later fetched from a mutable remote URL.
- [ ] Submission is explicit and single-attempt. Authenticated discovery and upload share caller-owned transport. Native WES 1.0 compatibility is outside generated models. OAuth token injection does not claim login/refresh support.
- [ ] Execution COMPLETE, contract conformance, artifact access and integrity remain separate. Missing/inaccessible/mismatched artifacts preserve sibling results and execution state. Expected SHA-256 plus source differs from a freshly computed digest alone.
- [ ] Object-store access uses selected ephemeral DRS plans, isolated credentials, no redirects, a byte bound and operation budget. Durable URL redaction is limited; arbitrary caller context/schema/scalar content is not a general secret-redaction facility.
- [ ] `contract_document` preserves root annotation scope. The small viewer helper handles inline hints; richer viewers must resolve referenced/conditional hints. The IPython adapter is for the known public demo report.
- [ ] Numeric comparison is exact rational arithmetic with the documented inclusive absolute/relative rule. Review >2^53 integers, zero, null/missing values, policy bounds, ignore rules and unordered zero-tolerance arrays. Required identity drift/missing evidence is inconclusive independently of metric agreement.

### Downstream/gateway boundary (#150)

Read the [companion README](https://github.com/ga4gh/ga4gh-sdk/blob/2eeaefd938e04c763623bdd9ab7592b9cdd3b7f2/examples/protes-companion/README.md),
then its `src/lib.rs`, `src/main.rs` and `tests/consumer.rs` at the same snapshot.
Pinned proTES API revision: `55c4d1591cd3be883ad0b4fd2da01d49976414cb`.

- [ ] Typed and raw provenance preserve unknown fields, ordered forwarding/hooks, build identities and actual `changes.paths`. HMAC before/after records are not mutation values or attestations.
- [ ] Explanation never submits. Rerun preparation requires supplied original client intent; transformed FULL responses cannot establish it. The caller's original-input assertion has explicit limits.
- [ ] Server-owned fields and recorded `gateway.*` state are stripped; only explicit supported choices are rebuilt. Backend choices match the pinned JSON-encoded object format.
- [ ] Exact prepared JSON preserves nested TES 1.1 fields such as `ignore_error`. A missing/malformed acknowledgement or ambiguous outcome requires reconciliation with no replay/failover. 403/404 retain their ambiguity rather than inventing causes.

### Provider/operations contracts (#151)

Read [leased sessions](https://github.com/ga4gh/ga4gh-sdk/blob/7e489d59b7a8a0f8461fc7dca4dc77af3131e2c5/docs/rfcs/leased-sessions.md)
and [workload secrets](https://github.com/ga4gh/ga4gh-sdk/blob/7e489d59b7a8a0f8461fc7dca4dc77af3131e2c5/docs/rfcs/workload-secrets.md),
then the matching executable prototypes. Owners below are required roles;
no named individual or deployment owner has accepted them yet.

| Decision/gate | Roles needed | Evidence required before issue closure |
| --- | --- | --- |
| Actual persistent scientific service and access policy (#99) | Scientific consumer + operator | Confirm why completed HTML output is insufficient; agree lifetime and access policy |
| Durable session lifecycle (#99) | TESK/controller + ingress operator | Actual task/Pod binding; restart reconciliation; lease races; active WebSocket closure; partial cleanup; expiry after client exit |
| Authorization and workload identity (#100) | Broker + runner + OpenBao operator | Authenticated requester; intent registry; verified task → Pod UID → subject; unsupported-runner rejection before launch |
| Secret handoff and lifetime (#100) | Broker/runner + protected-service owner | Protected channel; issuer/key and replay controls; deployed FULL task/provenance/log inspection; engine-appropriate expiry/renewal/revocation |

The session prototype uses local subprocesses and test bearer authentication.
It does not implement TESK, durable recovery or real ingress/WebSocket closure.
The OpenBao experiment uses a temporary RSA test issuer and real JWT/KV-v2;
production runner attestation and persisted TES inspection remain unproved.
Revoking KV read access cannot revoke a value already disclosed. Dynamic-service
credential revocation needs its own engine and acceptance evidence.

## Preserved validation and its limits

[Machine-readable evidence](phase2-evidence.json) includes source-file hashes,
hashes of original local evidence, per-suite counts and selected live observations.
[Validation excerpts](phase2-test-results.txt) retain test names/results and build
completion lines while excluding private paths, transient IDs and raw records.
They are author-recorded historical evidence, not CI artifacts or signed attestations.
Original logs do not embed a git SHA. Coverage below follows the implementation
record; current committed-file hashes do not prove which bytes a historical run used.

| Check | Recorded result | Coverage boundary |
| --- | --- | --- |
| SDK root fmt, workspace Clippy/build/test | 213 passed, 0 failed, 4 ignored; existing warnings | Working tree leading to `87ea442`, before final root-annotation addition |
| Final root-annotation change | 18 contract tests, 3 CLI tests; library Clippy passed | Latest SDK code at `00ce96e`; preceding unchanged checks reused |
| Optional feature disabled / package resources | Check passed; vendored schema files listed | During SDK implementation; no separate final-head package release claim |
| Companion root workspace | 76 passed, 0 failed; fmt/Clippy/build passed | Unchanged main Rust sources, also applicable to the RFC branch |
| Standalone companion | 5 passed; fmt/Clippy/build passed | Companion implementation preceding `2eeaefd`; not covered by root `cargo test` |
| Container QC journey | 1 opt-in test passed; both IPython reports consumed | Historical SDK run before final auth/comparison/annotation fixes; two local WES processes |
| proTES consumer journey | Real gateway/plugin/persistence; known rejection then acceptance; separate unknown outcome with one attempt/no replay; nested-field assertion | Controlled upstream TES fixtures; no scientific executor or external federation claim |
| Session / OpenBao feasibility | Both branch prototypes passed; observations preserved | Local provider; real OpenBao primitives with test issuer; external gates remain |

The QC run produced two COMPLETE executions of synthetic inputs `ACGT`, `AC`,
`ACGTAC`: sequence count 3, total length 12, mean length 4.0. Golden HTML bytes
were verified. Policy result was `equivalent_under_policy`; changed input identity
was inconclusive. These observations do not establish an independent ELIXIR
node, LS Login, Conquest, external federation, or a final-head container rerun.

Upstream Actions at the handoff observation report `action_required` for all three
code snapshots: [#149 run](https://github.com/ga4gh/ga4gh-sdk/actions/runs/42746780960),
[#150 run](https://github.com/ga4gh/ga4gh-sdk/actions/runs/42746382971),
[#151 run](https://github.com/ga4gh/ga4gh-sdk/actions/runs/42746391941).
A maintainer needs to inspect/allow the gated workflow and evaluate the resulting
checks. Empty status rollups are not a passing CI result. Subsequent documentation
pushes may create newer gated runs; inspect the current PR before approval.

## Reproduce without changing an existing checkout

Use separate checkouts/worktrees for these mutually independent snapshots. Fetch
the fork branches, then check out the fixed SHA for the PR being reviewed:

```sh
git fetch https://github.com/superbsky/ga4gh-sdk.git \
  feat/phase2-workflow-contracts feat/protes-downstream-companion \
  docs/phase2-provider-contracts
# In a separate review checkout:
git switch --detach 00ce96e869940ffdfa5f07a66052d69a0af9670e
```

Stable Rust with rustfmt/Clippy, dependency-download access and local mock sockets
are required. Run the four repository checks from each relevant root:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets
cargo build --workspace
cargo test --workspace
```

For #149, focused behavior, disabled-feature and packaging checks:

```sh
cargo test -p ga4gh-lib --features workflow-contracts --test workflow_contracts
cargo test -p ga4gh-cli --test workflow
cargo test -p ga4gh-lib --test wes_native_discovery
cargo check -p ga4gh-lib --no-default-features
cargo package -p ga4gh-lib --list --allow-dirty
```

Follow [the prepare/submit guide](../workflow-contracts.md) for valid and invalid
CLI inputs. Preparation needs no config/network; explicit submit can launch work.
The optional real QC journey needs Python 3.12+, Docker, image/download access
and a checkout shared into the Docker VM. It installs pinned Toil/IPython, starts
its own loopback WES processes, writes `target/phase2-qc/evidence.json` and stops
the servers. Select a working Docker context before running:

```sh
python3 tests/run-phase2-qc.py
```

Use the normal container mode for the recorded scenario. The documented
`GA4GH_QC_NO_CONTAINER=1` alternative is host execution and must be reported
separately. Other ignored integration suites have separate prerequisites in
[the test guide](../../tests/README.md).

For #150, check out its fixed SHA and run the separate crate explicitly:

```sh
cargo fmt --manifest-path examples/protes-companion/Cargo.toml --check
cargo clippy --manifest-path examples/protes-companion/Cargo.toml --all-targets
cargo build --manifest-path examples/protes-companion/Cargo.toml
cargo test --manifest-path examples/protes-companion/Cargo.toml
cargo run --manifest-path examples/protes-companion/Cargo.toml -- \
  explain examples/protes-companion/fixtures/provenance.json
cargo run --manifest-path examples/protes-companion/Cargo.toml -- \
  prepare examples/protes-companion/fixtures/original-task.json \
  --choices examples/protes-companion/fixtures/choices.json
```

The historical live gateway orchestration was temporary, not a committed turnkey
suite. To repeat it, use the pinned proTES revision's `tests/smoke/compose.yaml`
and smoke plugin, then supply authenticated TES configuration as documented in
the companion README. Recreate controlled fixtures for (a) known 400 rejection
followed by acceptance, (b) a separate 503 unknown outcome and backend-store
reconciliation, and (c) a new request containing `executors[0].ignore_error=true`
confirmed in FULL view. Inspect/fetch never creates work; `create` explicitly does.
Do not replay the unknown outcome to demonstrate recovery. The five committed
consumer tests provide the deterministic reviewer path without that live setup.

For #151, check out its fixed SHA. Python local sockets are needed for sessions;
OpenBao additionally needs Docker, `openssl` and image-download access. Each
prototype cleans its own processes/container/private test issuer:

```sh
python3 examples/provider-contracts/session_feasibility.py --evidence /tmp/phase2-session.json
python3 examples/provider-contracts/openbao_feasibility.py /tmp/phase2-openbao.json
```

## Exit criteria and feedback

- [ ] Review findings are attached to the relevant PR and pinned code location; identify whether they concern a prerequisite, phase-two implementation, scientific policy or provider decision.
- [ ] Maintainer evaluates gated CI on the actual reviewed head; no required check is inferred from local evidence.
- [ ] Before integration, synchronize dependencies (including #138), preserve generated-model reproducibility, reconcile overlapping docs and validate the integrated Rust/Python/companion behavior affected by those changes.
- [ ] Repeat the container QC journey on the integrated head before final execution acceptance; obtain the separately required external-node/auth/viewer evidence where those issues require it.
- [ ] Keep #99/#100 open until named operators accept ownership and their deployment gates above are demonstrated. RFC review alone cannot close them.

The packet is ready for review. Merge approval, successful upstream CI and
external acceptance remain distinct outstanding steps.
