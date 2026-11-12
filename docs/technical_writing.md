# Technical writing

Use this guide when you write or review English documentation and repository
instructions. It applies to maintainers and coding agents, including Claude
and Codex. Apply it to the text affected by the task. Keep unrelated rewrites
separate.

This guide adapts selected rules from
[ASD-STE100 Issue 9](https://www.asd-ste100.org/about_STE.html).
It does not claim full ASD-STE100 compliance. The standard includes both writing
rules and a controlled dictionary. It also permits subject-specific technical
nouns and verbs. Our technical terms and instruction structure are project
decisions.

The [engineering principles](engineering_principles.md) define properties to
preserve. This guide explains how to express those properties and user
procedures. The [reviewer framework](reviewer_framework.md) defines the review
workflow.

## Choose the audience and text type

For user documentation, use [Diátaxis](https://diataxis.fr/) to distinguish
learning, completing a task, looking up details, and understanding a design.
Diátaxis guides content organization; the adapted STE rules guide wording.
Agent and contributor instructions have their own action-and-evidence structure.

| Text | Reader needs | Structure |
| --- | --- | --- |
| Agent or contributor instruction | A clear action and a way to verify completion | Condition, action, evidence, and any exception |
| Tutorial | A first successful learning experience | A guided example, stated environment, and observable milestones |
| How-to procedure | A reproducible way to complete a known task | Prerequisites, ordered steps, expected result, and relevant failure guidance |
| API reference | Precise facts for lookup | Inputs, outputs, defaults, errors, side effects, and supported versions |
| Explanation | Reasons and relationships | Concepts, alternatives, tradeoffs, and links to procedures or reference |

Separate these purposes with sections or links where useful. A new feature does
not require four new documents. Keep the reason for a rule near the rule when
that reason helps the reader apply it; move extended rationale out of a task's
execution steps.

## Shared writing rules

The references below point to sections of the
[official standard](https://www.asd-ste100.org/assets/files/ASD-STE100_ISSUE9.pdf).
The application column defines our adaptation.

| Rule | Application in this repository | STE reference |
| --- | --- | --- |
| Use consistent terms | Use the same term for the same concept. Preserve distinctions that affect behavior. | Part 1, section 1; part 2 |
| Give one instruction per sentence | Split sequential actions into separate sentences. Keep simultaneous actions together when separation would change their meaning. | 5.2 |
| Start instructions with an action | Use an imperative verb. Name the actor in descriptions when responsibility could be ambiguous. | 5.3; section 3 |
| Put a necessary condition first | State when the instruction applies before telling the reader to act. | 5.4 |
| Keep sentences short | Aim for at most 20 words in instructions and 25 in descriptions. Review longer sentences for ambiguity. | 5.1; 6.3 |
| Keep paragraphs focused | Use one topic per paragraph. Put ordered work in numbered steps. | 6.4–6.5 |
| Keep instructions out of notes | Put required actions in the procedure. Use notes for supporting information. | 5.5 |

Sentence lengths are editorial targets in this guide. They are not automated
acceptance limits or a claim to implement STE word-count rules. Keep a necessary
condition or exception when shortening would change the meaning.

Preserve API names, commands, paths, JSON keys, code, and quoted specification
text exactly. Apply prose rules to the surrounding explanation. Identify the
source of a quotation. Change generated documentation through its maintained
source, as described in the [generation guide](autogenerate_OAI_models.md).

Do not replace distinct operations with a shared word merely to reduce
vocabulary. For example, local cancellation and remote task cancellation have
different effects. Use the [project terms](#project-terms) below to make that
difference explicit.

## Instructions for agents and contributors

For a rule with significant conditions or consequences, make these elements
clear:

- **Applies when:** the change, input, or state that activates the rule.
- **Action:** the work the maintainer or agent must perform.
- **Evidence:** an observable result that establishes completion.
- **Exception:** the supported case where the action does not apply, if any.

These elements can appear in ordinary prose. A four-field template is not
required for each paragraph. This structure is a project convention, not an
ASD-STE100 requirement.

Use an imperative for a required action within its stated scope. Use “prefer”
for a default that permits a justified alternative. Use “may” for an option.
Do not strengthen a recommendation into a requirement during an editorial
rewrite.

Replace unclear qualifiers with decision criteria. For example, “run relevant
tests” should identify the affected behavior and the evidence needed for it.
Do not prescribe every possible test for every change.

Example instruction:

> When you change HTTP error handling in a public SDK method, preserve the
> HTTP status in the returned error. Retain its structured cause for explicit
> inspection. Keep untrusted bodies and callback text out of default diagnostics.
> Exercise the public method against a local server that returns an error.
> Assert that the caller can inspect its status without parsing the message.
> If the server echoes a secret, verify that formatted errors omit it.

This example applies to methods that return HTTP failures as errors. A method
that deliberately returns raw HTTP responses has a different contract. See
[actionable failures](engineering_principles.md#preserve-actionable-failures).

Keep instructions separate from task authorization. A clearer procedure does
not expand the requested scope or authorize publication, remote execution, or
unrelated refactoring.

## User documentation

Before a procedure, state the required tools, environment, working directory,
and service configuration. Identify placeholders and prerequisites before the
reader reaches a command that depends on them.

Use numbered steps for ordered actions. Keep commands copyable from the stated
directory. Explain the expected result after the action. Show representative
output only when it helps the reader recognize success; identify variable IDs
and states.

Describe failures that change the user's next action. Distinguish local failure
from uncertain remote outcomes. For example, cancellation of local observation
does not establish that a remote task has stopped. Do not recommend a blind
retry when the first submission may have succeeded.

In API descriptions, name the input, output, error behavior, and side effects.
Distinguish an attribute from a method and a file path from JSON text. Keep the
supported-surface table, examples, and implementation consistent.

For example, an API-root procedure should identify the deployment URL and the
required service path. It should show the request paths that result from that
configuration. See the [API-root procedure](usage.md#api-root-base_path).

Keep editorial guidance out of user task flows. Users need instructions for
the SDK, not an explanation of this repository's writing process.

## Project terms

Use these meanings in prose. Preserve exact identifiers when an API or external
specification uses another name. Add a term when an ambiguity affects use or
implementation; this table is not a substitute for the STE dictionary.

| Term | Meaning in this repository |
| --- | --- |
| API root | The configured service URL, including its deployment path prefix. |
| Origin | The URL's scheme, host, and effective port; it excludes the path. |
| Endpoint path | A path resolved relative to an API root. |
| Decode | Convert an encoded representation into a value. Decoding alone does not establish validity for an operation. |
| Validate | Evaluate a value against named constraints. State which constraints are evaluated. |
| Verify | Establish a stated claim with evidence. Name the claim and evidence. |
| Review | Examine a change against its requirements, contracts, and evidence. Review does not imply that tests ran. |
| Correctness fix | A behavior change that restores the intended contract. |
| Refactoring | A structural change that preserves supported externally observable behavior. |
| Local cancellation | A request to stop local work. It does not establish remote task cancellation. |
| Remote cancellation | A request to stop work at a service. An acknowledgement need not mean that the task has stopped. |
| API version | The service protocol contract; distinct from server software and SDK package versions. |
| Capability | Support for a specific operation or behavior. State whether it is specified, observed, overridden, or unknown. |
| Unknown | Available evidence does not establish the value or capability. |
| Unsupported | The component does not support the operation or value in the stated context. |
| Failed | An attempted operation did not meet its success criteria. State whether its remote effects are known. |
| Partial result | A result containing completed work and an explicit account of remaining or failed work. |

## Review and verification

Read the original and revised text together. Preserve actors, conditions,
negation, sequence, exceptions, and requirement strength. If a rewrite changes
the contract, identify it as a behavior or policy change and review it separately.

For an agent instruction, ask whether the reader can identify its scope,
required action, and completion evidence. For a user procedure, follow its steps
from the stated environment when execution is appropriate. Otherwise inspect
the exact commands or API paths and state the evidence limit.

Keep language findings proportional to their effect. A missing condition or a
wrong command can be a correctness defect. A sentence-length preference alone
does not block unrelated work. Use the reviewer framework to classify findings.

## Adoption and automation

Apply this guide incrementally to new and changed prose within the task's scope.
Existing documentation is not presumed to satisfy it. Plan broader rewrites as
separate work, and check technical behavior as well as wording.

The initial publication applies the guide to selected engineering rules and the
API-root procedure. This is a pilot, not a completed rewrite of all documentation.
Evaluate both audiences before adding automated enforcement. Track publication in
[#178](https://github.com/ga4gh/ga4gh-sdk/issues/178) and the evaluation in
[#185](https://github.com/ga4gh/ga4gh-sdk/issues/185), under the rollout in
[#176](https://github.com/ga4gh/ga4gh-sdk/issues/176).

The evaluation task defines fixed cases, controls, repeated runs, and reporting
criteria. Keep its results separate from this guide. Include situations where a
rule does not apply, such as a raw-response API or a documentation-only request.
Static inspection of a revised instruction establishes clarity and consistency;
it does not establish improved agent performance. Likewise, a prose review does
not replace a required installation or live-service check of a user procedure.

No prose linter or new CI gate is introduced by this guide. After the pilot,
consider a small set of checks with useful, low-noise feedback. A tool such as
[Vale](https://vale.sh/features/markup) can restrict checks to prose and exclude
code and link destinations. Start new checks as advisory. Evaluate false
positives before making any check required.

Keep a rule's wording and its automated check consistent. Link each check to
the applicable rule. Use automation for properties it can detect, such as
terminology and sentence length. Technical meaning and compliance claims still
need review; see [STEMG's guidance on tools](https://asd-ste100.org/STEsoftware.html).
