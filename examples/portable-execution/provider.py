#!/usr/bin/env python3
"""Two cooperating loopback TES targets with optional admission/checkpoint APIs."""
import argparse
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import signal
import subprocess
import sys
import threading
import time
from urllib.parse import parse_qs, unquote, urlparse
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import (APP_FORMAT, INPUT_DIGEST, INPUT_ID, PUBLIC_INPUT, RUNTIME, TERMINAL,
                    Rejection, atomic_bytes, canonical, connect, encoded, fingerprint,
                    initialize, load_task, make_task, owned, policy, recipe, save_task,
                    update_task)


def iso(timestamp):
    return datetime.fromtimestamp(timestamp, timezone.utc).isoformat()


class Provider:
    def __init__(self, cluster, name, reconcile, port):
        self.cluster, self.root, self.name, self.reconcile = cluster, cluster.root, name, reconcile
        self.available = True
        self.processes = {}
        self.watchers = []
        self.process_lock = threading.Lock()
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_GET(self):
                self.handle_api()

            def do_POST(self):
                self.handle_api()

            def handle_api(self):
                try:
                    length = int(self.headers.get("Content-Length", "0"))
                    if length > 65536:
                        raise Rejection(413, "request too large")
                    body = json.loads(self.rfile.read(length)) if length else {}
                    principal = self.headers.get("X-H3-Principal", "demo")
                    parsed = urlparse(self.path)
                    path = unquote(parsed.path).replace("/ga4gh/tes/v1/", "/v1/")
                    result = outer.dispatch(self.command, path, parse_qs(parsed.query), body, principal)
                    status = 200
                except Rejection as error:
                    status, result = error.status, {"error": error.reason}
                except (ValueError, KeyError, TypeError) as error:
                    status, result = 422, {"error": "invalid request: " + str(error)}
                data = encoded(result)
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                try:
                    self.wfile.write(data)
                except (BrokenPipeError, ConnectionResetError):
                    pass  # Loss of client connection does not cancel accepted work.

        self.server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self.server.daemon_threads = True
        self.url = f"http://127.0.0.1:{self.server.server_port}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()

    def get_task(self, task_id, principal):
        with connect(self.root) as db:
            task = load_task(db, task_id)
        if task["provider"] != self.name:
            raise Rejection(404, "task belongs to another provider")
        if task["principal"] != principal:
            raise Rejection(403, "task access denied")
        return task

    def tes_task(self, task, view="FULL"):
        result = {"id": task["id"], "state": task["state"]}
        if view == "MINIMAL":
            return result
        result.update(task["spec"])
        result.update(id=task["id"], state=task["state"], creation_time=iso(task["created_at"]))
        logs = {"logs": [], "outputs": [], "metadata": {"execution_mode": "host CPU; no container isolation"},
                "start_time": iso(task["created_at"])}
        if task.get("ended_at"):
            logs["end_time"] = iso(task["ended_at"])
        if task.get("metrics"):
            logs["outputs"] = [{"path": "/outputs/metrics.json", "size_bytes": str(len(encoded(task["metrics"]))),
                                "url": self.url + f'/control/v1/tasks/{task["id"]}/metrics'}]
            logs["logs"] = [{"exit_code": 0}]
        if view == "FULL":
            logs["system_logs"] = [task["error"]] if task.get("error") else []
        result["logs"] = [logs]
        return result

    def candidates(self, spec, constraints):
        rec = recipe(spec)
        size = None if rec["input_id"] == "unknown-size" else len(PUBLIC_INPUT)
        location = None if rec["input_id"] == "unknown-location" else "alpha"
        trace = []
        for target in self.cluster.providers:
            reasons, uncertainty = [], []
            if not target.available:
                reasons.append("backend unavailable")
            if target.name not in constraints["allowed_execution_domains"]:
                reasons.append("execution domain forbidden")
            if target.name not in constraints["allowed_storage_domains"]:
                reasons.append("managed storage domain forbidden")
            if location is None:
                uncertainty.append("input location unknown")
            if size is None:
                uncertainty.append("input size unknown")
            if constraints["strict"] and uncertainty:
                reasons.extend(uncertainty)
            if constraints["no_export"] and (location is None or target.name != location):
                reasons.append("no-export forbids managed transfer")
            runtime_seconds = rec["steps"] / 100  # Explicitly synthetic, not a prediction model.
            compute = runtime_seconds * (.02 if target.name == "alpha" else .01)
            transfer_bytes = None if location is None or size is None else (0 if target.name == location else size)
            transfer = None if transfer_bytes is None else transfer_bytes * .00005
            storage = None if size is None else size * .0000001
            total = None if transfer is None or storage is None else compute + transfer + storage
            trace.append({"backend": target.name, "eligible": not reasons, "reasons": reasons,
                          "uncertainty": uncertainty, "input_location": location, "input_size_bytes": size,
                          "expected_transferred_bytes": transfer_bytes,
                          "estimate": {"compute": compute, "storage": storage, "transfer": transfer,
                                       "total": total, "currency": "synthetic-unit", "synthetic": True,
                                       "source": "fixed demonstrator rates v1", "source_timestamp": "2026-11-09T00:00:00Z",
                                       "runtime_seconds": runtime_seconds, "uncertainty": "illustrative rates and runtime; not invoice ceiling"}})
        return trace

    def constraints(self, db, requested=None):
        result = policy(db)
        # Client requests may tighten the active server policy, never relax it.
        requested = requested or {}
        result["no_export"] = result["no_export"] or requested.get("no_export", False)
        result["strict"] = result["strict"] or requested.get("strict", False)
        for key in ("allowed_execution_domains", "allowed_storage_domains"):
            if key in requested:
                result[key] = [x for x in result[key] if x in requested[key]]
        return result

    def plan(self, body, principal):
        spec = canonical(body["task"])
        with connect(self.root) as db:
            constraints = self.constraints(db, body.get("policy"))
            trace = self.candidates(spec, constraints)
            candidates = [x for x in trace if x["eligible"] and
                          (not body.get("backend") or x["backend"] == body["backend"])]
            if not candidates:
                raise Rejection(403, "no eligible backend: " + json.dumps(trace))
            selected = min(candidates, key=lambda x: x["estimate"]["total"] if x["estimate"]["total"] is not None else float("inf"))
            ttl = float(body.get("ttl_seconds", 60))
            if not .01 <= ttl <= 600:
                raise Rejection(422, "plan TTL must be in [0.01, 600] seconds")
            rec = recipe(spec)
            result = {"id": uuid.uuid4().hex, "schema": "h3-admission/v1", "principal": principal,
                      "task_fingerprint": fingerprint(spec), "input_digest": rec["input_digest"],
                      "inputs": [{"id": rec["input_id"], "sha256": rec["input_digest"],
                                  "logical_path": "/inputs/numbers.bin", "size_bytes": selected["input_size_bytes"],
                                  "location": selected["input_location"]}],
                      "executable": {"recipe": "public-cpu-v1", "sha256": rec["executable_digest"]},
                      "allowed_data_movements": [{"object": rec["input_id"], "source": selected["input_location"],
                                                  "destination": selected["backend"],
                                                  "expected_transferred_bytes": selected["expected_transferred_bytes"]}],
                      "executable_digest": rec["executable_digest"], "selected_backend": selected["backend"],
                      "policy_version": constraints["version"], "constraints": constraints,
                      "created_at": time.time(), "expires_at": time.time() + ttl, "decision_trace": trace,
                      "resource_profile": {"cpu_cores": 1, "ram_gb_max": 1},
                      "reservation": None, "quota": None,
                      "enforced": ["backend selection", "managed input staging", "managed output publication"],
                      "not_enforced": ["arbitrary application network traffic", "physical residency", "invoice ceiling"]}
            db.execute("INSERT INTO plans VALUES (?, ?)", (result["id"], json.dumps(result)))
        return result

    def validate_plan(self, db, plan_id, spec, principal):
        row = db.execute("SELECT data FROM plans WHERE id=?", (plan_id,)).fetchone()
        if not row:
            raise Rejection(404, "unknown plan")
        plan = json.loads(row[0])
        if plan["principal"] != principal:
            raise Rejection(403, "plan principal mismatch")
        if plan["expires_at"] <= time.time():
            raise Rejection(409, "plan expired; re-evaluation required")
        if plan["policy_version"] != policy(db)["version"]:
            raise Rejection(409, "policy changed; re-evaluation required")
        if plan["selected_backend"] != self.name:
            raise Rejection(409, "plan selects another backend")
        if plan["task_fingerprint"] != fingerprint(spec):
            raise Rejection(409, "task fingerprint mismatch")
        candidate = next(x for x in self.candidates(spec, plan["constraints"]) if x["backend"] == self.name)
        if not candidate["eligible"]:
            raise Rejection(403, "; ".join(candidate["reasons"]))
        return plan

    def stage(self, input_id, destination, principal, constraints=None, db=None):
        if input_id != INPUT_ID or destination not in {"alpha", "beta"}:
            raise Rejection(422, "unsupported managed object or destination")
        if db is None:
            with connect(self.root) as transaction:
                transaction.execute("BEGIN IMMEDIATE")
                return self.stage(input_id, destination, principal, constraints, db=transaction)
        current = policy(db)
        constraints = constraints or current
        if destination not in current["allowed_storage_domains"] or destination not in constraints["allowed_storage_domains"]:
            raise Rejection(403, "managed storage domain forbidden")
        if (current["no_export"] or constraints["no_export"]) and destination != "alpha":
            raise Rejection(403, "no-export forbids managed transfer")
        directory = self.root / "staging" / destination
        directory.mkdir(parents=True, exist_ok=True)
        atomic_bytes(directory / INPUT_ID, PUBLIC_INPUT)
        return {"input_id": input_id, "sha256": INPUT_DIGEST, "destination": destination,
                "observed_bytes_written": len(PUBLIC_INPUT), "observed_transferred_bytes": 0 if destination == "alpha" else len(PUBLIC_INPUT)}

    def new_task(self, db, spec, principal, rec, lineage=None, epoch=0, restore=None, receipt=None):
        task_id = uuid.uuid4().hex
        task = {"id": task_id, "provider": self.name, "principal": principal, "spec": spec,
                "fingerprint": fingerprint(spec), "recipe": rec, "state": "QUEUED", "pid": None,
                "physical_status": "PENDING", "created_at": time.time(), "lineage": lineage or task_id,
                "epoch": epoch, "restored_progress": restore["progress"] if restore else 0,
                "performed_steps": 0, "progress": restore["progress"] if restore else 0,
                "checkpoint_count": 0, "checkpoint_bytes_written": 0,
                "checkpoint_bytes_read": receipt["size_bytes"] if receipt else 0,
                "checkpoint_ttl": self.cluster.checkpoint_ttl, "restore": restore,
                "source_execution": receipt["source_execution"] if receipt else None,
                "result_commit_rejected": False, "metrics": None}
        db.execute("INSERT INTO tasks VALUES (?, ?, ?, ?)", (task_id, self.name, principal, json.dumps(task)))
        return task

    def existing_intent(self, db, intent, spec, principal):
        if not self.reconcile or not intent:
            return None
        row = db.execute("SELECT * FROM intents WHERE provider=? AND principal=? AND intent=?",
                         (self.name, principal, intent)).fetchone()
        if row:
            if row["fingerprint"] != fingerprint(spec):
                raise Rejection(409, "intent already bound to another task fingerprint")
            return row["task_id"]
        return None

    def register_intent(self, db, intent, spec, principal, task_id):
        if self.reconcile and intent:
            db.execute("INSERT INTO intents VALUES (?, ?, ?, ?, ?)",
                       (self.name, principal, intent, fingerprint(spec), task_id))

    def submit(self, spec, principal, plan_id=None):
        if not self.available:
            raise Rejection(503, "backend unavailable; no automatic fallback")
        spec, rec = canonical(spec), recipe(spec)
        intent = (spec.get("tags") or {}).get("h3.intent")
        # Stage before dispatch; validation and intent+ownership commit occur atomically below.
        with connect(self.root) as db:
            existing = self.existing_intent(db, intent, spec, principal)
            if existing:
                return {"id": existing}
            if plan_id:
                constraints = self.validate_plan(db, plan_id, spec, principal)["constraints"]
            else:
                constraints = self.constraints(db)
                candidate = next(x for x in self.candidates(spec, constraints) if x["backend"] == self.name)
                if not candidate["eligible"]:
                    raise Rejection(403, "; ".join(candidate["reasons"]))
        staged = self.stage(rec["input_id"], self.name, principal, constraints)
        with connect(self.root) as db:
            db.execute("BEGIN IMMEDIATE")
            existing = self.existing_intent(db, intent, spec, principal)
            if existing:
                return {"id": existing}
            if plan_id:
                self.validate_plan(db, plan_id, spec, principal)
            else:
                candidate = next(x for x in self.candidates(spec, self.constraints(db)) if x["backend"] == self.name)
                if not candidate["eligible"]:
                    raise Rejection(403, "; ".join(candidate["reasons"]))
            task = self.new_task(db, spec, principal, rec)
            task["staging_observed"] = staged
            save_task(db, task)
            db.execute("INSERT INTO ownership VALUES (?, ?, 0, NULL)", (task["lineage"], task["id"]))
            self.register_intent(db, intent, spec, principal, task["id"])
        self.launch(task)
        return {"id": task["id"]}

    def receipt(self, db, receipt_id, principal, rec):
        row = db.execute("SELECT data FROM receipts WHERE id=?", (receipt_id,)).fetchone()
        if not row:
            raise Rejection(404, "checkpoint not committed")
        receipt = json.loads(row[0])
        if receipt["principal"] != principal:
            raise Rejection(403, "checkpoint access denied")
        if receipt["expires_at"] <= time.time():
            raise Rejection(410, "checkpoint expired")
        if receipt["format"] != APP_FORMAT:
            raise Rejection(422, "incompatible checkpoint format")
        if receipt["runtime"] != RUNTIME:
            raise Rejection(422, "incompatible runtime")
        if receipt["input_digest"] != rec["input_digest"] or rec["input_id"] != INPUT_ID:
            raise Rejection(422, "changed input identity")
        if receipt["executable_digest"] != rec["executable_digest"]:
            raise Rejection(422, "changed executable")
        path = self.root / "checkpoints" / (receipt_id + ".json")
        if not path.exists():
            raise Rejection(422, "checkpoint artifact unavailable")
        raw = path.read_bytes()
        import hashlib
        if len(raw) != receipt["size_bytes"] or hashlib.sha256(raw).hexdigest() != receipt["sha256"]:
            raise Rejection(422, "checkpoint size/digest mismatch")
        try:
            state = json.loads(raw)
            if state["format"] != APP_FORMAT or state["runtime"] != RUNTIME:
                raise Rejection(422, "incompatible restored format/runtime")
            if state["input_digest"] != rec["input_digest"] or state["executable_digest"] != rec["executable_digest"]:
                raise Rejection(422, "restored identity mismatch")
            if state["parameter"] != rec["parameter"] or state["steps"] != rec["steps"]:
                raise Rejection(422, "changed application parameters")
            if not isinstance(state["progress"], int) or not 0 < state["progress"] <= rec["steps"]:
                raise ValueError("invalid progress")
            if not isinstance(state["rng_state"], int) or not 0 <= state["rng_state"] < 2 ** 32:
                raise ValueError("invalid RNG state")
            if len(bytes.fromhex(state["accumulator"])) != 32:
                raise ValueError("invalid accumulator")
        except (ValueError, TypeError, KeyError) as error:
            raise Rejection(422, "partial/invalid checkpoint state") from error
        return receipt, state

    def continue_task(self, body, principal):
        if body.get("mode", "application-checkpoint") != "application-checkpoint":
            raise Rejection(422, "unsupported checkpoint mode")
        if not self.cluster.fencing:
            raise Rejection(501, "exclusive result-writer fencing unsupported")
        if not self.available:
            raise Rejection(503, "backend unavailable; no automatic fallback")
        spec, rec = canonical(body["task"]), recipe(body["task"])
        intent = body.get("intent")
        with connect(self.root) as db:
            db.execute("BEGIN IMMEDIATE")
            existing = self.existing_intent(db, intent, spec, principal)
            if existing:
                return {"id": existing}
            plan = self.validate_plan(db, body["plan_id"], spec, principal)
            receipt, restored = self.receipt(db, body["receipt_id"], principal, rec)
            owner = db.execute("SELECT * FROM ownership WHERE lineage=?", (receipt["lineage"],)).fetchone()
            if owner["writer"] != receipt["source_execution"] or owner["epoch"] != receipt["ownership_epoch"] or owner["result"]:
                raise Rejection(409, "checkpoint source no longer owns an unfinished lineage")
            self.stage(rec["input_id"], self.name, principal, plan["constraints"], db=db)
            task = self.new_task(db, spec, principal, rec, receipt["lineage"], owner["epoch"] + 1, restored, receipt)
            source = load_task(db, receipt["source_execution"])
            task["source_progress_at_transfer"] = source["progress"]
            task["repeated_steps_at_transfer"] = max(0, source["progress"] - restored["progress"])
            save_task(db, task)
            db.execute("UPDATE ownership SET writer=?, epoch=? WHERE lineage=? AND writer=? AND epoch=?",
                       (task["id"], task["epoch"], task["lineage"], receipt["source_execution"], owner["epoch"]))
            self.register_intent(db, intent, spec, principal, task["id"])
        self.launch(task)
        return {"id": task["id"]}

    def launch(self, task):
        update_task(self.root, task["id"], state="RUNNING")
        try:
            process = subprocess.Popen([sys.executable, str(Path(__file__).with_name("program.py")), str(self.root), task["id"]],
                                       stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        except OSError as error:
            update_task(self.root, task["id"], state="SYSTEM_ERROR", physical_status="NOT_STARTED", error=str(error))
            return
        with self.process_lock:
            self.processes[task["id"]] = process
        update_task(self.root, task["id"], pid=process.pid, physical_status="RUNNING")

        def watch():
            returncode = process.wait()
            with connect(self.root) as db:
                db.execute("BEGIN IMMEDIATE")
                current = load_task(db, task["id"])
                current.update(physical_status="EXITED", returncode=returncode)
                if current["state"] == "CANCELING":
                    current.update(state="CANCELED", ended_at=time.time())
                elif current["state"] not in TERMINAL:
                    current.update(state="SYSTEM_ERROR", error="CPU worker exited before terminal commit", ended_at=time.time())
                save_task(db, current)
        watcher = threading.Thread(target=watch, daemon=True)
        self.watchers.append(watcher)
        watcher.start()

    def stop(self, task_id, principal, preempt=False):
        task = self.get_task(task_id, principal)
        if task["state"] in TERMINAL:
            return {}
        with self.process_lock:
            process = self.processes.get(task_id)
        if not process:
            raise Rejection(409, "process status unknown; cancellation unresolved")
        update_task(self.root, task_id, state="PREEMPTED" if preempt else "CANCELING", preemption_requested=preempt)
        process.terminate()
        return {}

    def evidence(self, task):
        with connect(self.root) as db:
            writer = bool(owned(db, task))
        return {key: value for key, value in task.items() if key not in {"restore", "spec", "recipe", "metrics"}} | {
            "authorized_result_writer": writer, "physical_cleanup_guarantee": "observed child exit only",
            "execution_mode": "host CPU subprocess; co-hosted cooperating endpoints",
            "exactly_once_physical_execution": False}

    def dispatch(self, method, path, query, body, principal):
        if method == "GET" and path in {"/v1/service-info", "/service-info"}:
            return {"id": "h3-" + self.name, "name": "H3 loopback CPU " + self.name,
                    "type": {"group": "org.ga4gh", "artifact": "tes", "version": "1.1.0"},
                    "version": "0.1.0", "description": "Loopback host CPU demonstrator; no container isolation",
                    "organization": {"name": "local demonstrator", "url": "https://ga4gh.org"},
                    "storage": ["h3://managed/" + self.name]}
        if path == "/v1/tasks":
            if method == "POST":
                return self.submit(body, principal)
            with connect(self.root) as db:
                tasks = [json.loads(row[0]) for row in db.execute("SELECT data FROM tasks WHERE provider=? AND principal=? ORDER BY rowid", (self.name, principal))]
            keys, values = query.get("tag_key", []), query.get("tag_value", [])
            tasks = [t for t in tasks if t["spec"].get("name", "").startswith(query.get("name_prefix", [""])[0]) and
                     ("state" not in query or t["state"] == query["state"][0]) and
                     all(k in (t["spec"].get("tags") or {}) and (i >= len(values) or values[i] == "" or t["spec"]["tags"][k] == values[i]) for i, k in enumerate(keys))]
            offset, size = int(query.get("page_token", [0])[0]), int(query.get("page_size", [256])[0])
            if offset < 0 or not 1 <= size <= 2047:
                raise Rejection(422, "invalid pagination")
            result = {"tasks": [self.tes_task(t, query.get("view", ["MINIMAL"])[0]) for t in tasks[offset:offset + size]]}
            if offset + size < len(tasks):
                result["next_page_token"] = str(offset + size)
            return result
        if path.startswith("/v1/tasks/"):
            suffix = path[len("/v1/tasks/"):]
            if method == "POST" and suffix.endswith(":cancel"):
                return self.stop(suffix[:-7], principal)
            if method == "GET":
                return self.tes_task(self.get_task(suffix, principal), query.get("view", ["FULL"])[0])
        if path == "/control/v1/capabilities" and method == "GET":
            return {"schema": "h3-control/v1", "provider": self.name, "application_checkpoints": True,
                    "process_snapshots": False, "engine_resume": False, "result_fencing": self.cluster.fencing,
                    "intent_reconciliation": self.reconcile, "managed_staging_enforcement": True,
                    "arbitrary_traffic_no_export": False, "physical_residency": False,
                    "execution_mode": "host CPU; not container execution", "runtime": RUNTIME}
        if path.startswith("/control/v1/intents/") and method == "GET":
            if not self.reconcile:
                raise Rejection(501, "provider-backed intent reconciliation unsupported")
            intent = path[len("/control/v1/intents/"):]
            with connect(self.root) as db:
                row = db.execute("SELECT * FROM intents WHERE provider=? AND principal=? AND intent=?", (self.name, principal, intent)).fetchone()
            if row is None:
                raise Rejection(404, "no durable intent binding; absence is not physical non-execution proof")
            return {"provider": row["provider"], "principal": principal, "intent": row["intent"],
                    "task_fingerprint": row["fingerprint"], "task_id": row["task_id"],
                    "evidence": "durable unique provider intent binding"}
        if path == "/control/v1/plans" and method == "POST":
            return self.plan(body, principal)
        if path == "/control/v1/submit" and method == "POST":
            return self.submit(body["task"], principal, body["plan_id"])
        if path == "/control/v1/continue" and method == "POST":
            return self.continue_task(body, principal)
        if path == "/control/v1/stage" and method == "POST":
            constraints = None
            if body.get("plan_id"):
                with connect(self.root) as db:
                    row = db.execute("SELECT data FROM plans WHERE id=?", (body["plan_id"],)).fetchone()
                    if not row:
                        raise Rejection(404, "unknown plan")
                    plan = json.loads(row[0])
                    if plan["principal"] != principal:
                        raise Rejection(403, "plan access denied")
                    if plan["expires_at"] <= time.time() or plan["policy_version"] != policy(db)["version"]:
                        raise Rejection(409, "expired or changed plan")
                    if body["destination"] != plan["selected_backend"]:
                        raise Rejection(403, "staging destination differs from approved backend")
                    constraints = plan["constraints"]
            return self.stage(body["input_id"], body["destination"], principal, constraints)
        if path == "/control/v1/policy" and method == "POST":
            if principal != "demo":
                raise Rejection(403, "test operator only")
            with connect(self.root) as db:
                db.execute("BEGIN IMMEDIATE")
                active = policy(db)
                active.update({k: v for k, v in body.items() if k in {"no_export", "strict", "allowed_execution_domains", "allowed_storage_domains"}})
                active["version"] = "policy-" + uuid.uuid4().hex
                db.execute("UPDATE config SET data=? WHERE key='policy'", (json.dumps(active),))
            return active
        if path == "/control/v1/availability" and method == "POST":
            if principal != "demo":
                raise Rejection(403, "test operator only")
            self.available = bool(body["available"])
            return {"available": self.available}
        if path.startswith("/control/v1/tasks/"):
            parts = path.split("/")
            task = self.get_task(parts[4], principal)
            action = parts[5] if len(parts) == 6 else ""
            if method == "GET" and action == "evidence":
                return self.evidence(task)
            if method == "GET" and action == "metrics":
                if task["state"] != "COMPLETE" or not task.get("metrics"):
                    raise Rejection(409, "metrics unavailable; task not successfully complete")
                return task["metrics"]
            if method == "GET" and action == "checkpoints":
                with connect(self.root) as db:
                    receipts = [json.loads(row[0]) for row in db.execute("SELECT data FROM receipts WHERE task_id=? AND principal=? ORDER BY rowid", (task["id"], principal))]
                return {"receipts": [r for r in receipts if r["expires_at"] > time.time()]}
            if method == "POST" and action == "preempt":
                return self.stop(task["id"], principal, preempt=True)
        raise Rejection(404, "unknown endpoint")

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=3)
        with self.process_lock:
            processes = list(self.processes.values())
        for process in processes:
            if process.poll() is None:
                process.terminate()
        for process in processes:
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        for watcher in self.watchers:
            watcher.join(timeout=5)


class Cluster:
    def __init__(self, root, ports=(0, 0), reconcile=(True, False), fencing=True, checkpoint_ttl=600):
        self.root = initialize(root)
        self.fencing, self.checkpoint_ttl = fencing, checkpoint_ttl
        self.providers = [Provider(self, name, proof, port) for name, proof, port in zip(("alpha", "beta"), reconcile, ports)]
        # On supervisor restart, stale process rows are observational uncertainty, never re-launch requests.
        with connect(self.root) as db:
            db.execute("BEGIN IMMEDIATE")
            for row in db.execute("SELECT data FROM tasks").fetchall():
                task = json.loads(row[0])
                if task["physical_status"] in {"RUNNING", "PENDING"}:
                    task["physical_status"] = "UNKNOWN_AFTER_RESTART"
                    save_task(db, task)

    def start(self):
        for target in self.providers:
            target.start()
        return self

    def close(self):
        for target in self.providers:
            target.close()

    def __enter__(self):
        return self.start()

    def __exit__(self, *args):
        self.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["serve"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--alpha-port", type=int, default=0)
    parser.add_argument("--beta-port", type=int, default=0)
    arguments = parser.parse_args()
    done = threading.Event()
    for sig in (signal.SIGINT, signal.SIGTERM):
        signal.signal(sig, lambda *_: done.set())
    with Cluster(arguments.root, ports=(arguments.alpha_port, arguments.beta_port)) as cluster:
        print(json.dumps({"execution_mode": "loopback host CPU", "providers": [{"name": p.name, "url": p.url} for p in cluster.providers]}), flush=True)
        done.wait()


if __name__ == "__main__":
    main()
