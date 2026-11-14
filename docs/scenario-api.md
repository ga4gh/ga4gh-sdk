# Shared synthetic GA4GH scenario API

This optional downstream prototype serves the same versioned, language-neutral
[scenario pack](../testbed/pack/scenarios.json) to independent clients. It never
executes submitted containers or fetches submitted URLs. Exact TES 1.1.0, WES
1.1.0, DRS 1.4.0 and TRS 2.0.1 revisions and SHA-256 hashes are recorded in
[spec-pins.json](../testbed/pack/spec-pins.json); their OpenAPI snapshots ship in
the pack. DRS/TRS expose only the fixture routes used by this flow.

```sh
python3 testbed/server.py --port 8815
python3 tests/run-h3-scenarios.py
# Validate an actual OCI deployment instead of starting another source server:
python3 tests/run-h3-scenarios.py --url http://127.0.0.1:8815
```

The cross-client harness needs Rust, Node 22+ and npm. It pins TypeScript 5.9.3,
checks the upstream source hashes, and compiles the unmodified maintained
`RestWesProvider` at cloud-components revision
`44c6f96fbfbeff6892ba0ca3836a7bcbd7c9069a`. The only subclass override supplies
session authentication through its documented `fetch` hook. Node's native
fetch/FormData/File exercise its browser API surface; this is not a browser CORS
or DOM integration claim. The Rust side injects an authenticated `reqwest`
client through the existing public `Transport::with_client` surface. Filling
configuration authentication fields alone is not currently an implemented auth
path (upstream SDK #31).

Both clients execute multipart submission including an actual CWL attachment,
opaque pagination, status/log retrieval, cancellation with pending state,
completion winning a cancellation race, accepted POST with lost response,
bounded overload and intentionally malformed JSON. Their WES event histories
must match. Evidence and replay bundles are written under
`target/h3-scenarios/`; they count three accepted WES runs, including the lost
response. The SDK additionally exercises TES create/get/cancel and ambiguous
submission. The Python HTTP suite verifies DRS 202/Retry-After, actual binary
bytes and checksum, expiry, concurrent isolation, reset and reproducible replay.
No automatic submission retry is performed after the simulated server accepts a
request but closes its socket.

## Control and GA4GH routes

`POST /control/v1/sessions` accepts `{"seed":17,"ttl_seconds":600}` and returns
an opaque session ID, bearer token and base path. Use the token for every request
under that session, including control operations. Session IDs alone never grant
access. Tokens are synthetic, ephemeral capabilities; do not reuse account
credentials. The server limits sessions to 32, executions per session to 32,
request bodies to 1 MiB, TTL to one hour and a configured fault to five responses.
Expiry uses server monotonic time, independent of client connection or logical
scenario time. Expired sessions return 410 and are reclaimed on allocation.

- `POST /control/v1/sessions/{id}/events` accepts `{"action":"advance","ticks":1}`
  to advance one protocol transition and logical time, or `{"action":"fault",
  "operation":"wes.create","name":"lost-response","count":1}`.
- Fault operations are `tes.create`, `tes.list`, `tes.cancel`, `wes.create`,
  `wes.list`, `wes.cancel`, `drs.get`. Named fault compatibility is validated.
- `GET .../{id}/journal` returns sanitized events and acceptance counts.
- `GET .../{id}/replay` exports version/image identity, spec and pack hashes,
  seed, accepted synthetic request inputs, attachment bytes and ordered events.
- `POST .../{id}` resets only that authorized session; it does not renew expiry.
- GA4GH roots are `/sessions/{id}/tes/v1`, `/wes/v1`, `/drs/v1`, `/trs/v2`.
  TES only accepts the pack task; WES accepts the pack request and attachment.
  Control fields never appear as invented TES/WES model members.

The journal excludes authorization, session IDs, request headers and caller
metadata. Replay only contains allowlisted synthetic inputs. The same seed and
ordered events reproduce identities and responses, without promising thread or
packet scheduling. `malformed-json` is an intentional failure input; the pack
separately identifies normative cases and provider-specific profiles.

## OCI distribution and hosted boundary

```sh
docker build --tag ga4gh-scenario-api:0.1.1 testbed
docker run --rm -p 127.0.0.1:8815:8815 ga4gh-scenario-api:0.1.1
# Export a standard content-addressed OCI archive, with verified layer diff IDs:
docker image save --output /tmp/scenario.docker.tar ga4gh-scenario-api:0.1.1
python3 testbed/export-oci.py /tmp/scenario.docker.tar /tmp/scenario.oci.tar
# Docker loads its own save format; OCI layout archives need an OCI-aware importer.
docker image load --input /tmp/scenario.docker.tar
# Or import the OCI archive with Skopeo:
skopeo copy oci-archive:/tmp/scenario.oci.tar docker-daemon:ga4gh-scenario-api:0.1.1
```

The image uses a digest-pinned Python base and an unprivileged user. The archive
records the platform and manifest digest. For a replay tied to a packaged image,
pass its manifest digest as `--image-digest sha256:...` after the image's default
ENTRYPOINT. `unpackaged-source` explicitly identifies local source runs. An OCI
release archive can be pinned and downloaded in CI without a hosted API dependency.

An optional LAN source listener uses `--host 0.0.0.0`; use only synthetic data.
For clients on another host, also set `--public-base-url` to the HTTP(S) origin
that routes back to this server. DRS artifact URLs use this configured origin,
not the request's `Host` header; keep it on the same trusted service. Without
an explicit origin, a wildcard listener advertises `127.0.0.1` and supports
local clients only.
Browser clients can add exact `--cors-origin https://client.example` values;
there is no permissive wildcard. HTTP is for local/internal testing. Public
hosting, TLS, operator authentication, resource quotas beyond this in-memory
service and Reference Cloud integration remain a separate operator stage tracked
by generic #103 / Reference Cloud #34. State intentionally resets on service
restart; exported replay artifacts carry reproduction evidence, not durable jobs.
Actual staging, execution and worker termination require real backend profiles.
