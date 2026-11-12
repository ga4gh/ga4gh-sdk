"""Configured credentials reach Python service and transport requests."""

import base64

import pytest

from ga4gh_sdk import Configuration, ServiceInfo, TES, Transport, WES


def header(server):
    return next(
        (
            value
            for name, value in server.last()["headers"].items()
            if name.lower() == "authorization"
        ),
        None,
    )


def test_basic_is_used_for_discovery_and_submission(server):
    config = Configuration(server.base)
    config.set_basic_auth("alice", "pw")
    tes = TES(config)
    assert header(server) == "Basic " + base64.b64encode(b"alice:pw").decode()
    tes.create({"executors": [{"image": "alpine", "command": ["true"]}]})
    assert header(server) == "Basic " + base64.b64encode(b"alice:pw").decode()


def test_bearer_is_used_by_service_info_wes_and_transport(server):
    config = Configuration(server.base)
    config.set_bearer_access_token("test-bearer")
    ServiceInfo(config).get_service_info()
    assert header(server) == "Bearer test-bearer"
    Transport(config).get("tasks/task-1")
    assert header(server) == "Bearer test-bearer"

    wes_config = Configuration(f"{server.root}/wes")
    wes_config.set_oauth_access_token("test-oauth")
    WES(wes_config)
    assert header(server) == "Bearer test-oauth"


def test_conflicts_and_invalid_tokens_fail_before_network(server):
    config = Configuration(server.base)
    config.set_basic_auth("alice", "secret-password")
    config.set_oauth_access_token("secret-token")
    before = server.paths()
    with pytest.raises(RuntimeError, match="Conflicting authentication sources") as error:
        ServiceInfo(config).get_service_info()
    assert "secret-password" not in str(error.value)
    assert "secret-token" not in str(error.value)
    assert server.paths() == before

    config.clear_auth()
    config.set_bearer_access_token("bad\nsecret")
    with pytest.raises(RuntimeError, match="Invalid bearer token"):
        Transport(config).get("tasks")
    assert server.paths() == before


def test_unauthorized_response_preserves_status_without_printing_token(server):
    config = Configuration(server.base)
    config.set_bearer_access_token("secret-token")
    before = server.paths().count("/api/auth-failure")
    with pytest.raises(RuntimeError, match="401") as error:
        Transport(config).get("auth-failure")
    assert "secret-token" not in str(error.value)
    assert header(server) == "Bearer secret-token"
    assert server.paths().count("/api/auth-failure") == before + 1


def test_server_echoing_token_does_not_expose_it_in_python_exception(server):
    config = Configuration(server.base)
    config.set_bearer_access_token("SENTINEL_TOKEN")
    with pytest.raises(RuntimeError) as raised:
        Transport(config).get("auth-echo")
    message = str(raised.value)
    assert "401" in message
    assert "SENTINEL_TOKEN" not in message
    assert "Authorization" not in message
    assert header(server) == "Bearer SENTINEL_TOKEN"
