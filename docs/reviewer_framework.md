# Pull request review framework

This guide covers review, PR publication, and the change record for the GA4GH
SDK. The [engineering principles](engineering_principles.md) define properties
to preserve in implementation. The [technical writing guide](technical_writing.md)
covers documentation and instructions. Apply each guide in proportion to the
change; repository guidance does not certify the existing implementation.

## How repository agents use this guide

Coding agents (e.g., Claude, Codex) use `AGENTS.md` as the shared entry point.
It links to the applicable guides; `CLAUDE.md` imports the same root instructions.
Load this framework for review or PR preparation and the relevant principles for
the affected contracts. Use `python/AGENTS.md` for binding-specific work. Keep
shared rules in one place rather than creating per-tool copies.

Use the instructions from the working copy being reviewed. A branch only has
these rules after it includes them. Instructions guide agents that load them;
they are not enforced configuration or an automatic GitHub approval. The
requested task determines authorization to edit, publish, or merge.

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

The reviewer framework cannot guarantee that every agent or human will obey
this gate. Repository instructions guide agents that load them; they are not a
hard block in `git` or GitHub. A mechanical hook can enforce a command or
artifact, but it cannot decide whether a design finding has been resolved.

## Review workflow

### 1. Establish the review boundary

- Read the linked issue, acceptance criteria, PR description, repository guide,
  and the diff against the PR's **actual base**. For a stacked PR, identify
  parent branches and review the new layer while accounting for inherited
  behavior. Check current `main` and identify already merged or superseded work;
  an old branch diff can include changes that are no longer part of the task.
- Write down the contracts this change touches: SDK API, CLI behavior, Python
  API, HTTP requests, configuration, generated models, build, CI, or docs.
- Select the relevant [engineering principles](engineering_principles.md#apply-the-principles-to-a-concrete-change).
  Connect each finding to a trigger, violated property, enforcement point, and
  evidence. Distinguish demonstrated defects from design questions and optional
  improvements; a code smell alone is not a blocking finding.
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

- **URLs and credentials:** keep configured service credentials within their
  API-root boundary. Check endpoint encoding and containment, redirects, and
  ownership of injected-client policy. Treat artifact credentials separately.
  Distinguish encoding of a path segment from encoding of a query value.
- **Inputs and errors:** apply the documented operation requirements and
  interoperability exceptions without a panic, silent fallback, or misleading
  second error. A failed request must not look like a
  successful CLI invocation. Preserve structured status and the response data
  promised by the API, subject to documented body limits. Keep displayed messages
  bounded and free of secrets. Exercise retained raw bodies and error causes
  separately from default diagnostics; an echoing server is a useful test.
- **Serialization:** check omitted, repeated, empty, Unicode, reserved, and
  unknown values where the protocol permits them. Verify the request actually
  sent, not only the object created before serialization.
- **State and side effects:** ask what happens if an operation stops halfway
  through, is retried, or receives a partial response. This matters especially
  for task creation, cancellation, model regeneration, and file replacement.

This source-to-sink approach follows [OWASP's secure code review guidance](https://cheatsheetseries.owasp.org/cheatsheets/Secure_Code_Review_Cheat_Sheet.html)
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

These questions align with the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html)
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
| Generated models | Pinned source/spec, reproducible generation, preservation of hand-maintained adaptations, and behavioral checks of affected constructors, response views, and request serialization. |
| Dependency or build change | Workspace and standalone builds, lockfile consistency, supported toolchain, and relevant advisory review. |
| CI change | Runs of affected execution paths, allowed skips and rejected failures at the final gate, event/path routing, timeouts, and safe permissions. |
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
- For documentation and instruction changes, apply the
  [technical writing guide](technical_writing.md#review-and-verification).
  Preserve conditions, exceptions, and requirement strength. Verify that a
  procedure identifies its prerequisites and expected result. Treat language
  preferences according to their impact, rather than as automatic blockers.
- Use `Closes #N` only when merging the PR will fully satisfy that issue.
  Otherwise use `Refs #N`, track the follow-up, and leave incomplete issue
  checklist items unchecked until their outcome lands.
- After fixes, reread every review thread against the new head. Reply with the
  evidence or commit that addresses it; distinguish an answered question from
  a code change. Do not treat a green CI run as automatic approval.
- For an existing PR, follow the [maintenance procedure](#maintaining-an-existing-pr).
  Explain material fixes before finalizing the review.

## Maintaining an existing PR

Before integration, compare the original proposal with current `main`, linked
issues, and later decisions. Preserve the intended capability while determining
what remains to implement. For each material scope change, identify work already
merged, retained here, superseded, or deferred with a linked owner. Do not revive
an obsolete behavior merely to preserve old commits.

Refresh the remote head before editing and again before pushing. If another
contributor advanced it, inspect and integrate their changes. Preserve their work
and avoid overwriting a shared branch. Resolve conflicts using the current
contracts, then review the combined tree; a clean merge alone is not evidence
that the integration is correct.

Preserve accurate rationale and issue-specific evidence in the author's PR
text. Correct stale claims and clearly distinguish historical checks from
verification of the current implementation. A short scope table can preserve
continuity after a substantial integration. The title and leading summary must
still describe the final change. Explain non-obvious decisions in commit bodies
so the reason remains available when reading history.

Explain material maintainer fixes in the PR record: the problem, resulting
behavior, affected commits or areas, and the evidence that checks the result.
For several changes, use a separate comment and link it from a concise final
review. A small edit can be explained in the review or description itself.
Keep the final review focused on findings, evidence limits, and the decision.
This adapts [Google's change-description guidance](https://google.github.io/eng-practices/review/developer/cl-descriptions.html).

Identify the reviewed head and base. If either changes, inspect the delta and
its interaction with the reviewed contracts. Rerun affected checks and update
the conclusion. An unrelated base change need not trigger every expensive test
again; a conflict resolution or changed dependency can invalidate earlier
results. Verify the head again before final approval. Existing CI, a historical
approval, or a checkbox does not validate an unreviewed integration.

For issue completion, map each acceptance criterion to merged behavior and its
evidence. Tick only delivered criteria. Use a focused child issue when an umbrella
contains independently deliverable work; close the umbrella only when its own
completion conditions are met. A clarification response alone does not complete
an implementation requirement.

## How to classify findings

Severity follows **impact and evidence**, not whether a comment sounds urgent.
Use these categories as a decision aid:

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

For example, a URL review can identify an absolute endpoint that leaves the
API root, explain which credential would reach another origin, and propose a
containment check with a zero-request regression. The concrete path and
consequence make the finding actionable.

## Author preflight

- [ ] The PR base and issue are correct; stacked dependencies and merge order
  are stated.
- [ ] The PR title names the change and its scope. Prefer `type(scope): summary`
  for code changes; put issue references and closing keywords in the body.
  Recheck the title when review changes the scope.
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
- [ ] Review threads and issue checkboxes reflect delivered work. Closing
  keywords describe what this merge will complete; follow-up links identify
  work that remains.

## Squash merge metadata

Before confirming a squash merge, inspect GitHub's generated commit message.
Remove unresolved placeholders and duplicate `Co-authored-by` trailers: the
commit author does not also need a co-author trailer. Use an email associated
with each other co-author's GitHub account if attribution is intended. This
check belongs at merge time because GitHub may populate the message from the
PR or its commits.

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
