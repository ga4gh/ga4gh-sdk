"""Behavior checks use real HTTP and separate CPU subprocesses."""
import concurrent.futures
import importlib.util
import json
from pathlib import Path
import tempfile
import time
import unittest
from urllib.error import HTTPError
from urllib.request import Request, urlopen

spec = importlib.util.spec_from_file_location("h3_provider", Path(__file__).with_name("provider.py"))
provider = importlib.util.module_from_spec(spec)
spec.loader.exec_module(provider)


def request(target, path, body=None, method=None, principal="demo"):
    data = None if body is None else json.dumps(body).encode()
    req = Request(target.url + path, data=data, method=method or ("GET" if body is None else "POST"),
                  headers={"Content-Type": "application/json", "X-H3-Principal": principal})
    try:
        with urlopen(req, timeout=10) as response:
            return response.status, json.load(response)
    except HTTPError as error:
        return error.code, json.load(error)


def wait_for(target, task_id, predicate, timeout=12):
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        status, result = request(target, f"/control/v1/tasks/{task_id}/evidence")
        if status == 200 and predicate(result):
            return result
        time.sleep(.025)
    raise AssertionError(f"Timed out for {task_id}: {result}")


class Providers(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="h3-provider-test-")
        self.cluster = provider.Cluster(Path(self.temp.name)).start()
        self.a, self.b = self.cluster.providers

    def tearDown(self):
        self.cluster.close()
        self.temp.cleanup()

    def create(self, target, **kw):
        status, result = request(target, "/v1/tasks", provider.make_task(**kw))
        self.assertEqual(status, 200, result)
        return result["id"]

    def test_real_cpu_dedup_and_protocol_cancel(self):
        service = request(self.a, "/v1/service-info")[1]
        self.assertTrue(all(key in service for key in ("id", "name", "type", "version", "organization")))
        task = provider.make_task(intent="stable", steps=80, delay_ms=5)
        _, first = request(self.a, "/v1/tasks", task)
        _, second = request(self.a, "/v1/tasks", task)
        self.assertEqual(first, second)
        self.assertEqual(request(self.a, "/v1/tasks", provider.make_task(parameter=4, intent="stable", steps=80, delay_ms=5))[0], 409)
        _, binding = request(self.a, "/control/v1/intents/stable")
        self.assertEqual(binding["task_id"], first["id"])
        self.assertEqual(binding["task_fingerprint"], provider.fingerprint(task))
        self.assertEqual(request(self.b, "/control/v1/intents/stable")[0], 501)
        status, listing = request(self.a, "/v1/tasks?tag_key=h3.intent&tag_value=stable&view=FULL")
        self.assertEqual([x["id"] for x in listing["tasks"]], [first["id"]])
        wait_for(self.a, first["id"], lambda x: x["physical_status"] == "RUNNING")
        self.assertEqual(request(self.a, f'/v1/tasks/{first["id"]}:cancel', {})[0], 200)
        evidence = wait_for(self.a, first["id"], lambda x: x["physical_status"] == "EXITED")
        self.assertEqual(evidence["state"], "CANCELED")
        self.assertIsNotNone(evidence["pid"])

    def test_checkpoint_continuation_matches_baseline_and_fences_live_source(self):
        baseline = self.create(self.a, steps=100, delay_ms=0, intent="baseline")
        wait_for(self.a, baseline, lambda x: x["state"] == "COMPLETE")
        source = self.create(self.a, steps=100, delay_ms=8, intent="source")
        wait_for(self.a, source, lambda x: x["checkpoint_count"] >= 1)
        _, receipts = request(self.a, f"/control/v1/tasks/{source}/checkpoints")
        receipt = receipts["receipts"][-1]
        task = provider.make_task(steps=100, delay_ms=8, intent="continuation")
        _, plan = request(self.b, "/control/v1/plans", {"task": task, "backend": "beta"})
        body = {"receipt_id": receipt["id"], "task": task, "plan_id": plan["id"], "intent": "continuation"}
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            results = list(pool.map(lambda _: request(self.b, "/control/v1/continue", body), range(2)))
        self.assertEqual(sorted(x[0] for x in results), [200, 409])
        target = next(x[1]["id"] for x in results if x[0] == 200)
        overlapping = wait_for(self.a, source, lambda x: not x["authorized_result_writer"])
        self.assertEqual(overlapping["physical_status"], "RUNNING")
        resumed = wait_for(self.b, target, lambda x: x["state"] == "COMPLETE")
        stopped = wait_for(self.a, source, lambda x: x["physical_status"] == "EXITED")
        self.assertEqual(stopped["state"], "PREEMPTED")
        self.assertTrue(stopped["result_commit_rejected"])
        self.assertGreater(resumed["restored_progress"], 0)
        self.assertEqual(resumed["performed_steps"], 100 - resumed["restored_progress"])
        self.assertEqual(request(self.a, f"/control/v1/tasks/{baseline}/metrics")[1],
                         request(self.b, f"/control/v1/tasks/{target}/metrics")[1])

    def test_admission_is_server_enforced_and_staging_is_controlled(self):
        task = provider.make_task()
        _, chosen = request(self.a, "/control/v1/plans", {"task": task})
        self.assertEqual(chosen["selected_backend"], "alpha")
        costs = {x["backend"]: x["estimate"] for x in chosen["decision_trace"]}
        self.assertGreater(costs["alpha"]["compute"], costs["beta"]["compute"])
        self.assertLess(costs["alpha"]["total"], costs["beta"]["total"])
        self.assertEqual(request(self.b, "/control/v1/submit", {"task": task, "plan_id": chosen["id"]})[0], 409)
        self.assertEqual(request(self.a, "/control/v1/submit", {"task": task, "plan_id": chosen["id"]}, principal="mallory")[0], 403)
        changed = provider.make_task(parameter=4)
        self.assertEqual(request(self.a, "/control/v1/submit", {"task": changed, "plan_id": chosen["id"]})[0], 409)
        cheap_remote = provider.make_task(steps=1000, delay_ms=0)
        _, unrestricted = request(self.a, "/control/v1/plans", {"task": cheap_remote})
        self.assertEqual(unrestricted["selected_backend"], "beta")
        request(self.a, "/control/v1/policy", {"no_export": True})
        _, restricted = request(self.a, "/control/v1/plans", {"task": cheap_remote})
        self.assertEqual(restricted["selected_backend"], "alpha")
        self.assertFalse(next(x for x in restricted["decision_trace"] if x["backend"] == "beta")["eligible"])
        self.assertEqual(request(self.a, "/control/v1/submit", {"task": task, "plan_id": chosen["id"]})[0], 409)
        _, plan = request(self.a, "/control/v1/plans", {"task": task})
        self.assertFalse(next(x for x in plan["decision_trace"] if x["backend"] == "beta")["eligible"])
        self.assertEqual(request(self.a, "/control/v1/stage", {"input_id": "public-numbers-v1", "destination": "beta", "plan_id": plan["id"]})[0], 403)
        self.assertEqual(request(self.b, "/v1/tasks", task)[0], 403)
        self.assertFalse((Path(self.temp.name) / "staging" / "beta").exists())
        self.assertEqual(request(self.a, "/control/v1/stage", {"input_id": "file:///etc/passwd", "destination": "alpha"})[0], 422)

    def checkpoint_source(self):
        source = self.create(self.a, steps=100, delay_ms=10, intent="checkpoint-source")
        wait_for(self.a, source, lambda x: x["checkpoint_count"] >= 1)
        request(self.a, f"/control/v1/tasks/{source}/preempt", {})
        wait_for(self.a, source, lambda x: x["physical_status"] == "EXITED")
        _, receipts = request(self.a, f"/control/v1/tasks/{source}/checkpoints")
        return source, receipts["receipts"][-1]

    def test_invalid_checkpoint_material_is_rejected_before_restore(self):
        source, receipt = self.checkpoint_source()
        task = provider.make_task(steps=100, delay_ms=10, intent="restore-negative")
        _, plan = request(self.b, "/control/v1/plans", {"task": task, "backend": "beta"})
        body = {"receipt_id": receipt["id"], "task": task, "plan_id": plan["id"], "intent": "restore-negative"}
        path = Path(self.temp.name) / "checkpoints" / (receipt["id"] + ".json")
        original = path.read_bytes()
        for corrupt in (original[:20], b"X" + original[1:]):
            with self.subTest(artifact_bytes=len(corrupt)):
                path.write_bytes(corrupt)
                status, result = request(self.b, "/control/v1/continue", body)
                self.assertEqual(status, 422, result)
                self.assertIn("size/digest", result["error"])
        path.write_bytes(original)
        for field, value, message in (("sha256", "0" * 64, "size/digest"),
                                      ("format", "alien/v2", "format"),
                                      ("runtime", "alien-architecture", "runtime"),
                                      ("input_digest", "0" * 64, "input"),
                                      ("executable_digest", "0" * 64, "executable")):
            with self.subTest(field=field):
                changed = receipt | {field: value}
                with provider.connect(self.temp.name) as db:
                    db.execute("UPDATE receipts SET data=? WHERE id=?", (json.dumps(changed), receipt["id"]))
                status, result = request(self.b, "/control/v1/continue", body)
                self.assertEqual(status, 422, result)
                self.assertIn(message, result["error"])
        with provider.connect(self.temp.name) as db:
            db.execute("UPDATE receipts SET data=? WHERE id=?", (json.dumps(receipt), receipt["id"]))
        changed_task = provider.make_task(parameter=4, steps=100, delay_ms=10)
        _, changed_plan = request(self.b, "/control/v1/plans", {"task": changed_task, "backend": "beta"})
        self.assertEqual(request(self.b, "/control/v1/continue", body | {"task": changed_task, "plan_id": changed_plan["id"]})[0], 422)
        _, other_plan = request(self.b, "/control/v1/plans", {"task": task, "backend": "beta"}, principal="mallory")
        denied = request(self.b, "/control/v1/continue", body | {"plan_id": other_plan["id"]}, principal="mallory")
        self.assertEqual(denied[0], 403)
        self.assertIn("checkpoint access", denied[1]["error"])
        changed_input = provider.make_task(steps=100, delay_ms=10)
        changed_input["inputs"][0]["contents"] = "different public input"
        self.assertEqual(request(self.b, "/control/v1/continue", body | {"task": changed_input})[0], 422)
        self.assertEqual(request(self.a, f"/control/v1/tasks/{source}/checkpoints", principal="mallory")[0], 403)
        self.assertEqual(request(self.b, "/control/v1/continue", body | {"mode": "process-snapshot"})[0], 422)
        self.cluster.fencing = False
        self.assertEqual(request(self.b, "/control/v1/continue", body)[0], 501)
        self.assertEqual(request(self.b, "/v1/tasks")[1]["tasks"], [])

    def test_expiry_and_compatible_preempted_restore_without_client_connection(self):
        task = provider.make_task(steps=100, delay_ms=10, intent="disconnected")
        _, created = request(self.a, "/v1/tasks", task)
        source = created["id"]
        first = wait_for(self.a, source, lambda x: x["checkpoint_count"] >= 1)
        # Drop all HTTP/SDK observation. A local test probe waits for worker-owned durable
        # progress instead of assuming that two fsynced checkpoints fit a fixed pause.
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            with provider.connect(self.temp.name) as db:
                disconnected = provider.load_task(db, source)
            if disconnected["checkpoint_count"] > first["checkpoint_count"]:
                break
            time.sleep(.02)
        self.assertGreater(disconnected["checkpoint_count"], first["checkpoint_count"])
        _, receipts = request(self.a, f"/control/v1/tasks/{source}/checkpoints")
        request(self.a, f"/control/v1/tasks/{source}/preempt", {})
        wait_for(self.a, source, lambda x: x["physical_status"] == "EXITED")
        receipt = receipts["receipts"][-1]
        _, expired = request(self.b, "/control/v1/plans", {"task": task, "backend": "beta", "ttl_seconds": .01})
        time.sleep(.03)
        self.assertEqual(request(self.b, "/control/v1/submit", {"task": task, "plan_id": expired["id"]})[0], 409)
        _, plan = request(self.b, "/control/v1/plans", {"task": task, "backend": "beta"})
        body = {"receipt_id": receipt["id"], "task": task, "plan_id": plan["id"], "intent": "restored-disconnected"}
        with provider.connect(self.temp.name) as db:
            db.execute("UPDATE receipts SET data=? WHERE id=?", (json.dumps(receipt | {"expires_at": time.time() - 1}), receipt["id"]))
        self.assertEqual(request(self.b, "/control/v1/continue", body)[0], 410)
        with provider.connect(self.temp.name) as db:
            db.execute("UPDATE receipts SET data=? WHERE id=?", (json.dumps(receipt), receipt["id"]))
        _, result = request(self.b, "/control/v1/continue", body)
        restored = wait_for(self.b, result["id"], lambda x: x["state"] == "COMPLETE")
        self.assertEqual(restored["restored_progress"], receipt["progress"])
        self.assertGreater(restored["checkpoint_bytes_read"], 0)
        self.assertLess(restored["performed_steps"], 100)

    def test_unknowns_failure_and_fallback_preserve_policy(self):
        for input_id, expected in (("unknown-size", "input size unknown"), ("unknown-location", "input location unknown")):
            task = provider.make_task(input_id=input_id)
            status, result = request(self.a, "/control/v1/plans", {"task": task})
            self.assertEqual(status, 403)
            self.assertIn(expected, result["error"])
        request(self.a, "/control/v1/policy", {"strict": False})
        status, uncertain = request(self.a, "/control/v1/plans", {"task": provider.make_task(input_id="unknown-size")})
        self.assertEqual(status, 200)
        self.assertIsNone(uncertain["decision_trace"][0]["estimate"]["total"])
        self.assertIn("input size unknown", uncertain["decision_trace"][0]["uncertainty"])
        request(self.a, "/control/v1/policy", {"no_export": True, "strict": True})
        request(self.a, "/control/v1/availability", {"available": False})
        status, failure = request(self.a, "/control/v1/plans", {"task": provider.make_task()})
        self.assertEqual(status, 403)
        self.assertIn("backend unavailable", failure["error"])
        self.assertIn("no-export", failure["error"])
        self.assertEqual(request(self.b, "/v1/tasks", provider.make_task())[0], 403)
        self.assertEqual(request(self.a, "/control/v1/stage", {"input_id": "public-numbers-v1", "destination": "beta"})[0], 403)

    def test_policy_is_rechecked_for_managed_output_publication(self):
        task_id = self.create(self.b, steps=60, delay_ms=10)
        wait_for(self.b, task_id, lambda x: x["checkpoint_count"] >= 1)
        request(self.a, "/control/v1/policy", {"allowed_storage_domains": ["alpha"]})
        count = request(self.b, f"/control/v1/tasks/{task_id}/evidence")[1]["checkpoint_count"]
        evidence = wait_for(self.b, task_id, lambda x: x["physical_status"] == "EXITED")
        self.assertEqual(evidence["state"], "SYSTEM_ERROR")
        self.assertIn("forbids managed result", evidence["error"])
        self.assertEqual(evidence["checkpoint_count"], count)
        self.assertEqual(request(self.b, f"/control/v1/tasks/{task_id}/metrics")[0], 409)
        self.assertFalse((Path(self.temp.name) / "results" / (task_id + ".json")).exists())

    def test_intent_binding_survives_provider_restart(self):
        task = provider.make_task(intent="durable-restart", steps=10, delay_ms=0)
        _, created = request(self.a, "/v1/tasks", task)
        wait_for(self.a, created["id"], lambda x: x["physical_status"] == "EXITED")
        self.cluster.close()
        self.cluster = provider.Cluster(Path(self.temp.name)).start()
        self.a, self.b = self.cluster.providers
        _, binding = request(self.a, "/control/v1/intents/durable-restart")
        self.assertEqual(binding["task_id"], created["id"])
        self.assertEqual(request(self.a, "/v1/tasks", task)[1], created)
        self.assertEqual(len(request(self.a, "/v1/tasks")[1]["tasks"]), 1)

    def test_without_dedup_lost_ack_has_no_reconciliation_proof(self):
        import http.client
        connection = http.client.HTTPConnection("127.0.0.1", self.b.server.server_port)
        task = provider.make_task(intent="lost-ack", steps=100, delay_ms=10)
        connection.request("POST", "/v1/tasks", body=json.dumps(task), headers={"Content-Type": "application/json"})
        connection.close()  # Accept may have happened, no task ID is recorded by this caller.
        end = time.monotonic() + 3
        listing = {"tasks": []}
        while time.monotonic() < end and not listing["tasks"]:
            _, listing = request(self.b, "/v1/tasks?tag_key=h3.intent&tag_value=lost-ack")
            time.sleep(.02)
        self.assertEqual(len(listing["tasks"]), 1)
        self.assertEqual(request(self.b, "/control/v1/intents/lost-ack")[0], 501)
        self.assertEqual(len(request(self.b, "/v1/tasks")[1]["tasks"]), 1)  # No retry POST.


if __name__ == "__main__":
    unittest.main()
