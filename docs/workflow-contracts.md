# Experimental workflow contracts

Enable the library's `workflow-contracts` feature. The CLI enables it for the
`workflow` subcommands. This optional client-side layer consumes the I/O Profile
and UI vocabulary at revision `45d41324353cb4900fdd999c307c2e36a3f2a410`.
The separate `wes` commands operate the WES service API directly; `workflow`
adds local validation, result evidence and comparison around an explicit WES
submission. It is not another execution service.
It adds no fields to standard WES requests and performs no execution binding or
TES command generation. The workflow engine owns staging and execution.
The [example file map](../examples/sequence-qc/README.md) and
[upstream schema provenance](../lib/src/workflow/schemas/README.md) identify the
inputs used below.

## Prepare and explicitly submit

```sh
pin=45d41324353cb4900fdd999c307c2e36a3f2a410
cargo run -p ga4gh-cli -- workflow prepare \
  examples/sequence-qc/profile.json examples/sequence-qc/request.json public-sequence-qc@1 \
  --profile-revision "$pin" --vocabulary-revision "$pin" > prepared.json
```

`invalid-request.json` produces a nonzero exit code, empty stdout, and JSON
parameter diagnostics in stderr with instance and schema locations. Preparation
works without a service configuration or network. Profile documents and parameters
are checked with Draft 2020-12. Selecting inputs/outputs retains the root document's
reference scope. Meta-schema/vocabulary resources are vendored inside the library;
unknown required vocabularies and unresolved resources fail. Remote parameter
URIs are validated as strings, without downloading their contents. Defaults and
UI annotations do not insert or modify parameters.

The immutable library `PreparedInvocation` records exact request parameters,
workflow/descriptor identity, exact schema-byte digest, pins, the complete profile,
expected outputs and a fingerprint. Private fields and the absence of Deserialize
prevent submitting an edited serialized plan as a validated object. Prepare again
for changed parameters, identities or schemas. Preparation rejects a request whose
`extra` fields name a RunWorkflow part, such as `workflow_params` or `tags`, as
submission does. The fingerprint is the SHA-256 of
the plan's canonical JSON (compact, keys sorted at every level), so it doesn't
depend on the order of map entries such as the request's `tags` and
`workflow_engine_parameters`; parameter values are retained. A caller supplies
workflow identity; preparation is not an independent attestation that a remote
workflow URL is immutable.

`PreparedInvocation::submit` composes the existing WES API. For local descriptors,
`submit_with_attachments` checks a supplied `sha256:` primary-descriptor digest
against the uploaded bytes. Multiple-file dependency verification can compose
with `clients::bundle::PreparedBundle`. A checksum supplied as context does not
prove which bytes an engine fetches later from a mutable remote URL.

Explicit CLI submission uses the `WES` section of `~/.ga4gh-cli/config.json`.
Downstream CLI callers can instead supply a `Configuration` through
`ga4gh_cli::dispatch_with_config()`.
For OAuth, set `oauth_access_token` to the current token. `workflow submit`
uses the same WES client and authentication path as `wes run`, including Basic
and generic bearer sources. The CLI does not implement login or token refresh.
For custom HTTP policy, use `WES::from_transport` with a caller-owned client;
see the [authentication guide](authentication.md):

```sh
cargo run -p ga4gh-cli -- workflow submit \
  examples/sequence-qc/profile.json examples/sequence-qc/request.json public-sequence-qc@1 \
  --profile-revision "$pin" --vocabulary-revision "$pin" \
  --attachment main.cwl=examples/sequence-qc/main.cwl
```

Add `--descriptor-digest sha256:<digest>` to bind the primary upload. Preparation
and submission are separate commands. Submission may execute a workflow and never
retries it automatically. Natively shaped WES 1.0 service-info responses, including
Toil's response without common discovery fields, are supported outside generated
models; cached discovery still retains the raw response.

## Typed outputs and selected artifact evidence

```sh
cargo run -p ga4gh-cli -- workflow results profile.json run-log.json wes/run-id \
  --profile-revision "$pin" --vocabulary-revision "$pin" > result.json
cargo run -p ga4gh-cli -- workflow verify result.json /report/location report.html \
  --sha256 <expected-sha256> --evidence-source pinned-manifest@revision > verified.json
```

`Profile::project_results` matches declared output properties and preserves their
nested data and schema annotations. An output contract must expose named
`properties`; schemas which describe only opaque output data fail explicitly.
Output names can be any string, including the empty string, `~`, characters that
URIs reserve or forbid (such as `/`, `#`, `%`, `?`, `[`, `]`, `{`, `}`,
`|`, `^` and spaces) and non-ASCII characters.
The durable `contract_document` preserves root scope for referenced scientific
and viewer annotations; consumers can resolve those references without fetching
a schema. The small viewer helper recognizes inline hints, while a richer viewer
owns reference/conditional hint interpretation.
Overall schema diagnostics cover cross-property constraints as well as per-output
status. Viewers should require both a conformant output and an acceptable overall
contract result. Optional absent outputs are conformant absent values; required
absent outputs are `missing`. A COMPLETE execution remains COMPLETE with incomplete
delivery. The report is JSON serializable and has no presentation dependency.

The result separates execution state, contract status, access status and integrity.
Only explicitly selected bytes are verified. An expected SHA-256 with an evidence
source can produce `verified` or `mismatch`; computing a new digest alone remains
`unverified`. No expected checksum is inferred from size, filename or ETag. A
malformed expected checksum (another algorithm, a digest that isn't 64 hex digits
or a blank evidence source) fails before anything is read, resolved or recorded,
also when the output is missing or the read fails.
Expected evidence is supplied by the caller and is not itself cryptographically
attested by this API.

`verify_access_plan` consumes an ephemeral DRS access plan using its object-store
headers, a fresh client without SDK credentials, no redirects, an operation budget
and a hard byte bound. `verify_drs` delegates negotiation and deferred resolution
to the owning `DrsResolver`; callers can also supply plans/evidence obtained from
bulk object resolution. Failed access preserves sibling outputs. Retry an expired
capability only by explicitly asking its owning resolver for a new plan. There is
no automatic bulk download or new credential store.

Durable HTTP(S) references have userinfo, query and fragment removed and mark
`access_details_redacted`; they must not be treated as refreshed usable access
URLs or guaranteed object identities. DRS references retain stable object identity.
AccessPlan headers and URLs are never serialized into artifact evidence.

The same redaction pass applies to the stored `contract_document` and output
annotations. The exceptions and limits below define what remains in a record:

- **Scope:** HTTP(S) URLs that make up a whole string, URLs embedded in longer
  strings, such as descriptions or log lines, and URLs inside other URLs. URLs are
  found and redacted by their text, so URLs that don't parse are covered too.
- **Where a URL starts:** at `http:` or `https:`, in any case, followed by any
  number of `/` or `\`, none included, as URL parsers accept them. This includes
  JSON-escaped `https:\/\/`.
- **Credentials:** userinfo through the last `@` before the next `/`,
  `\\`, `?`, `#` or whitespace is removed; the scheme and leading slashes remain.
  If the apparent authority has a password rather than a numeric port,
  a raw separator in the password is included up to the last `@` before
  whitespace.
- **Query and fragment:** everything from the first `?` or `#` after the
  scheme up to the next whitespace, except a trailing run of `.`, `,`, `;`,
  `:`, `)`, `]` or `}`, and a closing quote, angle bracket or backtick
  paired with one immediately before the URL.
  Parsers can accept those characters inside queries, so an unpaired one is
  removed with the query. Unspaced prose after a URL can be removed too.
  Credentials or queries with whitespace in them are only partly removed.
- **References:** the values of `$ref`, `$dynamicRef`, `$id` and `$schema` keep a
  fragment that is a JSON pointer or an anchor, so that references still resolve.
  Pointers with `?`, `@`, `=` or `:` aren't kept.
- **Patterns:** `pattern` values are regular expressions and stay as they are.
- **Instance data:** in the values of `default`, `const`, `examples`, `enum` and
  extension keywords (`x-...`), keys aren't keywords, so the two exceptions above
  don't apply there.
- **Output values:** a whole-string URL is also normalized, as before.
  Strings that are valid serialized JSON are processed string by string:
  object keys and unrelated values stay in place, while a changed string
  value may use normalized JSON escapes. Other unchanged text stays as given.
- **Digest:** `schema_digest` still identifies the profile as given.

Not redacted: other URL schemes (for example presigned `s3://` URLs), object keys,
checksum source labels and caller-owned context. Do not put credentials there. The
result format is evidence transport, not a generic secret-discovery or redaction
system.

`html_viewer_metadata` exposes declared HTML viewer metadata. The small adapter
in `examples/sequence-qc/view_report.py` feeds the **known public demo reports** to
IPython's existing `HTML` viewer. It does not authorize arbitrary remote content.
Other hints remain schema metadata; CrateGen continues to own RO-Crate conversion.

## Compare existing results

```sh
cargo run -p ga4gh-cli -- workflow compare policy.json left.json right.json
```

A version-1 policy names JSON output pointers, comparator and version, required
identity/context keys, tolerance and resource bounds. The supplied QC policy
compares sequence count and total length exactly and mean length with an explicit
absolute tolerance. Required workflow, schema and input identity must be available
and equal; missing evidence or required identity drift makes the overall conclusion
inconclusive even when metrics agree. Backend/image/reference-data evidence belongs
in context. Declare any of those keys required when equivalence needs them to match.
Context drift is reported independently.

The numeric rule is `abs(a-b) <= max(abs_tolerance,
relative_tolerance * max(abs(a), abs(b)))`, inclusive at the boundary and defined
at zero. JSON decimal numbers are compared using rational arithmetic, preserving
integers above 2^53. Negative/non-finite policy tolerances are errors. JSON cannot
represent NaN/infinity; a value missing at the compared output pointer, invalid
schema values, exhausted node/depth bounds and JSON pointers longer than 2048
bytes are inconclusive. Within a compared value, a definite difference
(a differing value, a key on one side only, or arrays of different lengths) makes
the output different; otherwise any compared `null` makes it inconclusive. The
outcome doesn't depend on key order. Large input files are bounded by the CLI.

Object member order is immaterial. Array order is significant unless explicitly
ignored. Unordered-array mode is restricted to zero tolerance and compares canonical
JSON ordering; tolerant multiset matching is outside version 1. Ignored JSON pointers
are relative to the selected output and apply only where explicitly listed. No
arbitrary comparator executable is loaded. A digest comparison requires internally
consistent verified SHA-256 evidence with its expected digest/source; matching
unverified digests are inconclusive. Result records are caller-supplied evidence,
not signed attestations. Conclusions cover only the selected outputs and policy.

## Demonstration and evidence boundary

`tests/run-workflow-contracts-qc.py` starts two independent local Toil WES 9.3.0 processes and
runs a public synthetic sequence-QC CWL workflow with a pinned Python image. It
prepares parameters, explicitly uploads the bound descriptor, waits for COMPLETE,
projects metrics and the report, verifies golden report bytes, compares the two
runs and feeds both reports into IPython. It also distinguishes changed input
identity and a missing output. Set `GA4GH_TOIL_ENV_DIR` to reuse a prepared venv.
Use a Docker context whose filesystem mounts include this checkout. The optional
`GA4GH_QC_NO_CONTAINER=1` host-execution mode must be reported as such.

This is evidence for two **local** WES instances. It does not demonstrate independent
ELIXIR deployments, LS Login, Conquest or a federated scientific validation campaign.
Those require explicit authorized endpoints and remain deployment acceptance work.
