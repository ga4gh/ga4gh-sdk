"""Durable ownership of bounded adaptive trials, using Temporal event history."""
from datetime import timedelta

from temporalio import workflow
from temporalio.common import RetryPolicy
from temporalio.exceptions import ActivityError
with workflow.unsafe.imports_passed_through():
    from model import (POLICY_VERSION, apply_observation, choose_parameter, digest,
                       recipe_digest, task_request, validate_config)

ONE_ATTEMPT = RetryPolicy(maximum_attempts=1)


@workflow.defn(name="ga4gh-adaptive-campaign-v1")
class AdaptiveCampaign:
    def __init__(self):
        self.state = {}
        self.resolutions = []

    @workflow.query
    def snapshot(self) -> dict:
        return self.state

    @workflow.signal
    def retire_unknown(self, resolution: dict):
        # Explicit operator risk acceptance, never evidence of remote absence.
        if resolution.get("trial_id") and resolution.get("reason", "").strip():
            self.resolutions.append(resolution)

    async def call(self, name, trial, timeout=5):
        try:
            return await workflow.execute_activity(name, trial,
                start_to_close_timeout=timedelta(seconds=timeout),
                schedule_to_close_timeout=timedelta(seconds=timeout + 2),
                retry_policy=ONE_ATTEMPT)
        except ActivityError:
            return {"error": "activity failed or timed out"}

    @workflow.run
    async def run(self, config: dict) -> dict:
        validate_config(config)
        deadline = workflow.now() + timedelta(seconds=config["deadline_seconds"])
        self.state = {"campaign_id": workflow.info().workflow_id, "policy": POLICY_VERSION,
                      "recipe_digest": recipe_digest(), "deadline": deadline.isoformat(),
                      "max_trials": config["max_trials"], "max_concurrency": config["max_concurrency"],
                      "status": "RUNNING", "trials": [], "decisions": [], "results": [],
                      "peak_reservations": 0, "deadline_reached": False,
                      "limits_scope": "owned protocol submissions, not physical resources or currency"}
        trials, results = self.state["trials"], self.state["results"]
        while True:
            expired = workflow.now() >= deadline
            self.state["deadline_reached"] = expired
            for resolution in self.resolutions:
                for trial in trials:
                    if trial["trial_id"] == resolution["trial_id"] and trial["state"] == "SUBMISSION_OUTCOME_UNKNOWN":
                        trial.update(state="OPERATOR_RETIRED_UNKNOWN", reservation=False,
                                     explicit_resolution=resolution,
                                     cleanup="UNKNOWN", outcome={"kind": "UNRESOLVED_RETIRED_BY_OPERATOR"})
            self.resolutions.clear()
            for trial in trials:
                if trial["state"] == "SUBMISSION_OUTCOME_UNKNOWN" and not trial.get("reconciliation_final"):
                    proof = await self.call("reconcile", trial)
                    trial["reconciliation"] = proof
                    if proof.get("kind") == "ADOPTED":
                        trial.update(state="ACKNOWLEDGED", remote_id=proof["remote_id"],
                                     acknowledgement={"source": "provider_receipt", "evidence": proof["evidence"]},
                                     reconciliation_final=True)
                    elif proof.get("kind") == "UNRESOLVED":
                        trial["reconciliation_final"] = True
                if trial["state"] != "ACKNOWLEDGED":
                    continue
                if expired:
                    trial["cancel_requested"] = True
                    if trial.get("cancel_acknowledgement", {}).get("request") != "ACKNOWLEDGED":
                        trial["cancel_acknowledgement"] = await self.call("cancel", trial)
                observation = await self.call("observe", trial)
                apply_observation(trial, observation)
                if "physical_evidence" in observation:
                    trial["physical_evidence"] = observation["physical_evidence"]
                if trial["state"] == "TERMINAL":
                    if trial["protocol_state"] == "COMPLETE":
                        outcome = await self.call("collect_result", trial)
                        if "error" in outcome:
                            outcome = {"kind": "METRICS_UNAVAILABLE"}
                        trial.update(state="RESULT_RECORDED", outcome=outcome,
                                     result_reference=outcome.get("reference"))
                        # Replayed ActivityCompleted events and duplicate observations
                        # cannot score a trial more than once.
                        event_id = trial["trial_id"] + ":" + trial["remote_id"]
                        if outcome["kind"] == "VALID_METRICS" and not any(r["event_id"] == event_id for r in results):
                            results.append({"event_id": event_id, "trial_id": trial["trial_id"],
                                            "parameter": trial["parameter"], "score": outcome["score"]})
                    else:
                        trial["outcome"] = {"kind": "REMOTE_" + trial["protocol_state"]}
            # Recompute time after remote observations: no late dispatch from a stale check.
            expired = workflow.now() >= deadline
            self.state["deadline_reached"] = expired
            pending = []
            while (not expired and len(trials) < config["max_trials"]
                   and sum(t["reservation"] for t in trials) < config["max_concurrency"]):
                index = len(trials)
                parameter, decision = choose_parameter(trials, results)
                trial_id = f"{self.state['campaign_id']}:trial:{index}"
                trial = {"trial_id": trial_id, "intent_id": trial_id,
                         "recipe_digest": self.state["recipe_digest"], "deadline": deadline.isoformat(),
                         "provider": config["providers"][index % 2],
                         "parameter": parameter, "steps": config["steps"], "delay_ms": config["delay_ms"],
                         "state": "RESERVED", "reservation": True,
                         "protocol_state": None, "cleanup": "UNKNOWN", "cancel_requested": False}
                trial["task_fingerprint"] = digest(task_request(trial))
                trial["dispatch_intent"] = {"intent_id": trial_id, "request_fingerprint": trial["task_fingerprint"],
                                            "reserved_at": workflow.now().isoformat()}
                trials.append(trial)
                self.state["decisions"].append({"trial_id": trial_id, "parameter": parameter, **decision})
                self.state["peak_reservations"] = max(self.state["peak_reservations"], sum(t["reservation"] for t in trials))
                # ActivityTaskScheduled is persisted by Temporal before any POST.
                # The workflow's reserved identity/decision reconstruct on replay.
                trial["state"] = "DISPATCHING"
                pending.append((trial, workflow.start_activity("dispatch", trial,
                    start_to_close_timeout=timedelta(seconds=4),
                    schedule_to_close_timeout=timedelta(seconds=6), retry_policy=ONE_ATTEMPT)))
            for trial, handle in pending:
                try:
                    response = await handle
                except ActivityError:
                    response = {"kind": "UNKNOWN", "reason": "dispatch activity failed or timed out"}
                if response.get("kind") == "ACKNOWLEDGED":
                    trial.update(state="ACKNOWLEDGED", remote_id=response["remote_id"],
                                 acknowledgement={"source": "TES_create_response", "remote_id": response["remote_id"]})
                elif response.get("kind") == "NOT_DISPATCHED_DEADLINE":
                    trial.update(state="DEADLINE_NOT_DISPATCHED", reservation=False,
                                 cleanup="NOT_STARTED", outcome={"kind": "DEADLINE_NOT_DISPATCHED"})
                else:
                    trial.update(state="SUBMISSION_OUTCOME_UNKNOWN", dispatch_failure=response)
            active = [t for t in trials if t["state"] in ("DISPATCHING", "ACKNOWLEDGED")]
            unknown = [t for t in trials if t["state"] == "SUBMISSION_OUTCOME_UNKNOWN"]
            if not active and (expired or len(trials) >= config["max_trials"]):
                if unknown and not expired:
                    self.state["status"] = "NEEDS_EXPLICIT_RESOLUTION"
                else:
                    self.state["status"] = "NEEDS_EXPLICIT_RESOLUTION" if unknown else ("DEADLINE_REACHED" if expired else "COMPLETE")
                    return self.state
            await workflow.sleep(timedelta(seconds=.1))
