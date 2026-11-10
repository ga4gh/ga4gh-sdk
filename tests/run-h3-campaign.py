#!/usr/bin/env python3
"""Focused contracts; --integration also runs real Temporal/CPU restart evidence."""
import argparse
import asyncio
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
from urllib.error import HTTPError, URLError

ROOT = Path(__file__).resolve().parents[1]
EXAMPLE = ROOT / "examples" / "adaptive-campaign"
sys.path.insert(0, str(EXAMPLE))
from model import (POLICY_VERSION, apply_observation, choose_parameter,
                   metric_outcome, validate_config)


class CampaignContracts(unittest.TestCase):
    def test_bounds_include_unknown_reservations(self):
        config = dict(max_trials=8, max_concurrency=2, deadline_seconds=30,
                      providers=[dict(name="a", url="http://127.0.0.1:1"),
                                 dict(name="b", url="http://127.0.0.1:2")],
                      steps=80, delay_ms=10)
        validate_config(config)
        for field, value in [("max_trials", 9), ("max_concurrency", 3),
                             ("max_concurrency", True), ("deadline_seconds", 0)]:
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                validate_config({**config, field: value})

    def test_metrics_are_typed_and_not_zero_on_error(self):
        trial = dict(parameter=2, steps=80)
        self.assertEqual(metric_outcome(None, trial)["kind"], "MISSING_METRICS")
        self.assertEqual(metric_outcome({"schema": "wrong"}, trial)["kind"], "INVALID_METRICS")
        metric = dict(schema="h3-cpu/v1", parameter=2, steps=80, score=-1,
                      checksum="a" * 64)
        self.assertEqual(metric_outcome(metric, trial)["score"], -1)
        for key, value in [("score", False), ("score", 0.0), ("parameter", 9),
                           ("steps", 81), ("checksum", "bad")]:
            self.assertEqual(metric_outcome({**metric, key: value}, trial)["kind"],
                             "INVALID_METRICS")
        self.assertEqual(metric_outcome({**metric, "score": 0}, {**trial, "parameter": 2})["score"], 0)

    def test_failed_observation_and_cancel_request_retain_reservation(self):
        trial = dict(state="ACKNOWLEDGED", reservation=True, protocol_state="RUNNING",
                     cleanup="UNKNOWN", cancel_requested=True)
        apply_observation(trial, {"error": "network unavailable"})
        self.assertTrue(trial["reservation"])
        self.assertEqual(trial["protocol_state"], "RUNNING")
        apply_observation(trial, {"protocol_state": "CANCELING", "cleanup": "UNKNOWN"})
        self.assertTrue(trial["reservation"])
        apply_observation(trial, {"protocol_state": "CANCELED", "cleanup": "UNKNOWN"})
        self.assertFalse(trial["reservation"])
        self.assertEqual(trial["cleanup"], "UNKNOWN")

    def test_pinned_policy_uses_result_events(self):
        trials = [dict(trial_id="t0", parameter=0), dict(trial_id="t1", parameter=6)]
        result = dict(event_id="t0:run0", trial_id="t0", parameter=0, score=-9)
        parameter, evidence = choose_parameter(trials, [result])
        self.assertEqual(parameter, 1)
        self.assertEqual(evidence["policy"], POLICY_VERSION)
        self.assertEqual(evidence["result_events"], ["t0:run0"])
        parameter2, _ = choose_parameter(trials, [{**result, "parameter": 6, "score": 0}])
        self.assertEqual(parameter2, 5)



@unittest.skipUnless(importlib.util.find_spec("temporalio"), "optional Temporal dependency not installed")
class AdapterContracts(unittest.TestCase):
    def trial(self):
        from model import recipe_digest
        return dict(trial_id="test:trial:0", intent_id="test:trial:0",
                    provider=dict(name="alpha", url="http://127.0.0.1:1"),
                    recipe_digest=recipe_digest(), parameter=2, steps=80, delay_ms=10,
                    remote_id="remote-1", deadline="2000-01-01T00:00:00+00:00")

    def test_expired_dispatch_does_not_call_tes(self):
        from activities import Activities
        with patch("activities.tes_client") as client:
            self.assertEqual(Activities().dispatch(self.trial())["kind"], "NOT_DISPATCHED_DEADLINE")
            client.assert_not_called()

    def test_adoption_requires_provider_binding_to_entire_request(self):
        from activities import Activities
        from model import digest, task_request
        trial = self.trial()
        receipt = dict(intent=trial["intent_id"], task_fingerprint=digest(task_request(trial)),
                       provider="alpha", task_id="remote-1")
        with patch("activities.read_json", return_value=receipt):
            self.assertEqual(Activities().reconcile(trial)["kind"], "ADOPTED")
        for field, value in [("intent", "other-intent"), ("task_fingerprint", "bad"),
                             ("provider", "beta"), ("task_id", "")]:
            with self.subTest(field=field), patch("activities.read_json", return_value={**receipt, field: value}):
                result = Activities().reconcile(trial)
                self.assertEqual(result["kind"], "UNRESOLVED")
                self.assertNotIn("remote_id", result)
        for status in (404, 501):
            with patch("activities.read_json", side_effect=HTTPError("http://test", status, "absent", {}, None)):
                self.assertEqual(Activities().reconcile(trial)["kind"], "UNRESOLVED")

    def test_metric_transport_missing_and_invalid_outcomes_are_distinct(self):
        from activities import Activities
        failures = [(HTTPError("http://test", 404, "absent", {}, None), "MISSING_METRICS"),
                    (HTTPError("http://test", 503, "unavailable", {}, None), "METRICS_UNAVAILABLE"),
                    (URLError("offline"), "METRICS_UNAVAILABLE"),
                    (ValueError("invalid JSON"), "INVALID_METRICS")]
        for error, kind in failures:
            with self.subTest(kind=kind), patch("activities.read_json", side_effect=error):
                result = Activities().collect_result(self.trial())
                self.assertEqual(result["kind"], kind)
                self.assertNotIn("score", result)


async def run_integration(args):
    from integration import demonstrate
    await demonstrate(args.providers, args.evidence)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--integration", action="store_true")
    parser.add_argument("--providers", type=Path,
                        default=ROOT / "examples" / "portable-execution" / "provider.py")
    parser.add_argument("--evidence", type=Path)
    args = parser.parse_args()
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    if not unittest.TextTestRunner(verbosity=2).run(suite).wasSuccessful():
        sys.exit(1)
    if args.integration:
        asyncio.run(run_integration(args))
