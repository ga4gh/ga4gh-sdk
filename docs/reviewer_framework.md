# Pull request review framework

This is a working guide for reviewing changes to the GA4GH SDK. It synthesizes
feedback posted by @uniqueg on PRs #106–#112 and adds general review practices.
Those comments explicitly distinguish static inspection from executed checks;
some suggestions were nonblocking. [`AGENTS.md`](../AGENTS.md) requires the
pre-PR review below. The remaining review questions are risk-based guidance,
not an automated GitHub rule or a substitute for human approval.

## How the guide reaches an agent

- Codex reads the repository's root `AGENTS.md` as project guidance. That file
  contains the short pre-PR rule and points here for the detailed procedure;
  this guide does not need to be loaded for unrelated edits. See the
  [OpenAI Docs guidance on repository instructions](https://developers.openai.com/cookbook/examples/codex/iterating-development-workflows-with-codex).
- This repository's `CLAUDE.md` imports `AGENTS.md` with `@AGENTS.md`, so Claude
  Code sees the same rule. [Claude Code's documentation](https://code.claude.com/docs/en/memory)
  describes that import and notes that instruction files are context, not
  enforced configuration.
- The guide and `AGENTS.md` must be present in the working copy. Branches and
  clones receive this workflow only after they include the commit that adds it.

## What the earlier reviews taught us

| Review | Recurring lesson |
| --- | --- |
| [#106](https://github.com/ga4gh/ga4gh-sdk/pull/106#issuecomment-6746379367) | Verify that documented commands work, formatting claims match generator behavior, and the review states what was not run. |
| [#107](https://github.com/ga4gh/ga4gh-sdk/pull/107#issuecomment-6746219194) | Check downstream paths after renames, CI coverage of each crate, and installation requirements for documented tools. |
| [#108](https://github.com/ga4gh/ga4gh-sdk/pull/108#issuecomment-6747086769) | Validate the lockfile against manifests, use a locked CI check, and separate current dependency risk from unrelated upgrade work. |
| [#109](https://github.com/ga4gh/ga4gh-sdk/pull/109#issuecomment-6751078622) | Review public URL handling as a security boundary; check encoding, API-root containment, migration effects, and readable errors. |
| [#110](https://github.com/ga4gh/ga4gh-sdk/pull/110#issuecomment-6750831221) | Make CI coverage and its final gate trustworthy; pin actions, bound job duration, and use accurate issue-closing language. |
| [#111](https://github.com/ga4gh/ga4gh-sdk/pull/111#issuecomment-6751079448) | Treat a new public CLI library as a downstream contract: output, configuration, errors, dependency versions, examples, and tests. |
| [#112](https://github.com/ga4gh/ga4gh-sdk/pull/112#issuecomment-6751080322) | Call out behavior changes and scope additions, check error consistency, and examine test isolation on shared mock servers. |

Each linked comment says it was drafted by Claude Code on behalf of @uniqueg.
The principles below are our synthesis of that feedback, not quotations or a
claim that every point was a merge blocker.

## Pre-PR publication gate

This gate applies **before creating any PR, including a draft**. It is an agent
workflow, so a green CI check after publication cannot stand in for it. Use a
shorter review for a small documentation change and a deeper one for public
API, authentication, transport, dependency, generated-code, or CI changes.

1. Finish the requested implementation and relevant local checks. Read the
   linked issue and review the complete diff against the intended base, not
   just the last commit. Use the workflow and author preflight below.
2. List concrete findings as blocking defects, required clarification or
   verification, nonblocking follow-ups, or questions. Check whether the
   changed components are actually covered by CI, but do not require CI
   results before a PR exists.
3. **Report the result in chat before opening the PR.** If required work is
   found, describe it and continue fixing what is within the task's scope.
   Rerun affected checks and repeat the review after each material change.
4. If a blocking defect, required check, or material design question remains,
   report the exact unresolved item and **do not create the PR**. This includes
   a draft PR. An existing draft should stay draft while such work remains.
5. Once the gate passes, create the PR if authorized by the task. Keep its body
   focused on behavior changes, material limitations, and issue-specific
   verification that CI does not show. Routine check results belong in the chat
   review report; do not copy a command list into the PR solely as a checklist.
   Mention an unrun opt-in suite only when it covers behavior changed by the PR
   and the omission matters to reviewers. Then inspect CI and review feedback;
   those are later gates.

When updating an existing PR, preserve accurate, issue-specific validation from
its author. Correct claims made stale by new commits or CI results, but do not
replace the author's evidence with a routine workspace-check list.

Use a concise report rather than a ceremonial checklist:

```text
Pre-PR review: PASS | BLOCKED
Base and issue: ...
Checks run: ...
Required findings: none | specific finding and next action
Nonblocking follow-ups: ...
Unverified areas: ...
```

The reviewer framework cannot guarantee that another tool or a human will
obey this gate. Repository instructions guide Codex and Claude Code; they are
not a hard block in `git` or GitHub. A mechanical hook can enforce a command or
artifact, but it cannot decide whether a design finding has been resolved.

## Review workflow

### 1. Establish the review boundary

- Read the linked issue, acceptance criteria, PR description, repository guide,
  and the diff against the PR's **actual base**. For a stacked PR, identify
  parent branches and review only the new layer; note what will need rebasing.
- Write down the contracts this change touches: SDK API, CLI behavior, Python
  API, HTTP requests, configuration, generated models, build, CI, or docs.
- Identify user-visible and wire-level behavior **before** evaluating the
  implementation. Compare old and new behavior, including failure paths.
- Scan the whole diff for accidental formatting, generated files, secrets,
  unrelated changes, and a mismatch between the PR title and the actual work.
  Focused PRs and author self-review make review easier and safer; see
  [GitHub's guidance](https://docs.github.com/en/pull-requests/concepts/helping-others-review-your-changes).
- State the evidence level: static read, local build, tests, integration run,
  or CI result. Name what was **not** checked. Green CI proves only what its
  jobs actually execute.

### 2. Trace behavior across boundaries

Follow at least one normal path and one plausible failure path from input to
result. For an HTTP command, that often means CLI argument or config → client
→ `Transport` → URL and query → response parsing → stdout, stderr, exit status,
or SDK error. Review both the modified lines and the callers that depend on
them. An input can originate in a file, environment variable, Python binding,
downstream CLI, or server response; do not assume the current CLI is the only
caller of a public function.

Check these boundaries where relevant:

- **URLs and credentials:** require relative endpoints under the configured
  API root; reject origin changes, dot segments, and ambiguous base URLs.
  Distinguish encoding of a path segment from encoding of a query value.
  Never let a future caller direct an authenticated request to another origin.
- **Inputs and errors:** reject malformed input without a panic, silent
  fallback, or misleading second error. A failed request must not look like a
  successful CLI invocation. Preserve structured status and full response data
  for callers; keep displayed messages bounded and free of secrets.
- **Serialization:** check omitted, repeated, empty, Unicode, reserved, and
  unknown values where the protocol permits them. Verify the request actually
  sent, not only the object created before serialization.
- **State and side effects:** ask what happens if an operation stops halfway
  through, is retried, or receives a partial response. This matters especially
  for task creation, cancellation, model regeneration, and file replacement.

This source-to-sink approach extends the URL and error findings in
[#109](https://github.com/ga4gh/ga4gh-sdk/pull/109#issuecomment-6751078622).
It also follows [OWASP's secure code review guidance](https://cheatsheetseries.owasp.org/cheatsheets/Secure_Code_Review_Cheat_Sheet.html)
on examining data flow and trust boundaries.

### 3. Review the public contract

- Can a downstream Rust CLI capture or reformat output and inject its own
  configuration? Keep process I/O at the binary boundary when callers need
  control over it.
- Are error kinds actionable and consistent across Rust, CLI, and Python?
  Check `Send`/`Sync` where clients and errors are meant to move across tasks.
- Does a public signature expose a dependency major version or implementation
  detail that will be costly to change? Decide this before downstream users
  adopt it. Use the workspace package version rather than a copied literal.
- Are public functions documented with inputs, errors, and a small downstream
  example? Compile or run at least one representative caller when practical.

These questions come from [#111](https://github.com/ga4gh/ga4gh-sdk/pull/111#issuecomment-6751079448)
and align with the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
on meaningful errors, caller control, `Send`/`Sync`, and examples. Apply them
to the **actual intended API**; do not require broad abstractions for private
implementation details.

### 4. Match checks to risk

| Change | Evidence to look for |
| --- | --- |
| CLI input/output | Subprocess test of stdout, stderr, and exit status; valid input, invalid input, and service failure. |
| HTTP path or query | Mock request matching the actual path/query, including encoded and adversarial values; verify no request escapes the API root. |
| Public Rust/Python API | A downstream call or compile check, error-kind assertions, and a documented usage example. |
| Configuration | Missing, malformed, and optional fields; migrations from previously accepted configuration. |
| Generated models | Pinned source/spec, reproducible generation, preservation of hand-maintained adaptations, and a clean formatting check. |
| Dependency or build change | Workspace and standalone builds, lockfile consistency, supported toolchain, and relevant advisory review. |
| CI change | A real run of every affected job, a final gate that reflects failures, path-filter behavior, timeouts, and safe permissions. |
| Documentation | Run or inspect the exact command from its stated working directory; verify filenames, prerequisites, and version claims. |

Run the standard Rust checks in [`AGENTS.md`](../AGENTS.md). Use `--locked` when
checking the committed dependency graph; [Cargo documents](https://doc.rust-lang.org/cargo/commands/cargo-help.html)
that it fails when the lockfile would need changing. Select opt-in integration
suites according to the affected behavior and record relevant omissions in the
chat review report. Do not present unrelated suites as PR limitations.
Prefer focused tests of observable behavior over tests that merely repeat the
implementation. Inspect shared mocks, files, ports, and environment variables
for parallel-test interference.

For CI, verify coverage rather than relying on the presence of a green badge.
Review path filters for blind spots, ensure the final gate includes all required
jobs, and give jobs reasonable timeouts. Pin third-party actions to full commit
SHAs when changing workflows; [GitHub's security guidance](https://docs.github.com/en/actions/reference/security/secure-use)
explains why a SHA is the immutable choice. Keep known warning debt and
dependency advisories visible, but do not expand an unrelated PR solely to
resolve them.

### 5. Review the change record

- Document changes to config defaults, CLI exit codes, HTTP paths, query
  behavior, error types, and Python exceptions. Give users a migration step
  when an old setup stops working, even for a pre-1.0 project.
- Keep unrelated behavior changes in a separate commit or PR when practical.
  If they remain in the PR, describe and test them explicitly.
- Check that docs, scripts, generated output, and CI tell the same story.
  Delete stale commands instead of copying them into new guidance.
- Use `Closes #N` only when merging the PR will fully satisfy that issue.
  Otherwise use `Refs #N`, track the follow-up, and leave incomplete issue
  checklist items unchecked until their outcome lands.
- After fixes, reread every review thread against the new head. Reply with the
  evidence or commit that addresses it; distinguish an answered question from
  a code change. Do not treat a green CI run as automatic approval.

## How to classify findings

Severity follows **impact and evidence**, not whether a comment sounds urgent.
Alex sometimes identified useful follow-ups while explicitly calling them
nonblocking. Use these categories as a decision aid:

| Category | Typical condition | Reviewer action |
| --- | --- | --- |
| Blocking defect | Demonstrated security-boundary escape, wrong success/failure result, data loss, broken public contract, or a required path that cannot work. | Show the failing path and request a fix plus a regression check before merge. |
| Required clarification or verification | The PR changes user-visible behavior without migration notes, claims a check it did not run, or lacks credible evidence for a material risk. | Ask for the missing explanation or targeted check; reassess after it arrives. |
| Nonblocking follow-up | Existing warning, adjacent dependency upgrade, cosmetic cleanup, or a useful improvement outside this issue. | Suggest a focused issue or later PR; do not hold the present change without a concrete dependency. |
| Question | A design choice may be intentional and the evidence does not establish a defect. | Ask the smallest question that resolves the uncertainty; avoid presenting speculation as a finding. |

When a change raises several concerns, report the highest-impact findings first.
Deduplicate comments that share one root cause. Give each finding a precise
location and a reproducible observation when possible. [GitHub's review guide](https://docs.github.com/en/pull-requests/how-tos/review-pull-requests/reviewing-proposed-changes-in-a-pull-request)
supports line-specific comments and a final review summary.

## A useful review comment

> **[Category] Location:** What happens with a concrete input or scenario.
> **Impact:** Who sees the failure, or what contract or boundary it breaks.
> **Suggested change:** The smallest correction, or two reasonable options if
> the design is undecided.
> **Verification:** A test or observable result that would close the finding.

For example, the [#109 URL review](https://github.com/ga4gh/ga4gh-sdk/pull/109#issuecomment-6751078622)
identified an absolute endpoint that could leave the API root, explained the
future credential risk, and proposed a same-origin/root check and tests. It
was actionable because it supplied a specific input and consequence.

## Author preflight

- [ ] The PR base and issue are correct; stacked dependencies and merge order
  are stated.
- [ ] The diff has one coherent purpose. I reviewed it myself for accidental
  changes, generated files, credentials, and stale documentation.
- [ ] The PR describes behavior changes, migration steps, and any unrelated
  change that remains in the diff.
- [ ] Relevant happy, failure, and boundary cases are covered by behavior tests;
  I can name what was not tested.
- [ ] The standard workspace checks and relevant opt-in suites have results;
  CI covers the affected crates and the locked graph.
- [ ] Public API changes have a downstream example or test; docs commands match
  actual paths and prerequisites.
- [ ] Review threads, issue checkboxes, closing keywords, and follow-up links
  accurately reflect the state **after** this PR merges.

## Read-only agent reviewer prompt

Use this when asking an agent for an independent first pass. An agent's report
is evidence for a human reviewer, not a substitute for approval.

```text
Review this PR against its linked issue, actual base branch, AGENTS.md, and
affected callers. Do not edit files, post comments, approve, or merge.

First state what you inspected and which checks you actually ran. Trace the
main success and failure paths, then inspect public API, trust boundaries,
serialization, tests, CI coverage, and documentation as relevant.

Report only actionable findings, ordered by impact. For each, include a file
and line, a concrete failing scenario, user or downstream impact, a minimal
fix or focused question, and a way to verify it. Label blockers, clarifications,
follow-ups, and questions separately. Do not turn an unverified possibility
into a defect. If no findings remain, say so and name any untested areas.
```
