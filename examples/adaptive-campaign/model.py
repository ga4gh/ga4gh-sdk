"""Pinned, deterministic campaign contract. No network, clock, or filesystem I/O."""
POLICY_VERSION = "integer-neighbors/v1"
TERMINAL = {"COMPLETE", "CANCELED", "EXECUTOR_ERROR", "SYSTEM_ERROR", "PREEMPTED"}
PROTOCOL_STATES = TERMINAL | {"UNKNOWN", "QUEUED", "INITIALIZING", "RUNNING", "PAUSED", "CANCELING"}


def validate_config(config):
    for name, lo, hi in [("max_trials", 1, 8), ("max_concurrency", 1, 2),
                         ("steps", 10, 1000), ("delay_ms", 0, 100)]:
        value = config.get(name)
        if type(value) is not int or not lo <= value <= hi:
            raise ValueError(f"{name} must be an integer in {lo}..{hi}")
    seconds = config.get("deadline_seconds")
    if type(seconds) not in (int, float) or not 0 < seconds <= 3600:
        raise ValueError("deadline_seconds must be finite and in (0, 3600]")
    providers = config.get("providers", [])
    if len(providers) != 2 or len({p["name"] for p in providers}) != 2:
        raise ValueError("configure exactly two uniquely named TES providers")
    # This demonstrator only understands the documented trusted-loopback contract.
    from urllib.parse import urlsplit
    for provider in providers:
        url = urlsplit(provider["url"])
        if (url.scheme != "http" or url.hostname not in ("127.0.0.1", "localhost", "::1")
                or url.username or url.password or url.query or url.fragment):
            raise ValueError("this adapter requires credential-free loopback HTTP providers")


def choose_parameter(trials, results):
    """Maximize the observed integer score; deterministic ties favor earlier trials."""
    used = {t["parameter"] for t in trials}
    evidence = {"policy": POLICY_VERSION, "result_events": [r["event_id"] for r in results]}
    if not trials:
        return 0, {**evidence, "reason": "initial lower point"}
    if len(trials) == 1:
        return 6, {**evidence, "reason": "initial upper point"}
    if results:
        best = max(results, key=lambda r: r["score"])
        center = best["parameter"]
        candidates = sorted(range(9), key=lambda p: (abs(p - center), p))
        evidence.update(best_event=best["event_id"], best_score=best["score"],
                        reason="nearest untested integer to observed best")
    else:
        candidates = list(range(9))
        evidence["reason"] = "no valid metrics; deterministic untested fallback"
    return next(p for p in candidates if p not in used), evidence


def metric_outcome(metrics, trial):
    if metrics is None:
        return {"kind": "MISSING_METRICS"}
    expected = {"schema", "parameter", "steps", "score", "checksum"}
    if (not isinstance(metrics, dict) or not expected <= metrics.keys()
            or metrics["schema"] != "h3-cpu/v1"
            or any(type(metrics[k]) is not int for k in ("parameter", "steps", "score"))
            or metrics["parameter"] != trial["parameter"]
            or metrics["steps"] != trial["steps"]
            or not isinstance(metrics["checksum"], str)
            or len(metrics["checksum"]) != 64
            or any(c not in "0123456789abcdef" for c in metrics["checksum"])):
        return {"kind": "INVALID_METRICS", "detail": "h3-cpu/v1 schema or trial identity mismatch"}
    return {"kind": "VALID_METRICS", **metrics}


def apply_observation(trial, observation):
    """Only a recognized terminal protocol state releases a known-run slot."""
    if "error" in observation:
        trial["observation_error"] = observation["error"]
        return
    state = observation.get("protocol_state")
    if state not in PROTOCOL_STATES:
        trial["observation_error"] = "unrecognized TES state"
        return
    trial.pop("observation_error", None)
    trial["protocol_state"] = state
    trial["cleanup"] = observation.get("cleanup", "UNKNOWN")
    if state in TERMINAL:
        trial["reservation"] = False
        trial["state"] = "TERMINAL"


def task_request(trial):
    return {"name": "public-cpu-trial", "executors": [{"image": "ga4gh-h3-cpu:v1",
            "command": ["h3-cpu", "--steps", str(trial["steps"]), "--parameter",
                        str(trial["parameter"]), "--delay-ms", str(trial["delay_ms"])]}],
            "inputs": [{"name": "public-numbers-v1", "url": "h3://inputs/public-numbers-v1",
                        "path": "/inputs/numbers.bin", "type": "FILE"}],
            "resources": {"cpu_cores": 1, "ram_gb": .1},
            "tags": {"h3.intent": trial["intent_id"], "h3.recipe": trial["recipe_digest"]}}


def digest(value):
    import hashlib
    import json
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def recipe_digest():
    return digest({"recipe": "public-sha256-lcg-shifted-quadratic/v1",
                   "input": list(range(256)), "image": "ga4gh-h3-cpu:v1",
                   "metric_schema": "h3-cpu/v1", "policy": POLICY_VERSION})
