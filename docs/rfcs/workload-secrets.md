# RFC: workload-bound OpenBao requirements, provider contract v1

Status: proposed; real OpenBao primitive evidence available, production runner
identity and requester authorization integration outstanding. Tracks
[ecosystem #100](https://github.com/elixir-cloud-aai/generic-issues/issues/100),
with related delegated identity [proTES #254](https://github.com/elixir-cloud-aai/proTES/issues/254)
and [security/provenance umbrella #69](https://github.com/elixir-cloud-aai/generic-issues/issues/69).

## Concrete contract proposed for review

Use proTES -> TESK -> Kubernetes as the candidate runner path, an operator-owned
attestation broker, OpenBao JWT auth, and KV-v2 for a benign protected QC input.
This contract does not add a standard TES field, make task tags into identity,
or put an OpenBao token exchange in the SDK. Requirements are an independently
versioned provider sidecar correlated with the original execution intent.

| Boundary | Owner | Binding and failure behavior |
| --- | --- | --- |
| Requester authorization | Broker using authenticated API identity | Authorize one opaque secret reference and purpose before launch |
| Execution intent | Broker + gateway launch registry | Store canonical request digest and authorized requirement outside user-writable tags |
| Workload identity | TESK controller + Kubernetes verifier | Verify projected token with TokenReview, audience, namespace, Pod UID and controller launch record |
| Attestation to OpenBao | Operator broker issuer | Short-lived signed assertion binds requester, task/Pod identity, intent digest and allowed secret reference |
| Secret engine and policy | OpenBao operator | Map the authorized reference to a narrowly scoped path; never accept a caller-supplied arbitrary path |
| Handoff to workload | Runner-owned broker channel | Deliver ephemeral access over authenticated local channel or protected pipe; no argv, executor.env, task JSON or logs |
| Requirement and handle | Downstream SDK consumer | Describe opaque requirements; reject unsupported capability before launching protected compute |

A Kubernetes ServiceAccount name alone is shared by multiple Pods and is
insufficient. A visible reference, task ID, user tag or unsigned intent digest
proves no workload identity. The broker must verify both Pod UID and the
operator's task-to-Pod launch binding, then mint the assertion itself. A
compromised issuer or runner remains inside the trust boundary.

## Sidecar and provider operations

A `workload-secret-requirement/1` sidecar contains provider identity,
`secret_ref`, purpose, requested maximum lifetime and canonical execution-intent
digest. It contains no secret value, authentication header, wrapping token,
redemption token or credential-bearing URL. A wrapping token is a bearer
capability and must receive the same protection as the secret.

Before implementation, the broker and runner must agree on a versioned digest
algorithm: which original request fields are included, how JSON is canonicalized,
and whether any mutable or credential-bearing values are excluded. The digest
must be calculated from the same client intent at authorization and launch; a
user-supplied digest alone is not proof of that binding. The feasibility script
uses a fixed fixture digest and does not test cross-component canonicalization.

The authenticated provider exposes capability discovery and an authorization
operation which returns a credential-free requirement/grant handle. Before
launch it confirms supported runner/identity mechanism and allowed engine/path.
The broker records an authorization decision linked to requester and intent;
public gateway provenance may record only the opaque reference and decision code.

At redemption, the runner supplies verified identity over a protected channel.
The broker checks authorization, expiry, revocation, exact launch binding and
intent, and issues a short-lived assertion for the configured OpenBao JWT role.
JWT claims include issuer, audience, subject unique to that execution instance,
requester, intent digest, allowed reference, expiry and nonce. OpenBao validates
the signature and bound claims before returning a narrowly scoped access token.
No caller controls the issuer key or the role's path/policy mapping. The
production broker must track nonces/grant state to prevent unintended replay;
OpenBao JWT login alone is not a single-use handoff protocol.

The runner obtains this response at the protected boundary and passes current
access to its workload without exposing it in persisted TES views or gateway
provenance. The SDK never fetches plaintext. A denied/expired grant fails closed;
there is no plaintext environment or URL fallback. Renewal requires fresh
identity and policy checks; it is not an automatic resubmission of the TES job.

## Lifetime and revocation semantics

For KV-v2, expiring/revoking the OpenBao token revokes future reads. It cannot
revoke a secret value already returned, and rotation of that static value belongs
to the protected service owner. KV version deletion is also not credential
revocation at a downstream service. The feasibility test makes only this static
read-access claim.

For a later dynamic database engine, the grant must retain an operator-only
lease reference and revoke the issued credential at its owning engine. That is
an additional implementation and test gate. It must prove credential rejection
at the database after revocation, not merely a failed subsequent OpenBao read.
If access expires during execution, report a distinct access-expired failure or
perform an explicitly allowed broker renewal. Task execution state and secret
access state remain separate observations.

A workload legitimately given plaintext can disclose it. This contract controls
authorization and delivery; it does not prevent arbitrary-code exfiltration.

## Real OpenBao feasibility evidence

Run `DOCKER_CONTEXT=<context> python3 examples/provider-contracts/openbao_feasibility.py /tmp/openbao-evidence.json`.
The script uses OpenBao 2.7.2 pinned by image digest, its real JWT auth plugin and
real KV-v2 engine. It generates a temporary RSA test issuer and binds a JWT role
to workload subject, audience, requester, intent and reference. A separate QC
subprocess obtains the fixture through a protected stdin handoff and returns only
its length. Another workload with the same visible reference and a workload with
an altered intent are denied. Expired assertions, expired read tokens and revoked
read tokens are denied. Containers and private issuer material are removed after
the run; evidence contains no credentials.

The test verifies credential absence in its local public request/sidecar and
provenance fixtures. It **does not** establish credential absence in a deployed
TES/gateway store, attest a Kubernetes Pod, verify a digest against a real TES
request or implement a production issuer.
The local test issuer is explicitly outside user task metadata.

## Completion gates and implementation boundaries

Do not close #100 as delivered until operators agree the runner identity and
secret-engine contract and demonstrate the complete path:

- Verify actual requester authorization and task -> Pod UID -> attested subject binding.
- Ratify and test the versioned execution-intent digest across authorization,
  gateway launch records and the runner.
- Reject an unsupported runner before creating protected work.
- Prove the selected grant's allowed workload succeeds and a different workload cannot redeem it.
- Inspect real persisted FULL task views, gateway provenance and operational logs for handoff leakage.
- Demonstrate expiry/renewal and revocation appropriate to the chosen engine.
- Review broker channel protection, issuer/key lifecycle and replay controls.

Then create backend/broker and downstream-adapter implementation issues. Keep
ordinary API login (#31/#90) and gateway/backend credentials separate from secrets
consumed inside the workload.

References: [OpenBao JWT API](https://openbao.org/api-docs/auth/jwt/),
[OpenBao KV-v2 API](https://openbao.org/api-docs/secret/kv/kv-v2/).
