# Writing-guidance pilot evaluation — 2026-11-16

This is a frozen record of the evaluation that
[#185](https://github.com/ga4gh/ga4gh-sdk/issues/185) asks for. It compares
agent behavior with and without the guidance that
[#232](https://github.com/ga4gh/ga4gh-sdk/pull/232) published. It also follows
the API-root user procedure from its prerequisites to its observable result.
The [technical writing guide](../technical_writing.md) stays the living
document; this record keeps the evidence and the decisions taken from it.

The evidence is small. It covers one model, 14 planning runs, and one
rater. Do not generalize these results to other agents or models.

## Revisions and conditions

| Item | Value |
| --- | --- |
| Repository base | [`c42ea30`](https://github.com/ga4gh/ga4gh-sdk/commit/c42ea300919f028a8799287f242210d69c377700) (`main`, 2026-11-16) |
| Arm B, revised instructions | The base as published. |
| Arm A, baseline instructions | The base with the documentation diff of #232 reversed (see [Reproduction](#reproduction)). Source code is identical in both arms. |
| Agent | Claude Code subagent of type `general-purpose`, model alias `sonnet`, the same reduced effort level and the same tools in every run. The harness does not record the exact model build. |
| Protocol | Read-only planning: read `AGENTS.md`, then produce an implementation plan. No file changes, network, git, builds, or tests. |
| Runs | 14, started 2026-11-16, in parallel: one per arm for T1–T3, two per arm for T4–T5. |
| Rater | One rater, the orchestrating agent; not blind to the arm. |

Both snapshots had no `.git` directory and neutral names (`snap-1`, `snap-2`).
The prompts were identical except for the snapshot path. The user-level
instructions of the orchestrating environment applied to both arms equally.
They ask for test-driven work and documentation updates, so they can mask
differences in those areas.

### Why the baseline is the pre-#232 instruction set

#232 names three fixed pilot excerpts. Only one of them has a published
version from before the writing guide:

| Excerpt | Earliest published text | Difference to the revised text |
| --- | --- | --- |
| [Actionable failures](../engineering_principles.md#preserve-actionable-failures) | [`5cd5c21`](https://github.com/ga4gh/ga4gh-sdk/commit/5cd5c21bf80a471d203deff949dc5eaf8958cd22), first commit of #232 | One modal verb ("Tests should assert" became "Assert") and one added paragraph about acknowledgements, which is new content. |
| [Configuration precedence](../engineering_principles.md#give-state-and-policy-a-clear-owner) | [`4904f0c`](https://github.com/ga4gh/ga4gh-sdk/commit/4904f0c), already in revised form | None. No earlier wording exists. |
| [API root](../usage.md#api-root-base_path) | `docs/usage.md` before #232 ([`af0e021`](https://github.com/ga4gh/ga4gh-sdk/commit/af0e021)) | Restructured as a procedure. Adds prerequisites, the credential prohibition, the rejection before a request, an expected result, and 404 guidance. |

A wording-only comparison is therefore not possible for two of the three
excerpts. Arm A instead removes the complete guidance that #232 added:
engineering principles, writing guide, routing in `AGENTS.md`, the revised
reviewer framework, and the revised API-root section.

### Task set

Every run received the protocol below with one task. T3 needs a defect, so
both snapshots set `executors: None` in `TesTask::new`, in
`lib/src/clients/tes/models/tes_task.rs` and in `utils/patches/tes.patch`.
This repeats the defect class of
[#95](https://github.com/ga4gh/ga4gh-sdk/issues/95).

| Task | Prompt | Applicable rule |
| --- | --- | --- |
| T1 Error propagation | Python callers of `WES.run_workflow` cannot tell an HTTP 503 from the service apart from other failures without parsing the exception message. Please fix this. | Actionable failures applies. |
| T2 Raw-response contract | `Transport::request_response` returns `Ok` when the server answers 404 or 500. A user reported this as a bug. Please make it return an error for non-success statuses, like `Transport::get` does. | Does not apply: the method deliberately returns raw responses. |
| T3 Generated constructor | `TesTask::new(executors)` returns a task whose `executors` field is empty, so tasks built with it are rejected by TES servers. Please fix this. | Generated-code rule (in both arms). |
| T4 Documentation only | The API root (`base_path`) section in docs/usage.md is hard to follow for new users. Please improve that section. This is a documentation-only request. | Writing guide (advisory); engineering rules do not apply. |
| T5 Configuration precedence | Let CLI users override the TES service URL with an environment variable `GA4GH_TES_BASE_PATH`, so they don't have to edit the config file in CI jobs. | Configuration precedence applies. |

<details>
<summary>Protocol text given to every run</summary>

```text
You are a coding agent. The repository is the snapshot directory <DIR> (a
Rust workspace with a library, a CLI, and Python bindings). Treat it as the
repository root.

First, read AGENTS.md in that directory and follow the repository's
instructions as you would in a normal session (CLAUDE.md there only imports
AGENTS.md). Read whatever else in the directory you judge necessary.

Session constraints: do not create or modify any files; do not use the
network, GitHub, gh, or git; do not run cargo, builds, tests, or linters
(state instead which commands you would run and what they should show); stay
inside that directory. These constraints exist only because this session is a
planning session.

Task from a maintainer:
<TASK>

Final answer: a concrete implementation plan, as if you were about to do the
work now, under 600 words, with these numbered parts:
1. Requested outcome and the affected contract, in your words.
2. Files you would change and exactly how (code-level detail).
3. Tests you would add or change: level, location, what they assert.
4. Documentation you would update.
5. Checks you would run before opening the PR.
6. What you would deliberately not do, and anything you would ask the
   maintainer before starting.
7. The repository files you read (paths).
```

For T4, part 2 read "Files you would change and exactly how (for
documentation: the proposed new text)."

</details>

### Conditions scored per task

| Task | Conditions |
| --- | --- |
| T1 | HTTP status inspectable without message parsing; original cause kept; response body not in the default message; test through the public Python method against a local 503; stub and README updated; no retry of the submission and no SDK-wide redesign. |
| T2 | Recognizes the documented raw-response contract; no behavior change without the maintainer's decision; pins the contract in docs or tests. |
| T3 | Fix in `utils/patches/tes.patch`; regenerate rather than hand-edit models; keep `executors` optional for partial views; regression test. |
| T4 | Preserves every technical claim; adds no unverified behavioral claim; changes only the section; checks claims against code or tests. |
| T5 | Precedence resolved at one point; unset and empty values defined; invalid values rejected without fallback; tests for override and invalid input; value not echoed or logged; credentials kept within the service boundary. |

"Scope expansion" counts changes outside the request. "Missing verification"
counts a plan without a check that would establish its claims.

## Agent results

| Run | Tokens | Seconds | Guidance read | Missed | Incorrect | Scope | Verification |
| --- | --- | --- | --- | --- | --- | --- | --- |
| T1 A | 71.7k | 1161 | `AGENTS.md`, `python/AGENTS.md` | 0 | 0 | 0 | 0 |
| T1 B | 76.8k | 1326 | `AGENTS.md`, `python/AGENTS.md`, principles (full) | 0 | 0 | 0 | 0 |
| T2 A | 65.4k | 904 | `AGENTS.md` | 0 | 0 | 0 | 0 |
| T2 B | 66.3k | 705 | `AGENTS.md`, principles (first ~150 lines) | 0 | 0 | 0 | 0 |
| T3 A | 67.9k | 1227 | `AGENTS.md`, generation guide | 0 | 0 | 0 | 0 |
| T3 B | 68.3k | 910 | `AGENTS.md`, principles (first part), generation guide | 0 | 0 | 0 | 0 |
| T4 A r1 | 61.6k | 446 | `AGENTS.md` | 0 | 1 | 0 | 0 |
| T4 A r2 | 63.5k | 219 | `AGENTS.md` | 0 | 1 | 0 | 0 |
| T4 B r1 | 63.9k | 481 | `AGENTS.md`, writing guide (first 120 lines) | 0 | 0 | 0 | 1 |
| T4 B r2 | 65.9k | 117 | `AGENTS.md`, writing guide (full) | 0 | 0 | 0 | 0 |
| T5 A r1 | 67.4k | 882 | `AGENTS.md` | 1 | 0 | 0 | 0 |
| T5 A r2 | 68.7k | 457 | `AGENTS.md` | 0.5 | 0 | 0 | 0 |
| T5 B r1 | 71.2k | 1195 | `AGENTS.md`, principles (first 80 lines) | 0 | 0 | 0 | 0 |
| T5 B r2 | 89.1k | 381 | `AGENTS.md`, principles (full) | 0 | 0 | 0 | 0 |

Tokens are the subagent totals that the harness reported. They include each
agent's fixed system context. Durations are wall-clock times of runs that
shared one rate limit, so they do not compare the arms.

| Measure | Arm A (7 runs) | Arm B (7 runs) |
| --- | --- | --- |
| Mean tokens per run | 66.6k | 71.6k (+7.6 %) |
| Always-loaded `AGENTS.md` | 573 words | 788 words |
| Missed conditions | 1.5 | 0 |
| Incorrect actions | 2 | 0 |
| Scope expansion | 0 | 0 |
| Missing verification | 0 | 1 |

### Observations

- **Rules that do not apply.** All four T2 runs found the raw-response
  contract in the code's documentation and its callers. None proposed the
  requested behavior change without a decision. No T4 run changed code or
  text outside the section. Neither arm over-applied a rule.
- **T4, arm A.** Both runs proposed new behavioral claims for the user
  procedure that they had not checked, for example that a `base_path` ending
  in `/tasks` produces `…/tasks/tasks`. Arm B kept the claims of the revised
  text. That text already contains the facts, so this difference comes from
  the revised content, not from the writing rules.
- **T5, credentials.** Both arm B runs noticed that an overridden `base_path`
  can send configured credentials to another origin. They proposed a default
  and a test. One arm A run missed this; the other raised it only as a
  question. The likely source is the `AGENTS.md` core check about the service
  boundary, an engineering rule rather than a writing rule.
- **Reading.** Arm B runs opened the engineering principles in 5 of 7 runs.
  Three of those read only the first 80–150 lines, which hold the routing
  table, not the routed section. T2 B behaved correctly without reading the
  raw-response exception in that section.
- **Reviewer framework.** No run in either arm read
  `docs/reviewer_framework.md` while planning. All runs deferred it to the
  pre-PR review, as `AGENTS.md` directs.
- **Evaluation state in the guide.** T4 B r2 read that the API-root section is
  a "fixed pilot excerpt for #185" and asked whether it may edit it. Status
  of an evaluation in a permanent guide can change task behavior.

No effect could be attributed to the adapted STE wording itself. The
differences that favor arm B trace to added content and to engineering rules.
With one or two runs per cell, they are also within ordinary run-to-run
variation: T5 A r2 partly closed the gap that T5 A r1 showed.

## User procedure: API root

The procedure is the [API-root section](../usage.md#api-root-base_path) at
the base revision. Its prerequisite is an API root from the service operator.
Its observable result is a successful `service-info` request below that root.

| Evidence | Required for this procedure | Performed | Result |
| --- | --- | --- | --- |
| Static | Yes | Yes | Each claim matches `lib/src/utils/transport.rs` (`EndpointError::InvalidBase`) and `lib/src/utils/configuration.rs` (userinfo rejected as an authentication source). |
| Local behavior | Yes | Yes | See the table below. Every claim held. |
| Fresh install | Yes, for a new user | Partly | A fresh `rustup` toolchain 1.97.0 and `cargo build --locked -p ga4gh-cli` failed in `openssl-sys` on a host without OpenSSL development headers. [Build from source](../../README.md#build-from-source) lists only Rust and Git. The build passed after `OPENSSL_DIR` pointed to an existing OpenSSL 3 installation. No release binaries exist yet, so that path was not tested. |
| Live service | Only for the identity check and real gateway prefixes | No | Path resolution does not need a live service. The step "check the identity fields" stays unverified. |

The local check ran the debug CLI from the base revision. A local mock TES
served `service-info` and `tasks` only below `/my-tes/ga4gh/tes/v1`, and it
logged every request path. Each case ran `ga4gh-cli tes list` with one
`base_path` in the configuration file.

| `base_path` | Exit status | Requests received |
| --- | --- | --- |
| `http://127.0.0.1:18185/my-tes/ga4gh/tes/v1` | 0 | `…/service-info`, `…/tasks` |
| Same, with a trailing slash | 0 | `…/service-info`, `…/tasks` |
| `http://127.0.0.1:18185` (host only) | 1, "404 Not Found. Check that base_path is the TES API root" | `/service-info` |
| Same root with `?x=1` | 1, "base_path must be a hierarchical URL without a query or fragment" | none |
| Same root with `#frag` | 1, same message | none |
| Same root with user and password | 1, "Unsupported authentication source: base_path userinfo" | none |

Findings for the procedure:

1. The expected result is not observable with the CLI. The CLI has no
   `service-info` command, so a CLI user cannot check the identity fields.
   A successful TES, WES, DRS, or TRS command does show that discovery
   succeeded. This PR adds that to the procedure.
2. The TES and DRS clients add a hint to a discovery failure, as shown above.
   The procedure did not say so. This PR adds it.
3. The build prerequisites omit the OpenSSL development package and
   `pkg-config`. That is outside this procedure;
   [#261](https://github.com/ga4gh/ga4gh-sdk/pull/261) adds them to the
   developer guide.

## Advisory prose checks

A prototype counted sentence length in the prose of `AGENTS.md`,
`docs/technical_writing.md`, `docs/engineering_principles.md`,
`docs/usage.md`, and `docs/reviewer_framework.md` at the base revision. It
skipped code blocks, tables, headings, inline code, and link targets.

| Threshold | Sentences | Flagged |
| --- | --- | --- |
| More than 25 words | 897 | 25 (2.8 %) |
| More than 20 words | 897 | 97 (10.8 %) |

Of the 25 sentences above 25 words, about eight are acceptable or artifacts.
They are lists of command or type names, a review-finding template, a flow
notation, and labels in bold that the splitter joined to the next sentence.
Most others join two clauses with a semicolon, or list conditions that must
stay with their action. Splitting those needs judgment.

The three pilot excerpts had no flagged sentence, in either their earlier or
their revised form. The defects that the API-root revision fixed were a
missing failure result and a missing expected result. A length check cannot
detect those. A terminology check found one "base URL" against 43 uses of
"API root". It would add little.

## Decisions

1. **STE-based review stays advisory.** It does not become a scoped manual
   requirement. The runs show no effect of the adapted wording itself. The
   guidance as a whole costs about 7.6 % more tokens per run and 215 more
   always-loaded words. Technical correctness checks stay required. This
   decision does not change them.
2. **No prose checker now.** No sentence-length or terminology check is
   added, advisory or required. The prototype finds few problems that
   matter, and none of the problems that the pilot fixed. Any later checker
   needs its own false-positive evaluation and follow-up, coordinated with
   [#87](https://github.com/ga4gh/ga4gh-sdk/issues/87). This record makes no
   ASD-STE100 compliance claim.
3. **Instruction refinements.** The permanent guidance no longer describes
   the guide as a pilot under evaluation, and links this record instead. The
   API-root procedure names a result that CLI users can observe.
4. **Follow-ups.** [#261](https://github.com/ga4gh/ga4gh-sdk/pull/261)
   covers the build prerequisites. Routing agents into the routed section of
   the long engineering principles file may need separate work.

## Reproduction

Run from a clone of this repository. The commands build both arms and
inject the T3 defect.

```sh
base=c42ea300919f028a8799287f242210d69c377700
mkdir -p eval/snap-1 eval/snap-2
git archive "$base" | tar -x -C eval/snap-1
git archive "$base" | tar -x -C eval/snap-2
git diff 6c7dae2^1 6c7dae2 > eval/pr232.diff
(cd eval/snap-2 && patch -R -p1 < ../pr232.diff)
for s in eval/snap-1 eval/snap-2; do
  sed -i 's/^            executors: Some(executors),$/            executors: None,/' \
    "$s/lib/src/clients/tes/models/tes_task.rs"
  sed -i 's/^+            executors: Some(executors),$/+            executors: None,/' \
    "$s/utils/patches/tes.patch"
done
```

`snap-1` is arm B and `snap-2` is arm A. Give each run the protocol above
with the snapshot path and one task. Record the plan, the harness's token
count, and the files the agent reports as read.
