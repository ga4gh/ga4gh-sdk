"""Static credentials: `Configuration.set_basic_auth()` and `set_access_token()`."""

import pytest

from ga4gh_sdk import Configuration, ServiceInfo, TES, Transport


def headers_of_last(server):
    return {name.lower(): value for name, value in server.last()["headers"].items()}


def test_basic_auth_is_sent(server):
    config = Configuration(f"{server.root}/basic")
    config.set_basic_auth("alice", "secret")
    assert "Example TES" in ServiceInfo(config).get_service_info()
    assert TES(config).get("FULL", "task-1").id == "task-1"
    assert Transport(config).get("tasks/task-1")


def test_access_token_is_sent(server):
    config = Configuration(f"{server.root}/auth")
    config.set_access_token("good")
    assert TES(config).get("FULL", "task-1").id == "task-1"
    assert headers_of_last(server)["authorization"] == "Bearer good"


def test_wrong_credentials_get_401(server):
    config = Configuration(f"{server.root}/basic")
    config.set_basic_auth("alice", "wrong")
    with pytest.raises(RuntimeError, match="401"):
        Transport(config).get("tasks/task-1")


def test_credentials_can_be_removed(server):
    config = Configuration(f"{server.root}/api")
    config.set_basic_auth("alice", "secret")
    config.set_basic_auth(None)
    config.set_access_token("good")
    config.set_access_token(None)
    assert Transport(config).get("tasks/task-1")
    assert "authorization" not in headers_of_last(server)


def test_password_needs_a_username():
    config = Configuration("https://example.org/ga4gh/tes/v1")
    with pytest.raises(ValueError, match="password needs a username"):
        config.set_basic_auth(None, "secret")


def test_conflicting_credentials_send_nothing(server):
    config = Configuration(f"{server.root}/basic")
    config.set_basic_auth("alice", "SENTINEL_PASSWORD")
    config.set_access_token("SENTINEL_TOKEN")
    before = len(server.paths())
    with pytest.raises(RuntimeError) as error:
        Transport(config).get("tasks/task-1")
    assert "basic_auth, oauth_access_token" in str(error.value)
    assert "SENTINEL" not in str(error.value)
    assert server.paths()[before:] == []


def test_static_credential_conflicts_with_a_token_provider(server):
    config = Configuration(f"{server.root}/auth")
    config.set_access_token("SENTINEL_TOKEN")
    config.set_token_provider(lambda audience: "good")
    with pytest.raises(RuntimeError, match="token provider") as error:
        Transport(config).get("tasks/task-1")
    assert "SENTINEL" not in str(error.value)
