"""Core client behaviour against the recording mock server in `conftest.py`."""

import json

import pytest

from GA4GH import Configuration, ServiceInfo, Transport


def test_get_service_info(server, config):
    result = ServiceInfo(config).get_service_info()

    request = server.last()
    assert (request["method"], request["path"]) == ("GET", "/api/service-info")
    assert result.startswith("Service: ")
    assert "org.example.tes" in result
    assert "Example TES" in result


def test_get_service_info_failure(server):
    service_info = ServiceInfo(Configuration(f"{server.root}/missing"))

    with pytest.raises(RuntimeError, match="Failed to get service info"):
        service_info.get_service_info()
    assert server.last()["path"] == "/missing/service-info"


@pytest.mark.parametrize(
    "method, args, path, body, expected",
    [
        ("get", ("tasks/task-1",), "/api/tasks/task-1", None, {"id": "task-1", "state": "COMPLETE"}),
        ("post", ("runs", '{"data": "test"}'), "/api/runs", {"data": "test"}, {"run_id": "run-1"}),
        ("put", ("plugins/p1", '{"data": "update"}'), "/api/plugins/p1", {"data": "update"}, {"updated": True}),
        ("delete", ("plugins/p1",), "/api/plugins/p1", None, {"deleted": True}),
    ],
)
def test_transport_methods(server, config, method, args, path, body, expected):
    result = json.loads(getattr(Transport(config), method)(*args))

    request = server.last()
    assert (request["method"], request["path"]) == (method.upper(), path)
    if body is None:
        assert request["body"] == b""
    else:
        assert json.loads(request["body"]) == body
    assert expected.items() <= result.items()


def test_configuration():
    # Test creating a Configuration instance
    base_path = "http://example.com/api"
    config = Configuration(base_path)

    # Verify the base path was set correctly
    assert config.get_base_path() == base_path

    # Test setting a new base path
    new_base_path = "http://example.com/new-api"
    config.set_base_path(new_base_path)

    # Verify the new base path
    assert config.get_base_path() == new_base_path
