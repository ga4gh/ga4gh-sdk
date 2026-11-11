#!/usr/bin/env python3
"""Run bounded real CPU checkpoint/admission demonstrations and save evidence."""
import argparse
import concurrent.futures
import json
from pathlib import Path
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from provider import Cluster, make_task


def request(target, path, body=None, principal="demo"):
    req = Request(target.url + path, data=None if body is None else json.dumps(body).encode(),
                  headers={"Content-Type": "application/json", "X-H3-Principal": principal})
    try:
        with urlopen(req, timeout=10) as response:
            return response.status, json.load(response)
    except HTTPError as error:
        return error.code, json.load(error)


def ok(target, path, body=None):
    status, result = request(target, path, body)
    assert status == 200, (status, result)
    return result


def wait(target, task_id, predicate):
    until = time.monotonic() + 20
    while time.monotonic() < until:
        result = ok(target, f"/control/v1/tasks/{task_id}/evidence")
        if predicate(result):
            return result
        time.sleep(.02)
    raise AssertionError("worker did not reach expected state: " + json.dumps(result))


def demonstrate(root):
    with Cluster(root) as cluster:
        alpha, beta = cluster.providers
        baseline = ok(alpha, "/v1/tasks", make_task(steps=100, delay_ms=0, intent="baseline"))["id"]
        wait(alpha, baseline, lambda x: x["physical_status"] == "EXITED")
        source = ok(alpha, "/v1/tasks", make_task(steps=100, delay_ms=10, intent="preempt-source"))["id"]
        time.sleep(.28)  # The worker checkpoints independently with no connected client.
        receipts = ok(alpha, f"/control/v1/tasks/{source}/checkpoints")["receipts"]
        assert receipts, "no durable checkpoint produced while client disconnected"
        ok(alpha, f"/control/v1/tasks/{source}/preempt", {})
        stopped = wait(alpha, source, lambda x: x["physical_status"] == "EXITED")
        receipt = receipts[-1]
        task = make_task(steps=100, delay_ms=0, intent="preempt-target")
        plan = ok(beta, "/control/v1/plans", {"task": task, "backend": "beta"})
        restored = ok(beta, "/control/v1/continue", {"receipt_id": receipt["id"], "task": task,
                      "plan_id": plan["id"], "intent": "preempt-target"})["id"]
        recovered = wait(beta, restored, lambda x: x["physical_status"] == "EXITED")
        expected = ok(alpha, f"/control/v1/tasks/{baseline}/metrics")
        actual = ok(beta, f"/control/v1/tasks/{restored}/metrics")
        assert expected == actual
        assert recovered["performed_steps"] == 100 - receipt["progress"]

        live_source = ok(alpha, "/v1/tasks", make_task(steps=100, delay_ms=10, intent="live-source"))["id"]
        wait(alpha, live_source, lambda x: x["checkpoint_count"] >= 1)
        live_receipt = ok(alpha, f"/control/v1/tasks/{live_source}/checkpoints")["receipts"][-1]
        task = make_task(steps=100, delay_ms=10, intent="live-successor")
        plan = ok(beta, "/control/v1/plans", {"task": task, "backend": "beta"})
        continuation = {"receipt_id": live_receipt["id"], "task": task,
                        "plan_id": plan["id"], "intent": "live-successor"}
        with concurrent.futures.ThreadPoolExecutor(2) as executor:
            race = list(executor.map(lambda _: request(beta, "/control/v1/continue", continuation), range(2)))
        assert sorted(status for status, _ in race) == [200, 409]
        live_target = next(result["id"] for status, result in race if status == 200)
        overlap = ok(alpha, f"/control/v1/tasks/{live_source}/evidence")
        assert overlap["physical_status"] == "RUNNING" and not overlap["authorized_result_writer"]
        fenced_source = wait(alpha, live_source, lambda x: x["physical_status"] == "EXITED")
        live_result = wait(beta, live_target, lambda x: x["physical_status"] == "EXITED")
        assert fenced_source["state"] == "PREEMPTED" and fenced_source["result_commit_rejected"]
        assert live_result["state"] == "COMPLETE"
        assert ok(beta, f"/control/v1/tasks/{live_target}/metrics") == expected

        local_preferred = ok(alpha, "/control/v1/plans", {"task": make_task()})
        assert local_preferred["selected_backend"] == "alpha"
        cheaper_remote_task = make_task(steps=1000, delay_ms=0)
        cheaper_remote = ok(alpha, "/control/v1/plans", {"task": cheaper_remote_task})
        assert cheaper_remote["selected_backend"] == "beta"
        unknown_status, unknown = request(alpha, "/control/v1/plans", {"task": make_task(input_id="unknown-size")})
        assert unknown_status == 403 and "unknown" in unknown["error"]
        ok(alpha, "/control/v1/policy", {"no_export": True})
        restricted = ok(alpha, "/control/v1/plans", {"task": cheaper_remote_task})
        assert restricted["selected_backend"] == "alpha"
        transfer_status, transfer = request(alpha, "/control/v1/stage", {"input_id": "public-numbers-v1", "destination": "beta"})
        assert transfer_status == 403
        forbidden_status, forbidden = request(beta, "/v1/tasks", make_task(intent="forbidden-fallback"))
        assert forbidden_status == 403
        ok(alpha, "/control/v1/availability", {"available": False})
        fallback_status, fallback = request(alpha, "/control/v1/plans", {"task": cheaper_remote_task})
        assert fallback_status == 403
        return {"schema": "h3-provider-demo/v1", "execution_mode": "real host CPU subprocesses, two loopback HTTP targets",
                "checkpoint": {"receipt": receipt, "source": stopped, "successor": recovered,
                               "uninterrupted_metrics": expected, "restored_metrics": actual, "outputs_match": True,
                               "completed_work_skipped": recovered["restored_progress"],
                               "repeated_steps_before_preemption": stopped["performed_steps"] - receipt["progress"],
                               "checkpoint_io_note": "seconds include artifact fsync/rename and metadata transaction setup; final DB commit excluded"},
                "fencing": {"competing_continuation_statuses": [status for status, _ in race],
                            "source_during_overlap": overlap, "source_after_exit": fenced_source,
                            "successor": live_result, "exactly_once_physical_execution": False},
                "admission": {"higher_compute_lower_total": local_preferred,
                              "cheaper_remote_without_restriction": cheaper_remote,
                              "no_export_plan": restricted, "forbidden_transfer": {"status": transfer_status, **transfer},
                              "forbidden_submission": {"status": forbidden_status, **forbidden},
                              "unknown_size": {"status": unknown_status, **unknown},
                              "policy_preserving_failure": {"status": fallback_status, **fallback}},
                "limits": ["co-hosted logical domains are not geographic residency", "no container isolation or arbitrary traffic controls",
                           "shared durable coordinator is required for exclusive result commit", "synthetic costs are advisory",
                           "supervisor restart can leave physical process status unknown; no blind relaunch"]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--root", type=Path, help="new or dedicated evidence directory; default temporary")
    arguments = parser.parse_args()
    if arguments.root:
        evidence = demonstrate(arguments.root)
    else:
        with tempfile.TemporaryDirectory(prefix="h3-portable-") as root:
            evidence = demonstrate(Path(root))
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(json.dumps(evidence, indent=2) + "\n")
    print(json.dumps({"report": str(arguments.output), "outputs_match": evidence["checkpoint"]["outputs_match"],
                      "restored_progress": evidence["checkpoint"]["completed_work_skipped"],
                      "race_statuses": evidence["fencing"]["competing_continuation_statuses"],
                      "forbidden_transfer_status": evidence["admission"]["forbidden_transfer"]["status"]}))


if __name__ == "__main__":
    main()
