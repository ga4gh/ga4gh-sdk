"""Query strings that the Python bindings actually send.

The cases match `lib/tests/query_encoding.rs`, which checks the Rust transport and pager.
"""

import asyncio

import pytest

from ga4gh_sdk import AsyncTES, Configuration, ListTasksParams, TES, Transport, WES

from .conftest import MockServer

SHARED_CASES = [
    pytest.param(
        {"q": "a b&c=d/é+%?#", "a b": "c"},
        "a+b=c&q=a+b%26c%3Dd%2F%C3%A9%2B%25%3F%23",
        id="reserved",
    ),
    pytest.param({"b": True, "f": 1.5, "n": -10, "s": ""}, "b=true&f=1.5&n=-10&s=", id="scalars"),
    pytest.param({"a": None, "e": [], "k": "v"}, "k=v", id="omitted"),
    pytest.param(
        {"tag_key": ["a", "b"], "tag_value": ["x", ""], "page_size": 2},
        "page_size=2&tag_key=a&tag_key=b&tag_value=x&tag_value=",
        id="repeated",
    ),
]


@pytest.mark.parametrize(("params", "query"), SHARED_CASES)
def test_transport_sends_shared_cases(
    server: MockServer, config: Configuration, params: dict, query: str
) -> None:
    """`Transport.get()` encodes parameters like the Rust transport."""
    Transport(config).get("tasks", params)
    assert server.last()["raw_query"] == query


@pytest.mark.parametrize(
    "params",
    [
        pytest.param({"filter": {"a": 1}}, id="object"),
        pytest.param({"k": [["a"]]}, id="nested-list"),
        pytest.param({"k": [{"a": 1}]}, id="object-item"),
        pytest.param({"k": ["x", None, "y"]}, id="null-item"),
    ],
)
def test_transport_rejects_unsupported_values_before_sending(
    server: MockServer, config: Configuration, params: dict
) -> None:
    """`Transport.get()` raises `ValueError` and sends nothing for other values.

    A `None` item is an error too: TES pairs `tag_key` and `tag_value` items by position.
    """
    before = len(server.paths())
    with pytest.raises(ValueError, match="^Invalid params: query parameter '(filter|k)' must be"):
        Transport(config).get("tasks", params)
    assert len(server.paths()) == before


def test_transport_appends_to_a_query_in_the_endpoint(
    server: MockServer, config: Configuration
) -> None:
    """Parameters follow a query that is already part of the endpoint."""
    Transport(config).get("tasks?view=FULL", {"tag_key": ["a b"]})
    assert server.last()["raw_query"] == "view=FULL&tag_key=a+b"


TAG_PARAMS = {
    "tag_key": ["team a", "x/y"],
    "tag_value": ["", "1&2"],
    "page_size": 2,
}
TAG_QUERY = "page_size=2&tag_key=team+a&tag_key=x%2Fy&tag_value=&tag_value=1%262"


def test_tes_list_repeats_tags(server: MockServer, config: Configuration) -> None:
    """`TES.list()` sends one parameter per tag key and value, in order."""
    TES(config).list(ListTasksParams(**TAG_PARAMS))
    assert server.last()["raw_query"] == TAG_QUERY


def test_async_tes_list_repeats_tags(server: MockServer, config: Configuration) -> None:
    """`AsyncTES.list()` sends the same query as `TES.list()`."""

    async def main() -> None:
        await AsyncTES(config).list(ListTasksParams(**TAG_PARAMS))

    asyncio.run(main())
    assert server.last()["raw_query"] == TAG_QUERY


def test_wes_list_encodes_the_page_token(server: MockServer, wes_config: Configuration) -> None:
    """`WES.list()` encodes an opaque page token like other parameters."""
    WES(wes_config).list(page_size=5, page_token="t/+ %")
    assert server.last()["raw_query"] == "page_size=5&page_token=t%2F%2B+%25"
