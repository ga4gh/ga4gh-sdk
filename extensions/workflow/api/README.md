# Programmatic workflow contract

This directory defines the in-memory protocol for `ga4gh.workflow`, independently
of `ga4gh.cli.commands`. Revision 1 is in development; the current native provider
does **not** advertise it. Do not fall back to CLI invocation when negotiation fails.

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
