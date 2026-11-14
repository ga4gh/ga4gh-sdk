"""Fixed checkpoint-aware public CPU program, launched only by provider.py."""
import hashlib
import json
from pathlib import Path
import sys
import time

from common import (APP_FORMAT, INPUT_ID, RUNTIME, atomic_bytes, checkpoint,
                    connect, encoded, load_task, owned, policy, save_task, update_task)


def run(root, task_id):
    root = Path(root)
    with connect(root) as db:
        task = load_task(db, task_id)
    recipe = task["recipe"]
    public_input = (root / "staging" / task["provider"] / INPUT_ID).read_bytes()
    if hashlib.sha256(public_input).hexdigest() != recipe["input_digest"]:
        raise ValueError("staged input digest mismatch")
    state = task.get("restore") or {"format": APP_FORMAT, "runtime": RUNTIME, "progress": 0,
            "rng_state": 123456789, "accumulator": "0" * 64,
            "parameter": recipe["parameter"], "steps": recipe["steps"],
            "input_digest": recipe["input_digest"], "executable_digest": recipe["executable_digest"]}
    start = state["progress"]
    cpu_started = time.process_time()
    for step in range(start, recipe["steps"]):
        state["rng_state"] = (1664525 * state["rng_state"] + 1013904223) % (2 ** 32)
        value = bytes.fromhex(state["accumulator"]) + public_input + encoded([recipe["parameter"], step, state["rng_state"]])
        for _ in range(300):
            value = hashlib.sha256(value).digest()
        state["accumulator"] = value.hex()
        state["progress"] = step + 1
        update_task(root, task_id, progress=step + 1, performed_steps=step + 1 - start)
        if (step + 1) % 10 == 0:
            checkpoint(root, task, state)
        if recipe["delay_ms"]:
            time.sleep(recipe["delay_ms"] / 1000)
    metrics = {"schema": "h3-cpu/v1", "parameter": recipe["parameter"], "steps": recipe["steps"],
               "score": -(recipe["parameter"] - 3) ** 2, "checksum": state["accumulator"]}
    with connect(root) as db:
        db.execute("BEGIN IMMEDIATE")
        current = load_task(db, task_id)
        current["cpu_seconds"] = time.process_time() - cpu_started
        if not owned(db, task):
            current.update(state="PREEMPTED", result_commit_rejected=True, ended_at=time.time())
        elif current["state"] in {"CANCELING", "CANCELED", "PREEMPTED"}:
            return
        else:
            active_policy = policy(db)
            if task["provider"] not in active_policy["allowed_storage_domains"] or (active_policy["no_export"] and task["provider"] != "alpha"):
                current.update(state="SYSTEM_ERROR", error="current policy forbids managed result publication", ended_at=time.time())
            else:
                # The result file rename and DB pointer commit are protected by the ownership transaction.
                # Only the DB pointer is authoritative; orphan bytes are never served.
                path = root / "results" / (task["lineage"] + ".json")
                atomic_bytes(path, encoded(metrics))
                db.execute("UPDATE ownership SET result=? WHERE lineage=? AND writer=? AND epoch=?",
                           (json.dumps(metrics), task["lineage"], task_id, task["epoch"]))
                current.update(state="COMPLETE", metrics=metrics, ended_at=time.time())
        save_task(db, current)


if __name__ == "__main__":
    try:
        run(sys.argv[1], sys.argv[2])
    except Exception as error:
        update_task(sys.argv[1], sys.argv[2], state="SYSTEM_ERROR", error=str(error), ended_at=time.time())
        raise
