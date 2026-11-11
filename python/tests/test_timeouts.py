"""Time limits of requests: `Configuration.set_timeouts()` and the file's `timeouts`."""

import json
import os
import time

import pytest

from GA4GH import TES, Configuration

DEFAULTS = {"connect": 10.0, "request": 60.0, "discovery": 10.0, "submission": 60.0}


def test_defaults():
    assert Configuration("https://example.org").get_timeouts() == DEFAULTS


def test_set_timeouts_changes_only_the_given_limits():
    config = Configuration("https://example.org")
    config.set_timeouts(discovery=30, submission=0.5)
    assert config.get_timeouts() == {**DEFAULTS, "discovery": 30.0, "submission": 0.5}
    config.set_timeouts(connect=2)
    assert config.get_timeouts() == {
        **DEFAULTS,
        "connect": 2.0,
        "discovery": 30.0,
        "submission": 0.5,
    }


@pytest.mark.parametrize("seconds", [0, -1, float("nan"), float("inf"), 1e300])
def test_set_timeouts_rejects_invalid_values(seconds):
    config = Configuration("https://example.org")
    with pytest.raises(ValueError, match="request must be a positive number of seconds"):
        config.set_timeouts(connect=5, request=seconds)
    # A rejected call changes nothing.
    assert config.get_timeouts() == DEFAULTS


def test_set_timeouts_takes_keywords_only():
    with pytest.raises(TypeError):
        Configuration("https://example.org").set_timeouts(5)


@pytest.mark.skipif(os.name != "posix", reason="HOME controls the config path on Unix")
def test_from_file_reads_timeouts(tmp_path, monkeypatch):
    config_dir = tmp_path / ".ga4gh-cli"
    config_dir.mkdir()
    (config_dir / "config.json").write_text(
        json.dumps(
            {
                "TES": {
                    "base_path": "https://example.org/ga4gh/tes/v1",
                    "timeouts": {"discovery": 30, "submission": 300},
                }
            }
        )
    )
    monkeypatch.setenv("HOME", str(tmp_path))
    config = Configuration("https://wrong.example")
    config.from_file("TES")
    assert config.get_timeouts() == {**DEFAULTS, "discovery": 30.0, "submission": 300.0}
    assert config.get_extra() == {}


def test_discovery_timeout_limits_the_service_check(server):
    config = Configuration(f"{server.root}/slow-2")
    config.set_timeouts(discovery=0.2)
    start = time.monotonic()
    with pytest.raises(RuntimeError, match="Failed to create TES instance"):
        TES(config)
    assert time.monotonic() - start < 1.5

    # A discovery limit longer than the request limit replaces it.
    config = Configuration(f"{server.root}/slow-0.5")
    config.set_timeouts(request=0.1, discovery=5)
    assert TES(config).config.get_timeouts()["discovery"] == 5.0
