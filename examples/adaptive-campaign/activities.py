"""Remote I/O lives here; TES operations use the existing GA4GH Rust binding."""
import json
from datetime import datetime, timezone
import os
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import urlopen

from temporalio import activity
from model import digest, metric_outcome, task_request


def read_json(url):
    with urlopen(url, timeout=2) as response:
        return json.load(response)


def tes_client(provider):
    # Optional native dependency. See the repository Python binding guide.
    from GA4GH import Configuration, TES
    return TES(Configuration(provider["url"] + "/v1"))


class Activities:
    def __init__(self, crash_trial=None, crash_marker=None):
        self.crash_trial = crash_trial
        self.crash_marker = crash_marker

    @activity.defn
    def dispatch(self, trial: dict) -> dict:
        """One attempt only. Any failure is ambiguous, never a blind POST retry."""
        try:
            deadline = datetime.fromisoformat(trial["deadline"])
            if datetime.now(timezone.utc) >= deadline:
                return {"kind": "NOT_DISPATCHED_DEADLINE"}
            client = tes_client(trial["provider"])
            if datetime.now(timezone.utc) >= deadline:
                return {"kind": "NOT_DISPATCHED_DEADLINE"}
            task = client.create(json.dumps(task_request(trial)))
            remote_id = task.id
            if not remote_id:
                return {"kind": "UNKNOWN", "reason": "empty acknowledgement"}
            if trial["trial_id"] == self.crash_trial:
                # Test notification only, with no remote ID. Neither this file nor
                # a client ledger is used to establish remote ownership.
                if self.crash_marker:
                    path = Path(self.crash_marker)
                    with path.open("w") as stream:
                        json.dump({"trial_id": trial["trial_id"], "point": "accepted-before-activity-completion"}, stream)
                        stream.flush()
                        os.fsync(stream.fileno())
                os._exit(86)
            return {"kind": "ACKNOWLEDGED", "remote_id": remote_id}
        except Exception as error:
            return {"kind": "UNKNOWN", "reason": type(error).__name__}

    @activity.defn
    def reconcile(self, trial: dict) -> dict:
        """Only the documented provider receipt can adopt an ambiguous intent."""
        provider = trial["provider"]
        url = provider["url"] + "/control/v1/intents/" + quote(trial["intent_id"], safe="")
        try:
            evidence = read_json(url)
        except HTTPError as error:
            if error.code in (404, 501):
                return {"kind": "UNRESOLVED", "reason": "no provider-backed binding; explicit resolution required",
                        "http_status": error.code}
            return {"kind": "RETRY_OBSERVATION", "reason": "receipt observation failed"}
        except (URLError, TimeoutError, ValueError):
            return {"kind": "RETRY_OBSERVATION", "reason": "receipt observation failed"}
        if (not isinstance(evidence, dict) or evidence.get("intent") != trial["intent_id"]
                or evidence.get("task_fingerprint") != digest(task_request(trial))
                or evidence.get("provider") != provider["name"]
                or not isinstance(evidence.get("task_id"), str) or not evidence["task_id"]):
            return {"kind": "UNRESOLVED", "reason": "provider receipt does not bind this intent, request and provider"}
        return {"kind": "ADOPTED", "remote_id": evidence["task_id"],
                "evidence": {**evidence, "receipt_url": url}}

    @activity.defn
    def observe(self, trial: dict) -> dict:
        try:
            task = tes_client(trial["provider"]).get("FULL", trial["remote_id"])
            state = task.status.strip('"').upper()
        except Exception as error:
            return {"error": type(error).__name__}
        cleanup, physical = "UNKNOWN", None
        try:
            url = trial["provider"]["url"] + "/control/v1/tasks/" + quote(trial["remote_id"], safe="") + "/evidence"
            physical = read_json(url)
            if physical.get("physical_status") == "EXITED":
                cleanup = "CONFIRMED_EXITED"
            elif physical.get("physical_status") == "RUNNING":
                cleanup = "WORKER_RUNNING"
        except (URLError, TimeoutError, ValueError, AttributeError):
            pass
        return {"protocol_state": state, "cleanup": cleanup, "physical_evidence": physical}

    @activity.defn
    def collect_result(self, trial: dict) -> dict:
        reference = trial["provider"]["url"] + "/control/v1/tasks/" + quote(trial["remote_id"], safe="") + "/metrics"
        try:
            metrics = read_json(reference)
        except HTTPError as error:
            if error.code == 404:
                return {"kind": "MISSING_METRICS", "reference": reference}
            return {"kind": "METRICS_UNAVAILABLE", "reference": reference, "http_status": error.code}
        except (URLError, TimeoutError):
            return {"kind": "METRICS_UNAVAILABLE", "reference": reference}
        except (ValueError, UnicodeError):
            return {"kind": "INVALID_METRICS", "reference": reference}
        return {**metric_outcome(metrics, trial), "reference": reference}

    @activity.defn
    def cancel(self, trial: dict) -> dict:
        try:
            from GA4GH import Configuration, Task, Transport
            task = Task(trial["remote_id"], Transport(Configuration(trial["provider"]["url"] + "/v1")))
            task.cancel()
            return {"request": "ACKNOWLEDGED", "terminal": False}
        except Exception as error:
            return {"request": "FAILED", "reason": type(error).__name__, "terminal": False}
