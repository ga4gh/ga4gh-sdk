#!/usr/bin/env python3
"""Loopback TES adapter for one allowlisted recipe; executes real CPU containers.

This is a local demonstrator, not a general TES implementation or security boundary
for untrusted users. It never fetches submitted URLs or runs submitted commands.
"""
import argparse
import hashlib
import http.server
import json
import os
import pathlib
import shutil
import subprocess
import sys
import uuid
import threading
import urllib.parse

IMAGE = "python@sha256:6771159cd4fa5d9bba1258caf0b82e6b73458c694d178ad97c5e925c2d0e1a91"
HERE = pathlib.Path(__file__).resolve().parent
COMMAND = ["python", "/workspace/preprocess.py", "/workspace/inputs/reference.fa", "/workspace/results/kmers.json", "3"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--workspace", type=pathlib.Path, required=True)
    parser.add_argument("--port-file", type=pathlib.Path, required=True)
    parser.add_argument("--name", required=True)
    args = parser.parse_args()
    args.workspace.mkdir(parents=True, exist_ok=True)
    state = {"accepted_tasks": 0, "container_executions": 0, "container_launch_attempts": 0, "mode": "available", "artifact_payload_bytes_sent": 0}
    lock = threading.Lock()
    task = None
    output = args.workspace / "results/kmers.json"
    original_bytes = None

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def send(self, status, body, content_type="application/json"):
            body = body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            path = urllib.parse.urlsplit(self.path).path
            if path == "/ga4gh/tes/v1/service-info":
                return self.send(200, {"id": args.name, "name": args.name, "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"}, "organization": {"name": "local synthetic demonstration", "url": "https://example.org"}, "version": "result-reuse-demo-v1"})
            if path == "/control":
                return self.send(200, state)
            if path == "/manifest" and task is not None:
                return self.send(200, {"sha256": hashlib.sha256(original_bytes).hexdigest(), "size_bytes": len(original_bytes), "runtime": json.loads((args.workspace / "runtime.json").read_text()), "image": IMAGE, "executable_sha256": hashlib.sha256((HERE / "preprocess.py").read_bytes()).hexdigest()})
            if path == "/ga4gh/tes/v1/tasks/local-1" and task is not None:
                return self.send(200, task)
            if path == "/ga4gh/tes/v1/tasks":
                return self.send(200, {"tasks": [] if task is None else [task]})
            if path == "/artifacts/kmers.json":
                if state["mode"] == "revoked":
                    return self.send(403, {"error": "access revoked"})
                if state["mode"] == "missing" or not output.exists():
                    return self.send(404, {"error": "artifact missing"})
                data = output.read_bytes()
                state["artifact_payload_bytes_sent"] += len(data)
                return self.send(200, data)
            self.send(404, {"error": "not found"})

        def do_POST(self):
            nonlocal task, original_bytes
            size = int(self.headers.get("Content-Length", "0"))
            if not 0 < size <= 65536:
                return self.send(400, {"error": "bounded JSON body required"})
            try:
                request = json.loads(self.rfile.read(size))
            except ValueError:
                return self.send(400, {"error": "invalid JSON"})
            if self.path == "/control":
                mode = request.get("mode")
                if mode not in ("available", "revoked", "corrupt", "missing"):
                    return self.send(400, {"error": "unknown mode"})
                state["mode"] = mode
                if original_bytes is not None:
                    if mode == "missing":
                        output.unlink(missing_ok=True)
                    elif mode == "corrupt":
                        output.write_bytes(b"X" + original_bytes[1:])
                    else:
                        output.write_bytes(original_bytes)
                return self.send(200, state)
            if self.path != "/ga4gh/tes/v1/tasks":
                return self.send(404, {"error": "not found"})
            executors = request.get("executors") or []
            if len(executors) != 1 or executors[0].get("image") != IMAGE or executors[0].get("command") != COMMAND or executors[0].get("env") != {"LC_ALL": "C", "PYTHONHASHSEED": "0"}:
                return self.send(400, {"error": "only the pinned deterministic recipe is supported"})
            with lock:
                if task is not None:
                    return self.send(409, {"error": "one bounded execution per local provider"})
                state["accepted_tasks"] += 1
                (args.workspace / "inputs").mkdir(exist_ok=True)
                (args.workspace / "results").mkdir(exist_ok=True)
                shutil.copyfile(HERE / "preprocess.py", args.workspace / "preprocess.py")
                shutil.copyfile(HERE / "reference.fa", args.workspace / "inputs/reference.fa")
                env = os.environ.copy()
                env.pop("DOCKER_HOST", None)
                context = env.get("GA4GH_REUSE_DOCKER_CONTEXT", "colima-ga4gh-phase2" if sys.platform == "darwin" else "default")
                container_name = f"ga4gh-result-reuse-{uuid.uuid4().hex}"
                invocation = ["docker", "--context", context, "run", "--name", container_name, "--pull=never", "--rm", "--network=none", "--read-only", "--cap-drop=ALL", "--security-opt=no-new-privileges", "--pids-limit=64", "--cpus=1", "--memory=128m", "--mount", f"type=bind,src={args.workspace},dst=/workspace", "--workdir", "/workspace", "--env", "LC_ALL=C", "--env", "PYTHONHASHSEED=0", IMAGE] + COMMAND
                state["container_launch_attempts"] += 1
                try:
                    completed = subprocess.run(invocation, env=env, capture_output=True, timeout=45)
                except subprocess.TimeoutExpired:
                    # Only this adapter's uniquely named container may be removed.
                    subprocess.run(["docker", "--context", context, "rm", "--force", container_name], env=env, capture_output=True, timeout=10)
                    task = {"id": "local-1", "state": "EXECUTOR_ERROR", "executors": executors}
                    return self.send(500, {"error": "bounded container execution timed out"})
                (args.workspace / "container.stderr").write_bytes(completed.stderr)
                if completed.returncode:
                    task = {"id": "local-1", "state": "EXECUTOR_ERROR", "executors": executors}
                    return self.send(500, {"error": "container failed; inspect provider workspace container.stderr"})
                state["container_executions"] += 1
                original_bytes = output.read_bytes()
                task = dict(request, id="local-1", state="COMPLETE")
                return self.send(200, {"id": "local-1"})

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    args.port_file.write_text(str(server.server_port))
    server.serve_forever()


if __name__ == "__main__":
    main()
