"""Bearer tokens from a caller-supplied provider: `Configuration.set_token_provider()`."""

import asyncio
import json
import os

import pytest

from ga4gh_sdk import AsyncTES, Configuration, ServiceInfo, TES, Transport


class Tokens:
    """A token provider that gives the next of `tokens`, then the last one, and
    records the audiences."""

    def __init__(self, *tokens):
        self.tokens = list(tokens)
        self.audiences = []

    def __call__(self, audience):
        self.audiences.append(audience)
        return self.tokens.pop(0) if len(self.tokens) > 1 else self.tokens[0]


def auth_config(server, provider, **kwargs):
    config = Configuration(f"{server.root}/auth")
    config.set_token_provider(provider, **kwargs)
    return config


def requests_since(server, before):
    """The paths of the requests that the server received after the first `before`."""
    return server.paths()[before:]


def test_clients_send_the_token(server):
    provider = Tokens("good")
    config = auth_config(server, provider)
    assert "Example TES" in ServiceInfo(config).get_service_info()
    tes = TES(config)
    assert tes.get("FULL", "task-1").id == "task-1"
    headers = {name.lower(): value for name, value in server.last()["headers"].items()}
    assert headers["authorization"] == "Bearer good"
    assert Transport(config).get("tasks/task-1")
    assert set(provider.audiences) == {f"{server.root}/auth"}


def test_async_clients_send_the_token(server):
    provider = Tokens("good")
    config = auth_config(server, provider)

    async def main():
        tes = AsyncTES(config)
        return await tes.get("FULL", "task-1")

    assert asyncio.run(main()).id == "task-1"
    assert set(provider.audiences) == {f"{server.root}/auth"}


def test_get_is_sent_once_more_after_401(server):
    provider = Tokens("stale", "good")
    before = len(server.paths())
    assert Transport(auth_config(server, provider)).get("tasks/task-1")
    assert requests_since(server, before) == ["/auth/tasks/task-1"] * 2
    assert len(provider.audiences) == 2


def test_submission_is_not_sent_again_after_401(server):
    provider = Tokens("stale", "good")
    before = len(server.paths())
    with pytest.raises(RuntimeError, match="401"):
        Transport(auth_config(server, provider)).post("tasks", {"executors": []})
    assert requests_since(server, before) == ["/auth/tasks"]
    assert len(provider.audiences) == 1


def test_token_with_reuse_time_is_reused(server):
    provider = Tokens(("good", 60.0))
    transport = Transport(auth_config(server, provider, audience="tes"))
    transport.get("tasks/task-1")
    transport.get("tasks/task-1")
    assert provider.audiences == ["tes"]


def test_provider_errors_send_nothing(server):
    def failing(audience):
        raise ValueError("no session for SENTINEL_TOKEN")

    before = len(server.paths())
    with pytest.raises(RuntimeError, match="The token provider failed") as raised:
        Transport(auth_config(server, failing)).get("tasks/task-1")
    assert "SENTINEL_TOKEN" not in str(raised.value)
    with pytest.raises(RuntimeError, match="The token provider failed"):
        Transport(auth_config(server, lambda audience: 3)).get("tasks/task-1")
    with pytest.raises(RuntimeError, match="The token provider failed"):
        Transport(auth_config(server, lambda audience: ("good", -1.0))).get("tasks/task-1")
    assert requests_since(server, before) == []


def test_invalid_tokens_are_not_shown(server):
    with pytest.raises(RuntimeError) as error:
        Transport(auth_config(server, lambda audience: "SENTINEL\nTOKEN")).get("tasks/task-1")
    assert "SENTINEL" not in str(error.value)
    assert "valid HTTP header value" in str(error.value)


def test_set_token_provider_arguments(server):
    config = Configuration(f"{server.root}/auth")
    with pytest.raises(TypeError, match="callable"):
        config.set_token_provider("good")
    with pytest.raises(ValueError, match="needs a token provider"):
        config.set_token_provider(None, audience="tes")
    config.set_token_provider(lambda audience: "good")
    config.set_token_provider(None)
    with pytest.raises(RuntimeError, match="401"):
        Transport(config).get("tasks/task-1")


def test_tes_create_is_not_sent_again_after_401(server):
    provider = Tokens("good", "stale", "good")
    tes = TES(auth_config(server, provider))
    before = len(server.paths())
    with pytest.raises(RuntimeError, match="401"):
        tes.create({"executors": [{"image": "alpine", "command": ["true"]}]})
    assert requests_since(server, before) == ["/auth/tasks"]
    # The next submission gets a new token.
    assert tes.create({"executors": [{"image": "alpine", "command": ["true"]}]}).id == "task-1"


def test_clients_keep_the_provider_they_were_built_with(server):
    provider = Tokens("good")
    config = auth_config(server, provider)
    transport = Transport(config)
    config.set_token_provider(None)
    assert transport.get("tasks/task-1")
    with pytest.raises(RuntimeError, match="401"):
        Transport(config).get("tasks/task-1")


def test_clear_auth_removes_provider_for_new_clients(server):
    config = auth_config(server, Tokens("good"), audience="tes")
    config.clear_auth()
    with pytest.raises(RuntimeError, match="401"):
        Transport(config).get("tasks/task-1")


@pytest.mark.skipif(os.name != "posix", reason="HOME controls the config path on Unix")
def test_file_credentials_and_from_file(server, tmp_path, monkeypatch):
    config_dir = tmp_path / ".ga4gh-cli"
    config_dir.mkdir()
    section = {"base_path": f"{server.root}/auth", "oauth_access_token": "SENTINEL_TOKEN"}
    (config_dir / "config.json").write_text(json.dumps({"TES": section}))
    monkeypatch.setenv("HOME", str(tmp_path))

    config = Configuration("https://example.org")
    config.from_file("TES")
    config.set_token_provider(Tokens("good"))
    before = len(server.paths())
    with pytest.raises(RuntimeError, match="can't be combined") as error:
        Transport(config).get("tasks/task-1")
    assert "SENTINEL" not in str(error.value)
    assert requests_since(server, before) == []

    # `from_file()` replaces the provider, too.
    config.from_file("TES")
    with pytest.raises(RuntimeError, match="401"):
        Transport(config).get("tasks/task-1")
