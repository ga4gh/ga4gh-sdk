# RFC: leased interactive sessions, provider contract v1

Status: proposed, operator agreement and TES integration outstanding. Tracks
[ecosystem #99](https://github.com/elixir-cloud-aai/generic-issues/issues/99).
This is an extension owned jointly by runner/controller, proxy and downstream
consumer. It adds no fields to standard `TesTask` and no portable SDK session API.

## User need and proposed deployment

A completed sequence-QC HTML report needs no live session. Use typed result
metadata and an existing viewer for that case. A Jupyter Server with an active
QC notebook kernel is a candidate when users need to reconnect to in-memory
analysis state. Operators must confirm that need and the permitted lifetime
before provisioning a production service.

The concrete deployment proposed for review is proTES -> TESK -> Kubernetes,
Jupyter Server, and an existing oauth2-proxy plus ingress authentication boundary.
These are component choices for discussion, not a claim that current TESK or
proTES implements this extension. The executable reference provider uses a local
subprocess controller and a real HTML report server behind an authenticated
HTTP proxy. It establishes lifecycle feasibility without pretending to be TESK.

| Responsibility | Proposed owner | Required evidence |
| --- | --- | --- |
| Requester and task ownership | Authenticated provider, proTES owner lookup | Another authenticated owner cannot inspect or renew a known ID |
| Task/Pod relationship | TESK controller deployment adapter | Operator records exact task ID, Pod UID, namespace and deployment |
| Readiness | Application probe, checked by controller | Process/task RUNNING precedes successful app probe |
| Route and endpoint reference | Authenticated ingress controller | Route allocated from configured ingress, never extracted from logs |
| Lease/renewal and reconciliation | Durable provider/controller | Cleanup after client exit and controller restart |
| Credential-free handle and reconnect | Downstream client | Reconnect inspects the same execution without creating a task |

## Versioned API

Provider capability discovery must advertise `leased-session/1`, the configured
proxy origin, minimum/maximum lifetime and supported readiness probe. Ordinary
TES endpoints without that capability return an explicit unsupported result;
clients must not attempt to infer support from task tags.

All operations authenticate the caller and check ownership independently of
knowledge of a task/session ID. Unknown and non-owned sessions may both return
403 to avoid disclosing existence. Endpoints below belong to the provider:

| Operation | Body / result | Semantics |
| --- | --- | --- |
| `POST /v1/sessions` | Owning execution, desired lifetime, declared service kind | Verify task ownership and supported runner before launch/attachment; idempotent for active execution + service kind |
| `GET /v1/sessions/{id}` | Session representation | Inspect/reconnect; never create a second task |
| `POST /v1/sessions/{id}/renew` | Requested lifetime, expected lease generation | Authorize again, bound renewal, reject an expired/stopped lease with 409 |
| `POST /v1/sessions/{id}/stop` | Expected lease generation | Idempotently invalidate route and request compute termination |
| `GET /v1/sessions/{id}/view` | Proxy response | Authenticate and authorize every connection; no bearer URL capability |

A session representation carries `version`, opaque `session_id`, owning
execution reference, readiness (`provisioning`, `ready`, `unavailable`),
`endpoint_reference`, `expires_at`, monotonic `lease_generation`, lifecycle
state, route state, compute state and last reconciliation error code. A durable
handle contains only provider identity, version, session ID and owning execution.
Endpoint references resolve through the provider's configured proxy. Tokens,
cookies and signed URLs do not belong in the handle.

Creation uses an idempotency key stored with its result; an uncertain create
response is reconciled by that key. Renewal uses compare-and-swap on
`lease_generation`; stop/expiry wins over concurrent renewals. The reference
prototype has one serialized controller and deliberately does not claim durable
idempotency or restart recovery.

## Cleanup and partial failure

The controller, not a client timer, enforces an absolute server-side expiry.
Before expiry it may refresh authenticated access while retaining the same
execution. At stop or expiry it durably revokes the route, closes existing proxy
sessions/connections, then terminates compute. Record each boundary separately:

1. `route_invalidation_pending`: access may still exist; urgent retry/alert.
2. `route_invalidated`: no new access; active connections must also be closed.
3. `compute_termination_pending`: access revoked, compute may still consume resources.
4. `compute_cleanup_failed`: keep the handle inspectable and retry with backoff.
5. `stopped` / `expired`: route removal and compute termination confirmed.

A renewed/recreated route must never resurrect an expired generation. On restart,
reconcile durable leases against proxy routes and task/Pod identities; default to
revoked access while ownership or expiry is uncertain. Provider crash recovery,
WebSocket termination and failed Kubernetes deletion are required backend tests,
not claims made by the local prototype.

## Executable evidence and remaining gate

Run `python3 examples/provider-contracts/session_feasibility.py --evidence /tmp/session-evidence.json`.
It starts a separate provider process and real application process, observes
RUNNING before readiness, authorizes the report through the proxy, denies a
second owner, reconnects from a client which then exits, renews without launching
a second process, and verifies server expiry, route denial and compute cleanup.
It also checks explicit stop and a credential-free handle. Static bearer tokens
are generated only for the local test; this is not a production identity system.

The following remain acceptance gates before closing #99:

- Operator confirmation of a real persistent scientific service and its access policy.
- TESK/controller/ingress ownership agreement and an actual deployment adapter.
- Durable lease generations, crash recovery, active WebSocket closure and partial cleanup evidence.
- Demonstration using that deployment after the SDK process has exited.

Only after those gates should implementation issues for the backend and a
downstream SDK adapter be opened against the ratified contract.
