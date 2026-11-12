"""Bearer tokens from a caller-supplied provider: `Configuration.set_token_provider()`."""

import asyncio

import pytest

from ga4gh_sdk import AsyncTES, Configuration, ServiceInfo, TES, Transport


class Tokens:
    """A token provider that gives the next of `tokens` and records the audiences."""

    def __init__(self, *tokens):
        self.tokens = list(tokens)
        self.audiences = []

    def __call__(self, audience):
        self.audiences.append(audience)
        return self.tokens.pop(0)


def auth_config(server, provider, **kwargs):
    config = Configuration(f"{server.root}/auth")
    config.set_token_provider(provider, **kwargs)
    return config


def requests_since(server, before):
    """The paths of the requests that the server received after the first `before`."""
    return server.paths()[before:]


def test_clients_send_the_token(server):
    provider = Tokens("good", "good", "good", "good")
    config = auth_config(server, provider)
    assert "Example TES" in ServiceInfo(config).get_service_info()
    tes = TES(config)
    assert tes.get("FULL", "task-1").id == "task-1"
    headers = {name.lower(): value for name, value in server.last()["headers"].items()}
    assert headers["authorization"] == "Bearer good"
    assert Transport(config).get("tasks/task-1")
    assert provider.audiences == [f"{server.root}/auth"] * 4


def test_async_clients_send_the_token(server):
    provider = Tokens("good", "good")
    config = auth_config(server, provider)

    async def main():
        tes = AsyncTES(config)
        return await tes.get("FULL", "task-1")

    assert asyncio.run(main()).id == "task-1"
    assert provider.audiences == [f"{server.root}/auth"] * 2


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
        raise ValueError("no session")

    before = len(server.paths())
    with pytest.raises(RuntimeError, match="The token provider failed .*ValueError: no session"):
        Transport(auth_config(server, failing)).get("tasks/task-1")
    with pytest.raises(RuntimeError, match="must return a str"):
        Transport(auth_config(server, lambda audience: 3)).get("tasks/task-1")
    with pytest.raises(RuntimeError, match="non-negative number of seconds"):
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
