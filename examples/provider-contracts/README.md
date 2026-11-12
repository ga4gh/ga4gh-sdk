# Provider-contract feasibility prototypes

These executable experiments support the proposed joint contracts in
[`docs/rfcs/leased-sessions.md`](../../docs/rfcs/leased-sessions.md) and
[`docs/rfcs/workload-secrets.md`](../../docs/rfcs/workload-secrets.md).
They are not portable GA4GH extensions or production SDK adapters.

```sh
python3 examples/provider-contracts/session_feasibility.py --evidence /tmp/session.json
DOCKER_CONTEXT=<your-context> python3 examples/provider-contracts/openbao_feasibility.py /tmp/openbao.json
```

The session experiment starts its own loopback-only HTTP provider/proxy and real
report process. It checks readiness, owner access, reconnect, renewal, server-side
expiry after a client exits, cleanup and explicit stop. Test credentials are
random and transient. It needs local sockets and creates no external deployment.
Its process controller and bearer auth stand in for deployment components;
capability discovery, lease generation CAS, persistent reconciliation and real
ingress/WebSocket cleanup remain backend work.

The OpenBao experiment requires Docker and `openssl`. It runs a pinned OpenBao
2.7.2 dev container with real JWT auth and KV-v2, generates a temporary RSA test
issuer, hands access to a separate QC subprocess over stdin, and verifies workload
and intent binding, expiration and revocation. It cleans its own container and
private key files. Evidence contains no credential values. Never use this dev
server or test issuer as a production secret-delivery system. Record inspection
covers local public fixtures; actual TES persistence/runner identity integration
is explicitly still unproved. Its intent digest is a fixed fixture; the RFC
requires a shared canonicalization rule before a real broker or runner uses it.

Both RFC issues stay open until their operator/backend gates are satisfied.
