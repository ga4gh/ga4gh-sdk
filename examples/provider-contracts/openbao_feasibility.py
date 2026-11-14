#!/usr/bin/env python3
"""Real OpenBao JWT + KV-v2 feasibility. Local test issuer, not a production TES identity adapter."""
import base64, hashlib, json, os, pathlib, secrets, subprocess, sys, tempfile, time, urllib.error, urllib.request

if not __debug__:
    raise SystemExit("Run the feasibility checks without Python optimization (-O)")

IMAGE = "openbao/openbao@sha256:5046911075e137e154ce49ab051502f4ec7cf367d484ae4a4dce665d9b90561f"


def b64(data):
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def api(base, path, data=None, token=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["X-Vault-Token"] = token
    request = urllib.request.Request(
        base + "/v1/" + path,
        data=None if data is None else json.dumps(data).encode(),
        headers=headers,
    )
    try:
        with urllib.request.urlopen(request, timeout=5) as response:
            raw = response.read()
            return response.status, json.loads(raw) if raw else {}
    except urllib.error.HTTPError as error:
        return error.code, json.loads(error.read())


def worker():
    # Protected broker->workload stdin, never argv, env, a TES tag, or a persisted record.
    payload = json.load(sys.stdin)
    status, record = api(
        payload["base"], "secret/data/qc-input", token=payload["token"]
    )
    if status != 200:
        sys.exit(3)
    text = record["data"]["data"]["sequence"]
    print(json.dumps({"sequence_length": len(text)}))


if "--workload" in sys.argv:
    worker()
    sys.exit()


def main():
    name = "ga4gh-phase2-bao-" + secrets.token_hex(5)
    root_token = secrets.token_urlsafe(32)
    docker = ["docker"]
    output = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
    with tempfile.TemporaryDirectory(prefix="ga4gh-bao-") as directory:
        directory = pathlib.Path(directory)
        private = directory / "issuer.pem"
        public = directory / "issuer.pub"
        subprocess.run(
            [
                "openssl",
                "genpkey",
                "-algorithm",
                "RSA",
                "-pkeyopt",
                "rsa_keygen_bits:2048",
                "-out",
                str(private),
            ],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        private.chmod(0o600)
        subprocess.run(
            ["openssl", "pkey", "-in", str(private), "-pubout", "-out", str(public)],
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        env = os.environ.copy()
        env["BAO_DEV_ROOT_TOKEN_ID"] = root_token
        subprocess.run(
            docker
            + [
                "run",
                "--detach",
                "--rm",
                "--name",
                name,
                "--cap-add=IPC_LOCK",
                "--publish",
                "127.0.0.1::8200",
                "--env",
                "BAO_DEV_ROOT_TOKEN_ID",
                IMAGE,
                "server",
                "-dev",
                "-dev-listen-address=0.0.0.0:8200",
            ],
            env=env,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        try:
            port = (
                subprocess.check_output(docker + ["port", name, "8200/tcp"], text=True)
                .strip()
                .split(":")[-1]
            )
            base = "http://127.0.0.1:" + port
            for _ in range(100):
                try:
                    if api(base, "sys/health")[0] == 200:
                        break
                except OSError:
                    pass
                time.sleep(0.1)
            else:
                raise RuntimeError("OpenBao readiness timeout")

            def admin(path, data):
                status, result = api(base, path, data, root_token)
                if status not in (200, 204):
                    raise RuntimeError(
                        "OpenBao setup failed at "
                        + path
                        + " with status "
                        + str(status)
                    )
                return result

            admin("sys/auth/jwt", {"type": "jwt"})
            admin(
                "auth/jwt/config",
                {
                    "jwt_validation_pubkeys": [public.read_text()],
                    "bound_issuer": "ga4gh-phase2-test-runner",
                },
            )
            admin(
                "sys/policies/acl/qc-test",
                {
                    "policy": 'path "secret/data/qc-input" { capabilities = ["read"] } path "auth/token/revoke-self" { capabilities = ["update"] }'
                },
            )
            fixture = "ACGTAC"
            admin("secret/data/qc-input", {"data": {"sequence": fixture}})
            intent = "sha256:" + hashlib.sha256(b"public-qc-intent-v1").hexdigest()
            reference = "protected-qc-input-v1"
            admin(
                "auth/jwt/role/qc-grant",
                {
                    "role_type": "jwt",
                    "bound_subject": "runner/workload-a",
                    "bound_audiences": ["qc-broker"],
                    "user_claim": "sub",
                    "bound_claims": {
                        "execution_intent": intent,
                        "requester": "demo-owner",
                        "secret_ref": reference,
                    },
                    "token_policies": ["qc-test"],
                    "token_no_default_policy": True,
                    "token_ttl": 3,
                    "token_explicit_max_ttl": 3,
                    "clock_skew_leeway": -1,
                    "expiration_leeway": -1,
                    "not_before_leeway": -1,
                },
            )

            def assertion(subject="runner/workload-a", expiry=30, intent_claim=intent):
                now = int(time.time())
                payload = {
                    "iss": "ga4gh-phase2-test-runner",
                    "sub": subject,
                    "aud": "qc-broker",
                    "iat": now,
                    "nbf": now - 1,
                    "exp": now + expiry,
                    "execution_intent": intent_claim,
                    "requester": "demo-owner",
                    "secret_ref": reference,
                }
                unsigned = (
                    b64(json.dumps({"alg": "RS256", "typ": "JWT"}).encode())
                    + "."
                    + b64(json.dumps(payload).encode())
                )
                signature = subprocess.run(
                    ["openssl", "dgst", "-sha256", "-sign", str(private)],
                    input=unsigned.encode(),
                    capture_output=True,
                    check=True,
                ).stdout
                return unsigned + "." + b64(signature)

            def login(jwt):
                return api(base, "auth/jwt/login", {"role": "qc-grant", "jwt": jwt})

            status, grant = login(assertion())
            assert status == 200, "authorized workload rejected"
            token = grant["auth"]["client_token"]
            completed = subprocess.run(
                [sys.executable, __file__, "--workload"],
                input=json.dumps({"base": base, "token": token}),
                capture_output=True,
                text=True,
                check=True,
            )
            assert json.loads(completed.stdout) == {"sequence_length": 6}
            other_status, _ = login(assertion(subject="runner/workload-b"))
            assert other_status in (400, 403)
            wrong_intent, _ = login(assertion(intent_claim="sha256:other"))
            assert wrong_intent in (400, 403)
            expired_assertion, _ = login(assertion(expiry=-30))
            assert expired_assertion in (400, 403)
            # Inspect only public submission/provenance fixtures. No claim about an integrated TES store.
            public_records = {
                "tes_request": {
                    "executors": [{"image": "public-qc", "command": ["qc"]}],
                    "tags": {"study": "qc"},
                },
                "requirement_sidecar": {
                    "secret_ref": reference,
                    "execution_intent": intent,
                },
                "provenance": {
                    "capability": "workload-secret-requirement/1",
                    "secret_ref": reference,
                },
            }
            serialized = json.dumps(public_records)
            assert (
                token not in serialized
                and root_token not in serialized
                and fixture not in serialized
            )
            time.sleep(4)
            expired_token, _ = api(base, "secret/data/qc-input", token=token)
            assert expired_token == 403
            status, grant = login(assertion())
            assert status == 200
            revoked = grant["auth"]["client_token"]
            status, _ = api(base, "auth/token/revoke-self", {}, revoked)
            assert status == 204
            revoked_status, _ = api(base, "secret/data/qc-input", token=revoked)
            assert revoked_status == 403
            evidence = {
                "format_version": 1,
                "openbao_image": IMAGE,
                "secret_engine": "kv-v2 static fixture",
                "identity": "operator test RSA issuer, subject + audience + requester + intent + reference bound by JWT role",
                "authorized_workload": True,
                "different_workload_denied": other_status,
                "different_intent_denied": wrong_intent,
                "expired_assertion_denied": expired_assertion,
                "expired_access_token_denied": expired_token,
                "revoked_access_token_denied": revoked_status,
                "public_fixture_records_credential_free": True,
                "limitations": [
                    "No production runner attestation or requester authorization integration",
                    "Execution-intent digest is a fixed fixture, not a canonicalized TES request",
                    "Record inspection covers local fixtures, not deployed TES persistence",
                    "KV read revocation cannot revoke a value already disclosed; no dynamic-credential revocation claim",
                ],
            }
            if output:
                output.write_text(json.dumps(evidence, indent=2) + "\n")
            print(json.dumps(evidence, indent=2))
        finally:
            subprocess.run(
                docker + ["rm", "--force", name],
                check=False,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )


if __name__ == "__main__":
    main()
