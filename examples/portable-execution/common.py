"""Durable coordinator and fixed public recipe; no third-party dependencies."""
from contextlib import contextmanager
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import time
import uuid

APP_FORMAT = "h3-checkpoint/v1"
RUNTIME = "python3-integer-sha256-v1"
IMAGE = "ga4gh-h3-cpu:v1"  # Recipe label only: execution is host CPU, not Docker.
INPUT_ID = "public-numbers-v1"
PUBLIC_INPUT = b"".join(i.to_bytes(4, "big") for i in range(256))
INPUT_DIGEST = hashlib.sha256(PUBLIC_INPUT).hexdigest()
TERMINAL = {"COMPLETE", "CANCELED", "PREEMPTED", "SYSTEM_ERROR", "EXECUTOR_ERROR"}
DEFAULT_POLICY = {"version": "policy-v1", "no_export": False, "strict": True,
                  "allowed_execution_domains": ["alpha", "beta"],
                  "allowed_storage_domains": ["alpha", "beta"]}


def canonical(value):
    if isinstance(value, dict):
        return {k: canonical(v) for k, v in value.items() if v is not None}
    if isinstance(value, list):
        return [canonical(v) for v in value]
    return value


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


def fingerprint(task):
    return hashlib.sha256(encoded(canonical({k: v for k, v in task.items()
                                            if k not in {"id", "state", "logs", "creation_time"}}))).hexdigest()


def executable_digest():
    return hashlib.sha256(Path(__file__).with_name("program.py").read_bytes()).hexdigest()


def make_task(parameter=2, steps=80, delay_ms=10, intent=None, input_id=INPUT_ID):
    task = {"name": "public-cpu-trial", "executors": [{"image": IMAGE,
            "command": ["h3-cpu", "--steps", str(steps), "--parameter", str(parameter),
                        "--delay-ms", str(delay_ms)]}],
            "inputs": [{"name": input_id, "url": "h3://inputs/" + input_id,
                        "path": "/inputs/numbers.bin", "type": "FILE"}],
            "resources": {"cpu_cores": 1, "ram_gb": .1}}
    if intent is not None:
        task["tags"] = {"h3.intent": intent}
    return task


class Rejection(Exception):
    def __init__(self, status, reason):
        self.status, self.reason = status, reason
        super().__init__(reason)


def recipe(task):
    """Parse one allowlisted recipe, never exec an arbitrary request command."""
    try:
        ex = task["executors"]
        if len(ex) != 1 or ex[0]["image"] != IMAGE:
            raise ValueError("unsupported executable")
        command = ex[0]["command"]
        if len(command) != 7 or command[0] != "h3-cpu" or command[1::2] != ["--steps", "--parameter", "--delay-ms"]:
            raise ValueError("unsupported command")
        steps, parameter, delay = map(int, [command[2], command[4], command[6]])
        if not 10 <= steps <= 1000 or not -100 <= parameter <= 100 or not 0 <= delay <= 100:
            raise ValueError("recipe bounds exceeded")
        inputs = task.get("inputs") or make_task()["inputs"]
        if len(inputs) != 1 or inputs[0].get("path") != "/inputs/numbers.bin" or inputs[0].get("contents") is not None:
            raise ValueError("unsupported managed input")
        input_id = inputs[0]["url"].removeprefix("h3://inputs/")
        if inputs[0]["url"] != "h3://inputs/" + input_id or input_id not in {INPUT_ID, "unknown-size", "unknown-location"}:
            raise ValueError("unknown managed input")
        if (ex[0].get("env") or task.get("volumes") or task.get("outputs") or
                any(ex[0].get(key) for key in ("workdir", "stdin", "stdout", "stderr", "ignore_error"))):
            raise ValueError("arbitrary environment, paths, volumes and output URLs are unsupported")
        resource = task.get("resources") or {}
        if resource.get("cpu_cores", 1) != 1 or resource.get("ram_gb", .1) > 1:
            raise ValueError("unsupported resource profile")
        return {"steps": steps, "parameter": parameter, "delay_ms": delay,
                "input_id": input_id, "input_digest": INPUT_DIGEST,
                "executable_digest": executable_digest(), "runtime": RUNTIME}
    except (KeyError, TypeError, ValueError, IndexError) as error:
        raise Rejection(422, str(error)) from error


@contextmanager
def connect(root):
    connection = sqlite3.connect(str(Path(root) / "coordinator.sqlite"), timeout=30)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA busy_timeout=30000")
    connection.execute("PRAGMA synchronous=FULL")
    try:
        with connection:
            yield connection
    finally:
        connection.close()


def initialize(root):
    root = Path(root)
    root.mkdir(parents=True, exist_ok=True)
    (root / "checkpoints").mkdir(exist_ok=True)
    (root / "results").mkdir(exist_ok=True)
    with connect(root) as db:
        db.execute("PRAGMA journal_mode=WAL")
        db.executescript("""
        CREATE TABLE IF NOT EXISTS tasks(id TEXT PRIMARY KEY, provider TEXT, principal TEXT, data TEXT);
        CREATE TABLE IF NOT EXISTS ownership(lineage TEXT PRIMARY KEY, writer TEXT, epoch INTEGER, result TEXT);
        CREATE TABLE IF NOT EXISTS receipts(id TEXT PRIMARY KEY, task_id TEXT, principal TEXT, data TEXT);
        CREATE TABLE IF NOT EXISTS intents(provider TEXT, principal TEXT, intent TEXT, fingerprint TEXT, task_id TEXT,
          PRIMARY KEY(provider, principal, intent));
        CREATE TABLE IF NOT EXISTS plans(id TEXT PRIMARY KEY, data TEXT);
        CREATE TABLE IF NOT EXISTS config(key TEXT PRIMARY KEY, data TEXT);
        """)
        db.execute("INSERT OR IGNORE INTO config VALUES ('policy', ?)", (json.dumps(DEFAULT_POLICY),))
    return root


def policy(db):
    return json.loads(db.execute("SELECT data FROM config WHERE key='policy'").fetchone()[0])


def load_task(db, task_id):
    row = db.execute("SELECT data FROM tasks WHERE id=?", (task_id,)).fetchone()
    if row is None:
        raise Rejection(404, "unknown task")
    return json.loads(row[0])


def save_task(db, task):
    db.execute("UPDATE tasks SET data=? WHERE id=?", (json.dumps(task), task["id"]))


def update_task(root, task_id, **changes):
    with connect(root) as db:
        db.execute("BEGIN IMMEDIATE")
        task = load_task(db, task_id)
        task.update(changes)
        save_task(db, task)


def atomic_bytes(path, data):
    """Commit bytes before publishing metadata. A crash may leave an unreferenced orphan."""
    path = Path(path)
    temporary = path.with_name(path.name + ".partial-" + uuid.uuid4().hex)
    with temporary.open("xb") as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())
    os.replace(temporary, path)
    descriptor = os.open(str(path.parent), os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def owned(db, task):
    row = db.execute("SELECT * FROM ownership WHERE lineage=?", (task["lineage"],)).fetchone()
    return row and row["writer"] == task["id"] and row["epoch"] == task["epoch"]


def checkpoint(root, task, state):
    started = time.perf_counter()
    checkpoint_id = uuid.uuid4().hex
    blob = encoded(state)
    path = Path(root) / "checkpoints" / (checkpoint_id + ".json")
    receipt = {"id": checkpoint_id, "source_execution": task["id"], "source_provider": task["provider"],
               "lineage": task["lineage"], "ownership_epoch": task["epoch"],
               "sequence": state["progress"] // 10, "progress": state["progress"],
               "artifact": "checkpoint:" + checkpoint_id, "sha256": hashlib.sha256(blob).hexdigest(),
               "size_bytes": len(blob), "format": APP_FORMAT, "application": "public-cpu-v1",
               "executable_digest": task["recipe"]["executable_digest"],
               "input_digest": task["recipe"]["input_digest"], "runtime": RUNTIME,
               "principal": task["principal"], "created_at": time.time(),
               "expires_at": time.time() + task["checkpoint_ttl"], "access": "same-principal-managed-only"}
    with connect(root) as db:
        db.execute("BEGIN IMMEDIATE")
        current = load_task(db, task["id"])
        active_policy = policy(db)
        storage_allowed = (task["provider"] in active_policy["allowed_storage_domains"] and
                           (not active_policy["no_export"] or task["provider"] == "alpha"))
        if current["state"] not in TERMINAL and owned(db, task) and storage_allowed:
            atomic_bytes(path, blob)
            db.execute("INSERT INTO receipts VALUES (?, ?, ?, ?)",
                       (checkpoint_id, task["id"], task["principal"], json.dumps(receipt)))
            current["checkpoint_bytes_written"] += len(blob)
            current["checkpoint_io_seconds"] = current.get("checkpoint_io_seconds", 0) + time.perf_counter() - started
            current["checkpoint_count"] += 1
            current["progress"] = state["progress"]
            save_task(db, current)
            return receipt
    return None
