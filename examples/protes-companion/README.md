# proTES downstream companion

A standalone consumer of shared SDK transport/authentication and the proTES API
at `55c4d1591cd3be883ad0b4fd2da01d49976414cb`. Proprietary endpoints and gateway
choices stay here, outside standard GA4GH clients. Tracks
[ecosystem #98](https://github.com/elixir-cloud-aai/generic-issues/issues/98).
It builds against current main without the phase-two workflow feature.

```sh
cargo run --manifest-path examples/protes-companion/Cargo.toml -- \
  explain examples/protes-companion/fixtures/provenance.json
cargo run --manifest-path examples/protes-companion/Cargo.toml -- \
  prepare examples/protes-companion/fixtures/original-task.json \
  --choices examples/protes-companion/fixtures/choices.json
cargo test --manifest-path examples/protes-companion/Cargo.toml
```

`explain` retains the exact parsed JSON record, ordered hook/forwarding attempts,
pipeline source/hash, plugin/build identities, changed paths, final reason and
unknown extensions. The fixture has two attempts, a changed path and an unknown
forwarding outcome. There are no unavailable old/new mutation values to reconstruct,
and provenance is operational evidence rather than cryptographic attestation.

`fetch <task-id>` uses the TES section of `~/.ga4gh-cli/config.json`, shared
transport headers and the escaped `tasks/{id}/provenance` endpoint. HTTP errors
retain their status/body: 403 may cover a non-owned or unknown task; 404 can mean
absent provenance or another documented absent case. The consumer does not guess
which case occurred. Set `oauth_access_token` in that config section for CLI
bearer authentication; the companion injects one authenticated reqwest client
into shared SDK transport for discovery, provenance and explicit create. It
does not implement login/refresh or silently treat unsupported auth as anonymous.
Library callers can inject another authenticated transport. Inspection never submits, including `outcome_unknown`.

`prepare` requires a caller-supplied **original client request**. It does not fetch
FULL views as reconstruction input because those may contain plugin mutations.
Without original intent, explanation remains available and replay reconstruction
is marked unavailable. The original-input label is a caller assertion, not proof
that a supplied file was never transformed.

The plan whitelists writable TES fields, removes id/state/logs/creation_time and
all stored `gateway.*` tags. Optional explicit choices rebuild only
`gateway.pipeline` (a preset/inline pipeline, subject to operator policy) and
`gateway.backends` (a JSON-encoded object with `type` order/allow/forbid and backend
URLs), matching the pinned API. It never copies recorded backend IDs/routes.
The immutable prepared request is submitted as exact JSON; a round trip through
older generated models must not silently discard nested TES 1.1 fields. Malformed
creation acknowledgements report an uncertain outcome and require reconciliation,
without another POST.
Unknown top-level fields are dropped with a diagnostic list rather than promoted
into a new request. Review the JSON plan before a separate explicit `create`
command using the same original request and choices; that command may launch a
new execution. It has no automatic replay or failover path.

Validation includes the actual offline CLI, typed/raw preservation, changed paths,
ordered attempts, sanitization, OAuth header reuse and HTTP 403/404 boundaries.
A real local proTES gateway at the pinned revision was exercised with its upstream
smoke pipeline and `smoke.tag` plugin. A controlled fixture TES returned a known
400 rejection, the second fixture accepted, and companion fetch retained both
ordered attempts, actual build hashes and `changes.paths`. A separate 503 fixture
produced one `outcome_unknown` attempt and no failover; both fixture stores were
read to reconcile the absence of execution, without replaying that intent.
`changes.before`/`after` are gateway HMAC evidence, not old/new mutation values.

The gateway, plugin engine, MongoDB/RabbitMQ persistence and companion were real.
The upstream TES backend is a controlled fixture, so this does not demonstrate
scientific container execution through TES or an external federated deployment.
Use the standard SDK execution suites for runner acceptance. The public fixtures
here are synthetic and preserve no credentials or private deployment details.
