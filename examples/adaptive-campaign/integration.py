"""Opt-in, real local restart demonstration. No Docker, cloud, or mocked TES I/O."""
import asyncio
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time

from temporalio.common import WorkflowIDReusePolicy
from temporalio.testing import WorkflowEnvironment
from temporalio.worker import Replayer
from activities import read_json
from model import POLICY_VERSION
from workflow import AdaptiveCampaign


async def until(check, *, seconds=30, label="condition"):
    end = time.monotonic() + seconds
    last = None
    while time.monotonic() < end:
        try:
            last = await check()
        except Exception as error:
            last = repr(error)
        if last and not isinstance(last, str):
            return last
        await asyncio.sleep(.05)
    raise AssertionError(f"timed out waiting for {label}: {last}")


def load_provider(path):
    sys.path.insert(0, str(path.parent))
    spec = importlib.util.spec_from_file_location("h3_campaign_providers", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


async def demonstrate(provider_path, evidence_dir=None):
    provider_path = Path(provider_path).resolve()
    if not provider_path.exists():
        raise SystemExit("integration requires examples/portable-execution/provider.py (see --providers)")
    import GA4GH  # Fail before launching services if the optional binding is unavailable.
    import temporalio
    root = Path(evidence_dir) if evidence_dir else Path(tempfile.mkdtemp(prefix="h3-campaign-evidence-"))
    root.mkdir(parents=True, exist_ok=True)
    runtime = Path(tempfile.gettempdir()) / "h3-campaign-runtime"
    runtime.mkdir(exist_ok=True)
    provider_module = load_provider(provider_path)
    cluster = provider_module.Cluster(root / "providers").start()
    workers, logs = [], []
    summary = {"temporal_python": temporalio.__version__, "binding": str(GA4GH.__file__),
               "scope": "local cooperating TES endpoints and real host CPU subprocesses",
               "policy": POLICY_VERSION, "evidence": str(root), "checks": []}
    try:
        async with await WorkflowEnvironment.start_local(download_dest_dir=str(runtime),
                dev_server_database_filename=str(root / "temporal.sqlite")) as env:
            address = env.client.service_client.config.target_host
            providers = [{"name": p.name, "url": p.url} for p in cluster.providers]
            (root / "manifest.json").write_text(json.dumps({"providers": providers, "temporal": address}, indent=2))
            queue = "h3-campaign-restart-proof"

            def worker(name, crash_trial=None):
                log = (root / (name + ".log")).open("w")
                logs.append(log)
                command = [sys.executable, str(Path(__file__).with_name("controller.py")),
                           "--address", address, "--task-queue", queue, "worker"]
                if crash_trial:
                    command += ["--crash-trial", crash_trial, "--crash-marker", str(root / (name + ".crash.json"))]
                process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                workers.append(process)
                return process

            async def stop(process):
                if process.poll() is None:
                    process.kill()
                    await asyncio.to_thread(process.wait, 5)

            async def snapshot(handle):
                return await handle.query(AdaptiveCampaign.snapshot)

            async def save_history(handle, name):
                history = await handle.fetch_history()
                (root / (name + ".history.json")).write_text(history.to_json())
                await Replayer(workflows=[AdaptiveCampaign]).replay_workflow(history)
                summary["checks"].append(name + ": deterministic history replay")
                return history

            # Interrupt once during two acknowledged, still physically running jobs.
            first = worker("worker-1")
            config = dict(providers=providers, max_trials=6, max_concurrency=2,
                          deadline_seconds=18, steps=80, delay_ms=20)
            handle = await env.client.start_workflow(AdaptiveCampaign.run, config,
                id="bounded-restart-demo", task_queue=queue,
                id_reuse_policy=WorkflowIDReusePolicy.REJECT_DUPLICATE)
            async def known_runs():
                state = await snapshot(handle)
                if len(state.get("trials", [])) != 2 or not all(t["state"] == "ACKNOWLEDGED" for t in state["trials"]):
                    return json.dumps(state)
                physical = [await asyncio.to_thread(read_json, p["provider"]["url"] + "/control/v1/tasks/" + p["remote_id"] + "/evidence") for p in state["trials"]]
                return state if all(p["physical_status"] == "RUNNING" for p in physical) else None
            before = await until(known_runs, label="two acknowledged running CPU trials")
            (root / "before-known-restart.json").write_text(json.dumps(before, indent=2))
            await stop(first)
            summary["known_restart"] = {"worker_pid": first.pid, "returncode": first.returncode,
                                        "remote_ids": [t["remote_id"] for t in before["trials"]]}
            # After those jobs finish, crash on trial 3 (beta has no receipt API).
            second = worker("worker-2", "bounded-restart-demo:trial:3")
            async def crashed():
                return {"returncode": second.returncode} if second.poll() is not None else None
            await until(crashed, label="accepted-before-persistence worker crash")
            assert second.returncode == 86, (root / "worker-2.log").read_text()
            marker = json.loads((root / "worker-2.crash.json").read_text())
            assert "remote_id" not in marker
            # Provider-side evidence independently proves the ambiguous task was accepted.
            beta_tasks = await asyncio.to_thread(read_json, providers[1]["url"] + "/v1/tasks?view=FULL")
            ambiguous = [t for t in beta_tasks["tasks"] if t["tags"]["h3.intent"] == marker["trial_id"]]
            assert len(ambiguous) == 1
            summary["ambiguous_restart"] = {"worker_pid": second.pid, "returncode": second.returncode,
                                             "marker": marker, "provider_accepted_id": ambiguous[0]["id"]}
            third = worker("worker-3")
            async def recovered_unknown():
                state = await snapshot(handle)
                unknown = [t for t in state.get("trials", []) if t["trial_id"] == marker["trial_id"]
                           and t["state"] == "SUBMISSION_OUTCOME_UNKNOWN"]
                return state if unknown and unknown[0].get("reconciliation_final") else None
            recovered = await until(recovered_unknown, label="unresolved beta reservation")
            unknown = next(t for t in recovered["trials"] if t["trial_id"] == marker["trial_id"])
            assert unknown["reservation"] and "remote_id" not in unknown
            assert unknown["reconciliation"]["http_status"] == 501
            (root / "after-ambiguous-restart.json").write_text(json.dumps(recovered, indent=2))
            final = await asyncio.wait_for(handle.result(), timeout=30)
            assert len(final["trials"]) == 6 and final["peak_reservations"] == 2
            assert final["status"] == "NEEDS_EXPLICIT_RESOLUTION"
            assert final["decisions"][:2] == before["decisions"]
            assert final["decisions"][:len(recovered["decisions"])] == recovered["decisions"]
            assert final["results"][:len(recovered["results"])] == recovered["results"]
            assert len({r["event_id"] for r in final["results"]}) == len(final["results"]) == 5
            assert {t["provider"]["name"] for t in final["trials"] if t.get("outcome", {}).get("kind") == "VALID_METRICS"} == {"alpha", "beta"}
            assert any(d["result_events"] for d in final["decisions"][2:])
            remote_tasks = []
            for provider in providers:
                remote_tasks += (await asyncio.to_thread(read_json, provider["url"] + "/v1/tasks?view=FULL"))["tasks"]
            main_tasks = [t for t in remote_tasks if t["tags"]["h3.intent"].startswith("bounded-restart-demo:")]
            assert len(main_tasks) == 6
            assert len({t["tags"]["h3.intent"] for t in main_tasks}) == 6
            for initial in before["trials"]:
                final_trial = next(t for t in final["trials"] if t["trial_id"] == initial["trial_id"])
                assert final_trial["remote_id"] == initial["remote_id"]
            (root / "campaign-result.json").write_text(json.dumps(final, indent=2))
            summary["checks"] += ["known-run restart retained acknowledged IDs", "six intents, six accepted jobs, no duplicate submissions",
                                  "five unique scored results across both providers", "saved decision and result prefixes unchanged",
                                  "beta unknown reservation retained through deadline", "two campaign reservations maximum"]
            await save_history(handle, "bounded-restart-demo")
            await stop(third)

            # A separate real crash proves positive provider-backed recovery.
            fourth = worker("worker-4", "receipt-recovery-demo:trial:0")
            receipt_handle = await env.client.start_workflow(AdaptiveCampaign.run,
                {**config, "max_trials": 2, "max_concurrency": 1, "deadline_seconds": 15},
                id="receipt-recovery-demo", task_queue=queue)
            async def receipt_crashed():
                return {"returncode": fourth.returncode} if fourth.poll() is not None else None
            await until(receipt_crashed, label="alpha accepted-before-persistence crash")
            assert fourth.returncode == 86, (root / "worker-4.log").read_text()
            fifth = worker("worker-5")
            receipt_result = await asyncio.wait_for(receipt_handle.result(), timeout=30)
            adopted = receipt_result["trials"][0]
            assert adopted["acknowledgement"]["source"] == "provider_receipt"
            assert adopted["reconciliation"]["evidence"]["task_fingerprint"] == adopted["task_fingerprint"]
            assert len(receipt_result["results"]) == 2
            (root / "receipt-recovery-result.json").write_text(json.dumps(receipt_result, indent=2))
            summary["checks"].append("alpha adopts lost acknowledgement only from matching provider receipt")
            await save_history(receipt_handle, "receipt-recovery-demo")

            # Deadline cancellation: protocol terminal observed before slot release.
            deadline_handle = await env.client.start_workflow(AdaptiveCampaign.run,
                {**config, "max_trials": 8, "deadline_seconds": .4, "steps": 200, "delay_ms": 20},
                id="deadline-demo", task_queue=queue)
            deadline_result = await asyncio.wait_for(deadline_handle.result(), timeout=20)
            assert len(deadline_result["trials"]) == 2
            assert deadline_result["deadline_reached"]
            assert all(t["cancel_requested"] and t["protocol_state"] == "CANCELED" and not t["reservation"] for t in deadline_result["trials"])
            (root / "deadline-result.json").write_text(json.dumps(deadline_result, indent=2))
            summary["checks"].append("deadline stops new trials; cancellation slots released only after terminal CANCELED")
            await save_history(deadline_handle, "deadline-demo")
            summary.update(main_trials=len(final["trials"]), main_valid_results=len(final["results"]),
                           retained_unknown=sum(t["state"] == "SUBMISSION_OUTCOME_UNKNOWN" for t in final["trials"]),
                           peak_reservations=final["peak_reservations"])
            (root / "summary.json").write_text(json.dumps(summary, indent=2))
            print(json.dumps(summary, indent=2), flush=True)
    finally:
        for process in workers:
            if process.poll() is None:
                process.kill()
                await asyncio.to_thread(process.wait, 5)
        for log in logs:
            log.close()
        await asyncio.to_thread(cluster.close)
