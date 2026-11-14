# Horizon three: reviewer handoff

## Upstream reconciliation: 2026-11-10

All five H3 branches now incorporate upstream main `a1876a9` through their
phase-one/phase-two prerequisites. Merged PRs #137–#140 and #113/#117/#123 are
included with their final maintainer corrections. The remaining prerequisite
PRs are #141/#142 and #149; campaign #157 also carries provider #155.
Generated model files are identical to main.

On the reconciled profile branch all four workspace Rust checks passed:
220 tests passed, none failed, 6 explicitly ignored. Current native diagnostic
regressions passed (three tests); the native offline multipart profile passed
10 checks with one unsupported item. Provider classification tests also passed.
Scenario API sources and SDK scenario tests match reconciled #153, whose actual
SDK/RestWesProvider comparison passed. The provider branch repeated 15 contracts
and the real CPU continuation/race demonstration; reuse repeated eight contracts
and one real A container with zero B submissions. Campaign contracts passed all
seven tests, including the installed Temporal adapter. Its Rust sources match
the tested provider branch, so those unchanged checks are reused.

The historical pinned reports below remain historical evidence. No new cloud
POST was sent. Accepted Seqera execution and bounded Nextflow container cleanup
remain unverified/failed as recorded; operator integration gates remain open.


Six issue scopes are implemented in five stacked PRs ready for review. The local prototype
and published scenario-service deliverables are validated. Native Seqera accepted
execution and the final Nextflow physical-container cleanup check remain gaps;
this document does not claim every provider acceptance check passed.

| Scope | Implementation | Observed result |
| --- | --- | --- |
| H3-01 adaptive campaigns | [PR #157](https://github.com/ga4gh/ga4gh-sdk/pull/157) | Actual Temporal restart/acceptance-loss demonstration:6 accepted jobs,5 unique scored results,1 retained unknown, peak2 reservations;3 histories replay |
| H3-02 portable checkpoints | [PR #155](https://github.com/ga4gh/ga4gh-sdk/pull/155) | Restore 20/100 steps, target executes 80;380-byte checkpoint; matching outputs and competing-writer 200/409 |
| H3-03 bound admission | [PR #155](https://github.com/ga4gh/ga4gh-sdk/pull/155) | Real managed copies, forbidden staging/submission rejected, plan expiry and intent binding regressions;15 behavior tests |
| H3-04 exact deterministic reuse | [PR #156](https://github.com/ga4gh/ga4gh-sdk/pull/156) | One real A container, zero B submissions, 114 bytes revalidated/transferred;8 focused contracts and all explicit rejection cases |
| H3-05 shared scenario API | [PR #153](https://github.com/ga4gh/ga4gh-sdk/pull/153) | Public OCI+pack;9 HTTP tests; SDK and maintained RestWesProvider share 7 WES scenarios, including an accepted lost response; source, actual imported image and LAN URL exercised |
| H3-06 named provider profiles | [PR #158](https://github.com/ga4gh/ga4gh-sdk/pull/158) | Real direct Funnel, upstream proTES and Nextflow/nf-ga4gh profiles; offline native multipart; live Seqera HTTP 400 without an acknowledged run; final NF container cleanup observation fails |

[Machine-readable selected measurements](horizon3-evidence.json) preserve scope,
commands, source/checksum identities and limits.
[Profile JSON/JUnit and detailed observations](horizon3-interop/README.md) preserve
both positive and negative results. [Coordination issue #148](https://github.com/ga4gh/ga4gh-sdk/issues/148)
links all six original issue bodies. Existing upstream scopes remain open.

## Validation and review

The final composed code at `a76fac1d20f14be97eef137ad12e8e0ac91d7c16` passed
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets`,
`cargo build --workspace` and `cargo test --workspace`: 222 passed, 0 failed,
7 ignored default tests. Separate opt-in demonstrations are identified above;
ignored suites are not counted as default passes. Final evidence/documentation
commits do not change those Rust inputs. Existing Clippy warnings remain.

The final four checks used a dedicated artifact directory. An earlier attempt
using the same target directory as other worktrees failed to resolve the reuse
module despite its export being present; the isolated run passed. Use separate
`CARGO_TARGET_DIR` values for concurrent worktrees.

Independent reviewers exercised the material race, canonical-identity and fixture
findings; regression fixes were rechecked. A final reviewer independently tested
native one-POST/unknown-ledger preservation, disabled redirects and separation of
queued acceptance from real execution. Review is not a conformance certification.
Upstream PR CI has not supplied passing runs; these are local validation results.

All PRs target upstream `main` and inherit first-phase SDK changes in #137–#142
plus workflow contracts in #149. Their descriptions identify the focused H3 files
and commits. Generated-model differences in the full stacked diffs belong to
those earlier dependencies; H3 code does not hand-edit generated models. The dependency
ordering remains explicit for maintainers; code and evidence are ready for review.

## Reproduce

Use [the integration suite catalog](../../tests/README.md) and each experiment's
instructions. Optional runtimes do not become required SDK dependencies.

[Download the scenario API OCI archive and pack](https://github.com/superbsky/ga4gh-sdk/releases/tag/horizon3-scenario-api-v0.1.0),
verify `SHA256SUMS`, import and run with `--pull=never`. The linux/arm64 manifest
is `sha256:8ca416a8a1ed149b0312db5ce742bb7b804558cc6eddb3a949332747359c5663`.
All three published payload hashes were verified after downloading the release.
The service only accepts synthetic fixtures and never executes submitted code.
Hosted Reference Cloud operation is the explicitly separate next stage in H3-05.

## Remaining observed limits

The final Nextflow run passed seven protocol/storage checks, including actual
nested directory/glob bytes and API cancellation of an acknowledged running task.
One bound task container remained running after 15.149 seconds. A later operator
probe saw zero, but that does not make bounded cleanup pass. Direct Funnel had
separate evidence of eventual stop after 1.044 seconds. Neither proves termination
of every worker process. Nonshared/object storage, authoritative last multi-attempt
log selection and TES IDs exposed as WES TaskLog remain unsupported/not run.

Native Seqera authentication succeeded. The sole
explicitly approved native SDK multipart launch returned HTTP 400, without a run
ID. No second POST was sent. Its private intent ledger retains uncertainty;
live accepted-run status/log/cancellation were not demonstrated. The response
body/request ID were not retained, so the cause cannot be attributed to the
SDK, requested descriptor, account context or provider. The example now retains bounded private diagnostics (4,096 body bytes and up to
256 request-ID bytes from the body) in its existing 0600 intent ledger, with
status-only public output. Three actual-SDK loopback regressions passed an
independent review. A further cloud attempt is prepared and awaits separate
permission; no additional POST has been made.

The CPU providers cooperate on one host and share a durable coordinator. Their
fencing covers authorized result publication, with physical work overlap visible.
Synthetic placement rates are advisory; there is no production billing cap,
geographic-residency proof, arbitrary-application network isolation or cloud
operator agreement. Reuse requires explicit determinism, independent trust and
fresh current access; it is not scientific metric comparison or inferred caching.
