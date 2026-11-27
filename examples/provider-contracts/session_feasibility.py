#!/usr/bin/env python3
"""Reference lease provider + authenticated HTTP proxy + real report process.
The subprocess controller is a feasibility stand-in for a TES runner adapter.
"""
import argparse, signal, datetime, http.server, json, os, pathlib, secrets, socket, subprocess, sys, tempfile, threading, time, urllib.error, urllib.request
from urllib.parse import urlparse

if not __debug__:
    raise SystemExit("Run the feasibility checks without Python optimization (-O)")


def timestamp(value):
    return datetime.datetime.fromtimestamp(value, datetime.timezone.utc).isoformat()


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def dashboard(port, report):
    time.sleep(
        0.6
    )  # Process RUNNING precedes application readiness; HTTP probing establishes readiness.

    class Dashboard(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            body = pathlib.Path(report).read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    http.server.HTTPServer(("127.0.0.1", port), Dashboard).serve_forever()


class Provider:
    def __init__(self, report, owners, executions):
        self.report = report
        self.owners = owners
        self.executions = executions
        self.sessions = {}
        self.lock = threading.RLock()
        threading.Thread(target=self.reaper, daemon=True).start()

    def view(self, s):
        return {k: v for k, v in s.items() if k not in ("process", "app_port")}

    def cleanup(self, s, reason):
        s["route_enabled"] = False
        s["cleanup_state"] = (
            "route_invalidated"  # Invalidate access before compute shutdown.
        )
        try:
            s["process"].terminate()
            try:
                s["process"].wait(timeout=2)
            except subprocess.TimeoutExpired:
                s["process"].kill()
                s["process"].wait(timeout=2)
        except (OSError, subprocess.TimeoutExpired):
            s["cleanup_state"] = "compute_cleanup_failed"
            return
        s["state"] = reason
        s["cleanup_state"] = "complete"

    def reaper(self):
        while True:
            with self.lock:
                for s in self.sessions.values():
                    if s["state"] in ("stopped", "expired"):
                        continue
                    if time.time() >= s["expires_epoch"]:
                        self.cleanup(s, "expired")
                        continue
                    if s["state"] == "provisioning":
                        try:
                            with urllib.request.urlopen(
                                f"http://127.0.0.1:{s['app_port']}/", timeout=0.1
                            ) as response:
                                if response.status == 200:
                                    s["state"] = "ready"
                                    s["route_enabled"] = True
                        except OSError:
                            pass
            time.sleep(0.05)

    def create(self, owner, execution, ttl):
        if self.executions.get(execution) != owner:
            raise PermissionError()
        # Reconnecting uses inspect; repeated creation for a live execution returns its session.
        for s in self.sessions.values():
            if (
                s["owner"] == owner
                and s["owning_execution"] == execution
                and s["state"] not in ("expired", "stopped")
            ):
                return self.view(s)
        identifier = secrets.token_hex(16)
        port = free_port()
        now = time.time()
        proc = subprocess.Popen(
            [
                sys.executable,
                __file__,
                "--dashboard",
                str(port),
                "--report",
                self.report,
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        s = {
            "version": "leased-session/1",
            "session_id": identifier,
            "owner": owner,
            "owning_execution": execution,
            "state": "provisioning",
            "task_state": "RUNNING",
            "expires_epoch": now + ttl,
            "expires_at": timestamp(now + ttl),
            "endpoint_reference": f"/v1/sessions/{identifier}/view",
            "route_enabled": False,
            "cleanup_state": "not_started",
            "process": proc,
            "app_port": port,
        }
        self.sessions[identifier] = s
        return self.view(s)


def serve(port, report, config):
    credentials = json.loads(pathlib.Path(config).read_text())
    provider = Provider(report, credentials["owners"], credentials["executions"])

    class Proxy(http.server.BaseHTTPRequestHandler):
        def reply(self, status, data):
            body = json.dumps(data).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

        def do_POST(self):
            self.handle_request()

        def do_GET(self):
            self.handle_request()

        def handle_request(self):
            token = self.headers.get("Authorization", "").removeprefix("Bearer ")
            owner = provider.owners.get(token)
            if owner is None:
                return self.reply(401, {"error": "unauthorized"})
            path = urlparse(self.path).path
            with provider.lock:
                if path == "/v1/sessions" and self.command == "POST":
                    body = json.loads(
                        self.rfile.read(
                            min(int(self.headers.get("Content-Length", "0")), 8192)
                        )
                    )
                    ttl = body.get("ttl", 2)
                    if not isinstance(ttl, (int, float)) or not 1 <= ttl <= 10:
                        return self.reply(
                            400,
                            {
                                "error": "TTL must be between 1 and 10 seconds in this prototype"
                            },
                        )
                    try:
                        return self.reply(
                            201, provider.create(owner, body["owning_execution"], ttl)
                        )
                    except PermissionError:
                        return self.reply(403, {"error": "forbidden"})
                parts = path.strip("/").split("/")
                if parts[:2] != ["v1", "sessions"] or len(parts) not in (3, 4):
                    return self.reply(404, {"error": "not_found"})
                s = provider.sessions.get(parts[2])
                if s is None or s["owner"] != owner:
                    return self.reply(403, {"error": "forbidden"})
                if time.time() >= s["expires_epoch"] and s["state"] not in (
                    "expired",
                    "stopped",
                ):
                    provider.cleanup(s, "expired")
                if len(parts) == 3:
                    if self.command != "GET":
                        return self.reply(405, {"error": "method_not_allowed"})
                    return self.reply(200, provider.view(s))
                operation = parts[3]
                if operation == "renew":
                    if self.command != "POST":
                        return self.reply(405, {"error": "method_not_allowed"})
                    if s["state"] in ("stopped", "expired"):
                        return self.reply(409, {"error": "lease_not_active"})
                    s["expires_epoch"] = time.time() + 2
                    s["expires_at"] = timestamp(s["expires_epoch"])
                    return self.reply(200, provider.view(s))
                if operation == "stop":
                    if self.command != "POST":
                        return self.reply(405, {"error": "method_not_allowed"})
                    provider.cleanup(s, "stopped")
                    return self.reply(200, provider.view(s))
                if operation == "view":
                    if self.command != "GET":
                        return self.reply(405, {"error": "method_not_allowed"})
                    if not s["route_enabled"]:
                        return self.reply(
                            410 if s["state"] in ("expired", "stopped") else 503,
                            {"error": "route_unavailable"},
                        )
                    try:
                        with urllib.request.urlopen(
                            f"http://127.0.0.1:{s['app_port']}/", timeout=1
                        ) as upstream:
                            body = upstream.read(1024 * 1024)
                        self.send_response(200)
                        self.send_header("Content-Type", "text/html")
                        self.send_header("Content-Length", str(len(body)))
                        self.end_headers()
                        return self.wfile.write(body)
                    except OSError:
                        return self.reply(503, {"error": "application_unavailable"})
                return self.reply(404, {"error": "not_found"})

    def shutdown(*_):
        with provider.lock:
            for session in provider.sessions.values():
                if session["state"] not in ("expired", "stopped"):
                    provider.cleanup(session, "stopped")
        raise SystemExit(0)

    signal.signal(signal.SIGTERM, shutdown)
    try:
        http.server.ThreadingHTTPServer(("127.0.0.1", port), Proxy).serve_forever()
    finally:
        shutdown()


def request(base, path, token, data=None):
    r = urllib.request.Request(
        base + path,
        headers={
            "Authorization": "Bearer " + token,
            "Content-Type": "application/json",
        },
        data=None if data is None else json.dumps(data).encode(),
    )
    try:
        with urllib.request.urlopen(r, timeout=5) as response:
            body = response.read()
            return response.status, (
                json.loads(body)
                if response.headers.get("Content-Type") == "application/json"
                else body
            )
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read())


def test(output=None):
    with tempfile.TemporaryDirectory(prefix="ga4gh-sessions-") as directory:
        directory = pathlib.Path(directory)
        config = directory / "provider.json"
        token = secrets.token_urlsafe(24)
        other = secrets.token_urlsafe(24)
        config.write_text(
            json.dumps(
                {
                    "owners": {token: "owner-a", other: "owner-b"},
                    "executions": {"tes-fixture-a": "owner-a"},
                }
            )
        )
        config.chmod(0o600)
        report = directory / "report.html"
        report.write_text(
            "<!doctype html><title>Sequence QC</title><h1>Sequence QC</h1><p>Public synthetic sequences: 3</p>"
        )
        port = free_port()
        provider = subprocess.Popen(
            [
                sys.executable,
                __file__,
                "--serve",
                str(port),
                "--report",
                str(report),
                "--config",
                str(config),
            ],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        base = "http://127.0.0.1:" + str(port)
        try:
            for _ in range(100):
                try:
                    request(base, "/v1/sessions/unknown", token)
                    break
                except OSError:
                    time.sleep(0.05)
            status, session = request(
                base,
                "/v1/sessions",
                token,
                {"owning_execution": "tes-fixture-a", "ttl": 2},
            )
            assert status == 201
            assert (
                session["task_state"] == "RUNNING"
                and session["state"] == "provisioning"
            )
            identifier = session["session_id"]
            path = "/v1/sessions/" + identifier
            assert request(base, "/other/path/" + identifier, token)[0] == 404
            assert request(base, path + "/unknown", token)[0] == 404
            assert request(base, path + "/view", token, {})[0] == 405
            assert request(base, path + "/renew", token)[0] == 405
            assert request(base, path, other)[0] == 403
            assert request(base, path + "/view", other)[0] == 403
            assert request(base, path + "/view", token)[0] == 503
            for _ in range(100):
                status, view = request(base, path + "/view", token)
                if status == 200:
                    break
                time.sleep(0.05)
            assert status == 200 and b"Sequence QC" in view
            # A separate client exits; the provider's lease and compute remain alive.
            code = 'import json,urllib.request,sys;p=json.load(sys.stdin);r=urllib.request.Request(p["url"],headers={"Authorization":"Bearer "+p["token"]});print(json.load(urllib.request.urlopen(r))["session_id"])'
            client = subprocess.run(
                [sys.executable, "-c", code],
                input=json.dumps({"url": base + path, "token": token}),
                text=True,
                capture_output=True,
                check=True,
            )
            assert client.stdout.strip() == identifier
            assert request(base, path + "/renew", token, {})[0] == 200
            duplicate = request(
                base,
                "/v1/sessions",
                token,
                {"owning_execution": "tes-fixture-a", "ttl": 2},
            )[1]
            assert duplicate["session_id"] == identifier
            handle = {
                "version": "leased-session/1",
                "provider": "local-reference",
                "session_id": identifier,
                "owning_execution": "tes-fixture-a",
            }
            assert token not in json.dumps(handle)
            time.sleep(2.3)
            status, state = request(base, path, token)
            assert state["state"] == "expired" and state["cleanup_state"] == "complete"
            assert not state["route_enabled"]
            assert request(base, path + "/view", token)[0] == 410
            assert request(base, path + "/renew", token, {})[0] == 409
            _, second = request(
                base,
                "/v1/sessions",
                token,
                {"owning_execution": "tes-fixture-a", "ttl": 2},
            )
            _, stopped = request(
                base, "/v1/sessions/" + second["session_id"] + "/stop", token, {}
            )
            assert stopped["state"] == "stopped" and not stopped["route_enabled"]
            evidence = {
                "format_version": 1,
                "task_running_before_readiness": True,
                "owner_checks": True,
                "authenticated_proxy_delivered_report": True,
                "reconnect_without_second_process": True,
                "renew": True,
                "expiry_after_client_exit": True,
                "route_invalidated_before_compute_stop": True,
                "explicit_stop": True,
                "credential_free_handle": handle,
                "limitations": [
                    "Local subprocess controller, not an integrated TES backend",
                    "Static test bearer auth, not production identity/proxy federation",
                    "Capability discovery, lease generation CAS and durable idempotency are not implemented by the prototype",
                    "Provider restart recovery and termination-failure retry require the backend implementation",
                ],
            }
            if output:
                pathlib.Path(output).write_text(json.dumps(evidence, indent=2) + "\n")
            print(json.dumps(evidence, indent=2))
        finally:
            provider.terminate()
            provider.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--serve", type=int)
    parser.add_argument("--dashboard", type=int)
    parser.add_argument("--report")
    parser.add_argument("--config")
    parser.add_argument("--evidence")
    args = parser.parse_args()
    if args.dashboard:
        dashboard(args.dashboard, args.report)
    elif args.serve:
        serve(args.serve, args.report, args.config)
    else:
        test(args.evidence)
