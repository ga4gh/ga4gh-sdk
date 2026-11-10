"""TES views are validated before a request and sent with their TES spelling."""

import pytest

from GA4GH import ListTasksParams, TES


@pytest.mark.parametrize(
    ("view", "expected"),
    [("MINIMAL", "MINIMAL"), ("basic", "BASIC"), (" Full ", "FULL")],
)
def test_get_sends_valid_view(server, config, view, expected):
    task = TES(config).get(view, "task-1")
    assert task.id == "task-1"
    assert server.last()["query"] == {"view": [expected]}


@pytest.mark.parametrize("view", ["EVERYTHING", "", "FULL VIEW"])
def test_get_rejects_invalid_view_without_request(server, config, view):
    client = TES(config)
    discovery = server.last()
    with pytest.raises(ValueError, match="MINIMAL, BASIC, or FULL"):
        client.get(view, "task-1")
    assert server.last() == discovery


def test_list_rejects_invalid_views_before_request_and_sends_valid_values(server, config):
    client = TES(config)
    discovery = server.last()

    for view in ("EVERYTHING", "", "FULL VIEW"):
        with pytest.raises(ValueError, match="MINIMAL, BASIC, or FULL"):
            ListTasksParams(view=view)
        assert server.last() == discovery

    for view, expected in (("MINIMAL", "MINIMAL"), ("basic", "BASIC"), (" Full ", "FULL")):
        params = ListTasksParams(view=view)
        assert params.view == expected
        client.list(params)
        assert server.last()["query"] == {"view": [expected]}
