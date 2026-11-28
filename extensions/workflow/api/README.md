# Programmatic workflow contract

This directory defines the in-memory protocol for `ga4gh.workflow`, independently
of `ga4gh.cli.commands`. The provider advertises revision 1 alongside the CLI
capability. ABI v1, capability revision, provider release and binding versions
are independent. Always negotiate the capability on the exact installed artifact.
Do not fall back to CLI invocation when negotiation fails.

## Preparation (revision 1)

`prepare` takes a UTF-8 JSON message matching `v1/prepare-request.schema.json`.
The capability revision and operation are supplied by the invoke ABI, not by a
field in the message. Limit each request and reply to 64 MiB of encoded bytes.
The caller supplies `profile_utf8` as the original UTF-8 profile text, including
whitespace and the final newline. JSON escaping in transit is fine; parsing the
profile and serializing it again changes its byte digest. `request` is the WES
run request, including any extension fields. The schema checks the envelope;
the workflow implementation validates the pins, profile and request semantics.
No WES client, credentials, file paths to read, or network action are needed.

`v1/prepare-reply.schema.json` distinguishes a prepared result from a public
workflow error. A successful reply carries the existing serialized preparation
record unchanged, plus an opaque live-state reference. The reference belongs
to the instance that issued it and must not be reused for a later preparation
after release. Bindings must retain that instance, not look up a new provider
by name. A saved record or fingerprint is **not** a state reference: to import
a plan, call `prepare` again with the original profile bytes, request, pins and
identity. Never submit by deserializing the exported record.

Validation errors may include JSON-pointer diagnostics. Their public messages
must not expose private native error payloads. ABI failures and unsupported
capability revisions are loader errors, not workflow error replies. WES errors
are separate: `prepare` does not submit anything.

Each instance keeps at most one live preparation. A second `prepare`
returns `state_in_use` without replacing the first plan. Call `release` with
`{"state_ref":"<reference>"}` on that instance; success returns `{"released":true}`.
A stale or foreign reference returns `invalid_state` in the workflow error
envelope and leaves any live plan untouched. A malformed release returns
`invalid_request`. Destroying the instance also releases its preparation.
An oversized prepared reply returns `result_too_large` without allocating a
live-state reference for the caller. Input-schema failures retain their JSON
pointers without echoing rejected parameter values in their messages.

## Results and submission

`v1/operations.schema.json` names each request and reply in `$defs`. `results`
projects the supplied run reference, state and named outputs; `verify` adds
evidence to a result record using supplied bytes (padded standard Base64);
`compare` returns the existing policy report. These calls perform no I/O.
Success returns `{"ok": ...}`; a workflow error uses the `error` envelope.

`begin_submit` accepts the live reference and attachment bytes, validates them
and returns the WES request and bytes to send. With a pinned descriptor digest
the primary attachment is mandatory. Once it returns an action the instance
rejects another `begin_submit` for this plan, even if acknowledgement is lost.
The host calls its own WES client once and invokes `finish_submit` on the same
instance with either `{"submitted":{"run_id":"..."}}` or `"failed"`. Completion
consumes the plan. The host retains the actual WES error, or the acknowledged
run ID if the provider fails to produce a matching receipt. No credentials,
service URLs from configuration, or private failure diagnostics are sent to the provider.
