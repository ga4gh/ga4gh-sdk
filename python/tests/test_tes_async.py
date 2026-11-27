"""The asyncio TES API: awaitable calls that run concurrently and can be cancelled."""

import asyncio
import contextvars
import inspect
import json
import time
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any, TypeVar

import pytest

from ga4gh_sdk import (
    AsyncTask,
    AsyncTES,
    Configuration,
    ListTasksParams,
    ListTasksResponse,
    TES,
    TesState,
    TesTask,
    Transport,
)

from .conftest import MockServer
from .mock_server import TASK

NEW_TASK = {
    "name": "hello",
    "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
    "extension_field": {"kept": True},
}

T = TypeVar("T")

# The tests run coroutines with `asyncio.run()`, so they need no async plugin for pytest.


def run(coroutine: Callable[[], Awaitable[T]]) -> T:
    """Runs a coroutine function on a new event loop, failing if it takes over 20 seconds.

    Args:
        coroutine: The coroutine function to run.

    Returns:
        The coroutine's result.
    """

    async def bounded() -> T:
        return await asyncio.wait_for(coroutine(), timeout=20)

    return asyncio.run(bounded())


class Heartbeat:
    """Counts the ticks of a task that sleeps 10 ms at a time, to show the loop runs."""

    def __init__(self) -> None:
        """Starts counting at zero; `start()` creates the task."""
        self.ticks = 0
        self.task: asyncio.Task[None] | None = None

    async def _beat(self) -> None:
        """Ticks until cancelled."""
        while True:
            await asyncio.sleep(0.01)
            self.ticks += 1

    def start(self) -> None:
        """Starts ticking on the running loop."""
        self.task = asyncio.get_running_loop().create_task(self._beat())

    def stop(self) -> None:
        """Stops ticking."""
        assert self.task is not None
        self.task.cancel()


def test_connect_checks_the_service(server: MockServer, config: Configuration) -> None:
    """`connect()` reads the service info and returns a client."""

    async def main() -> AsyncTES:
        return await AsyncTES.connect(config)

    tes = run(main)
    assert type(tes) is AsyncTES
    assert server.last()["path"] == "/api/service-info"
    assert tes.config.get_base_path() == config.get_base_path()


def test_connect_rejects_a_root_without_tes(server: MockServer) -> None:
    """`connect()` raises `RuntimeError` when the root isn't a TES API root."""

    async def main() -> AsyncTES:
        return await AsyncTES.connect(Configuration(server.root))

    with pytest.raises(RuntimeError, match="Failed to create TES instance.*404"):
        run(main)


def test_constructor_does_not_contact_the_server() -> None:
    """`AsyncTES(config)` works even if nothing listens at the root."""
    tes = AsyncTES(Configuration("http://127.0.0.1:9/api"))
    assert isinstance(tes.transport, Transport)


def test_first_call_checks_the_service_once(server: MockServer, config: Configuration) -> None:
    """Without `connect()`, the first call reads the service info; later calls don't."""

    async def main() -> None:
        tes = AsyncTES(config)
        before = len(server.paths())
        await tes.get("FULL", "task-1")
        await tes.list()
        assert server.paths()[before:] == ["/api/service-info", "/api/tasks/task-1", "/api/tasks"]

    run(main)


def test_first_call_fails_for_a_root_without_tes(server: MockServer) -> None:
    """Without `connect()`, a failed check raises from the first call, as for `TES(config)`."""

    async def main() -> None:
        tes = AsyncTES(Configuration(server.root))
        with pytest.raises(RuntimeError, match="Failed to create TES instance.*404"):
            await tes.create(NEW_TASK)

    run(main)


def test_calls_waiting_for_a_failed_check_share_its_error(server: MockServer) -> None:
    """Concurrent first calls make one check; if it fails, all fail at once, not in turn."""
    path = "/failing-0.5/service-info"

    async def main() -> tuple[list[BaseException | Any], float]:
        tes = AsyncTES(Configuration(f"{server.root}/failing-0.5"))
        start = time.monotonic()
        results = await asyncio.gather(*(tes.list() for _ in range(5)), return_exceptions=True)
        return results, time.monotonic() - start

    before = server.paths().count(path)
    results, elapsed = run(main)
    assert all(isinstance(result, RuntimeError) for result in results)
    assert all("Failed to create TES instance" in str(result) for result in results)
    assert server.paths().count(path) - before == 1
    assert elapsed < 2  # One attempt after the other would take 2.5 s.


def test_methods_return_futures_of_the_results(config: Configuration) -> None:
    """Every method returns an `asyncio.Future` of the sync method's type of result."""

    async def main() -> None:
        tes = AsyncTES(config)
        task = AsyncTask("task-1", tes.transport)
        calls: list[tuple[Awaitable[Any], type | None]] = [
            (tes.create(NEW_TASK), AsyncTask),
            (tes.get("FULL", "task-1"), TesTask),
            (tes.list(), ListTasksResponse),
            (task.status(), TesState),
            (task.cancel(), type(None)),
            (AsyncTES.connect(config), AsyncTES),
        ]
        for call, result_type in calls:
            assert inspect.isawaitable(call) and isinstance(call, asyncio.Future)
            assert type(await call) is result_type

    run(main)


def test_create_sends_the_task_as_given(server: MockServer, config: Configuration) -> None:
    """`create()` sends the JSON, unknown fields included, and returns an `AsyncTask`."""

    async def main() -> AsyncTask:
        return await AsyncTES(config).create(NEW_TASK)

    task = run(main)
    assert type(task) is AsyncTask
    assert task.id == "task-1"
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/api/tasks")
    assert json.loads(request["body"]) == NEW_TASK


def test_create_takes_task_objects_and_files(
    server: MockServer, config: Configuration, tmp_path: Path
) -> None:
    """`create()` takes the same arguments as `TES.create()`."""
    path = tmp_path / "task.json"
    path.write_text(json.dumps(NEW_TASK))

    async def main() -> None:
        tes = AsyncTES(config)
        for argument in (TesTask(NEW_TASK), path, str(path)):
            await tes.create(argument)
            assert json.loads(server.last()["body"]) == NEW_TASK

    run(main)


def test_create_rejects_invalid_arguments_when_called(
    config: Configuration, tmp_path: Path
) -> None:
    """Invalid arguments raise when the method is called, before anything is sent."""

    async def main() -> None:
        tes = AsyncTES(config)
        with pytest.raises(ValueError, match=r"AsyncTES\.create\(\).*from_json"):
            tes.create(json.dumps(NEW_TASK))
        with pytest.raises(ValueError, match="task"):
            tes.create({"executors": "not a list"})
        with pytest.raises(FileNotFoundError):
            tes.create(tmp_path / "missing.json")

    run(main)


def test_get_returns_a_typed_task(server: MockServer, config: Configuration) -> None:
    """`get()` gives the same `TesTask` as `TES.get()`."""

    async def main() -> TesTask:
        return await AsyncTES(config).get("FULL", "task-1")

    task = run(main)
    assert isinstance(task, TesTask)
    assert task.state is TesState.COMPLETE
    assert task.to_dict() == TASK
    assert server.last()["query"] == {"view": ["FULL"]}


def test_get_takes_views_in_any_case_and_rejects_others_when_called(
    server: MockServer, config: Configuration
) -> None:
    """`get()` parses the view as `TES.get()` does, before anything is sent."""

    async def main() -> None:
        tes = AsyncTES(config)
        await tes.get("basic", "task-1")
        assert server.last()["query"] == {"view": ["BASIC"]}
        before = len(server.paths())
        with pytest.raises(ValueError):
            tes.get("EVERYTHING", "task-1")
        assert len(server.paths()) == before

    run(main)


def test_list_sends_the_filters(server: MockServer, config: Configuration) -> None:
    """`list()` repeats list filters and returns a `ListTasksResponse`."""

    async def main() -> ListTasksResponse:
        params = ListTasksParams(state="running", tag_key=["a", "b"], page_size=2)
        return await AsyncTES(config).list(params)

    page = run(main)
    assert isinstance(page, ListTasksResponse)
    assert [task.id for task in page.tasks] == ["task-1"]
    assert page.next_page_token == "next"
    assert server.last()["query"] == {
        "state": ["RUNNING"],
        "tag_key": ["a", "b"],
        "page_size": ["2"],
    }


def test_task_status_and_cancel(server: MockServer, config: Configuration) -> None:
    """`AsyncTask.status()` gives a `TesState`; `cancel()` gives `None`."""

    async def main() -> tuple[Any, Any]:
        task = AsyncTask("task-1", Transport(config))
        return await task.status(), await task.cancel()

    status, cancelled = run(main)
    assert status is TesState.COMPLETE
    assert cancelled is None
    request = server.last()
    assert (request["method"], request["path"]) == ("POST", "/api/tasks/task-1:cancel")


def test_task_has_id_and_transport(server: MockServer, config: Configuration) -> None:
    """`AsyncTask` has the same properties as `Task`."""
    task = AsyncTask("a/b", Transport(config))
    assert task.id == "a/b"
    assert json.loads(task.transport.get("tasks/task-1"))["id"] == "task-1"
    assert server.last()["path"] == "/api/tasks/task-1"


def test_get_sends_the_request_of_the_sync_client(
    server: MockServer, config: Configuration
) -> None:
    """`AsyncTES.get()` and `TES.get()` send the same request, also for IDs to encode."""

    async def main() -> TesTask:
        return await AsyncTES(config).get("BASIC", "a/b c")

    with pytest.raises(RuntimeError):
        run(main)
    sent = server.last()
    with pytest.raises(RuntimeError):
        TES(config).get("BASIC", "a/b c")
    assert server.last() == sent


AsyncCall = Callable[[AsyncTES, AsyncTask], Awaitable[Any]]


@pytest.mark.parametrize(
    ("call", "message"),
    [
        (lambda tes, task: tes.create(NEW_TASK), "Failed to create task"),
        (lambda tes, task: tes.get("FULL", "task-1"), "Failed to get task"),
        (lambda tes, task: tes.list(), "Failed to list tasks"),
        (lambda tes, task: task.status(), "Failed to get task status"),
        (lambda tes, task: task.cancel(), "Failed to cancel task"),
    ],
)
def test_failed_requests_raise_when_awaited(
    server: MockServer, call: AsyncCall, message: str
) -> None:
    """A failed request raises `RuntimeError` from the awaitable, not from the call."""
    config = Configuration(f"{server.root}/broken")  # Only the service info is found.

    async def main() -> None:
        tes = AsyncTES(config)
        awaitable = call(tes, AsyncTask("task-1", tes.transport))
        with pytest.raises(RuntimeError, match=f"{message}.*404"):
            await awaitable

    run(main)


def test_invalid_responses_raise_when_awaited(config: Configuration) -> None:
    """Responses that don't fit the TES models raise `RuntimeError` from the awaitable."""

    async def main() -> None:
        tes = AsyncTES(config)
        with pytest.raises(RuntimeError, match="Invalid TES response"):
            await tes.get("FULL", "bad")
        with pytest.raises(RuntimeError, match="Failed to get task status"):
            await AsyncTask("bad", tes.transport).status()

    run(main)


def test_calls_run_concurrently_without_blocking_the_loop(config: Configuration) -> None:
    """Five calls of 0.5 s each take about 0.5 s together, while the loop keeps running."""

    async def main() -> tuple[list[TesTask], float, int]:
        tes = AsyncTES(config)
        heartbeat = Heartbeat()
        heartbeat.start()
        start = time.monotonic()
        tasks = await asyncio.gather(*(tes.get("MINIMAL", "slow-0.5") for _ in range(5)))
        elapsed = time.monotonic() - start
        heartbeat.stop()
        return tasks, elapsed, heartbeat.ticks

    tasks, elapsed, ticks = run(main)
    assert [task.state for task in tasks] == [TesState.RUNNING] * 5
    assert elapsed < 2  # One after the other, they would take 2.5 s.
    assert ticks >= 5


def test_cancelling_connect_at_once_drops_the_check(server: MockServer) -> None:
    """A `connect()` cancelled before it runs never leaves its check running."""
    path = "/slow-10.5/service-info"

    async def main() -> None:
        pending = AsyncTES.connect(Configuration(f"{server.root}/slow-10.5"))
        pending.cancel()
        with pytest.raises(asyncio.CancelledError):
            await pending
        await asyncio.sleep(0.5)  # Time for a check that wasn't dropped to reach the server.

    run(main)
    deadline = time.monotonic() + 5
    while path in server.paths() and path not in server.disconnects():
        assert time.monotonic() < deadline, "the check is still running"
        time.sleep(0.05)


def test_cancelling_a_call_does_not_block_the_loop(
    server: MockServer, config: Configuration
) -> None:
    """Cancelling a pending call returns at once and drops its request; the loop goes on."""

    async def main() -> float:
        tes = AsyncTES(config)
        heartbeat = Heartbeat()
        heartbeat.start()
        pending = asyncio.ensure_future(tes.get("MINIMAL", "slow-10"))
        await asyncio.sleep(0.2)
        start = time.monotonic()
        pending.cancel()
        with pytest.raises(asyncio.CancelledError):
            await pending
        elapsed = time.monotonic() - start
        ticks = heartbeat.ticks
        await asyncio.sleep(0.2)
        assert heartbeat.ticks > ticks
        with pytest.raises(TimeoutError):
            await asyncio.wait_for(AsyncTask("slow-10.0", tes.transport).status(), 0.2)
        assert (await tes.get("MINIMAL", "task-1")).id == "task-1"
        heartbeat.stop()
        return elapsed

    assert run(main) < 1
    expected = {"/api/tasks/slow-10", "/api/tasks/slow-10.0"}
    deadline = time.monotonic() + 5
    while not expected <= set(server.disconnects()) and time.monotonic() < deadline:
        time.sleep(0.05)
    assert expected <= set(server.disconnects())


CALLS: dict[str, Callable[[Configuration], Any]] = {
    "connect": lambda config: AsyncTES.connect(config),
    "create": lambda config: AsyncTES(config).create(NEW_TASK),
    "get": lambda config: AsyncTES(config).get("FULL", "task-1"),
    "list": lambda config: AsyncTES(config).list(),
    "status": lambda config: AsyncTask("task-1", Transport(config)).status(),
    "cancel": lambda config: AsyncTask("task-1", Transport(config)).cancel(),
}


@pytest.mark.parametrize("method", sorted(CALLS))
def test_calls_need_a_running_loop(config: Configuration, method: str) -> None:
    """Calling a method outside of a running event loop raises `RuntimeError`."""
    with pytest.raises(RuntimeError, match="no running event loop"):
        CALLS[method](config)


REQUEST_ID: contextvars.ContextVar[str] = contextvars.ContextVar("REQUEST_ID", default="")


class TokenTES(AsyncTES):
    """A subclass with a further constructor argument, built on the event loop."""

    def __init__(self, config: Configuration, token: str) -> None:
        """Keeps the token, the running loop, a context variable and the service's tasks.

        Args:
            config: The client's configuration.
            token: An extra argument.
        """
        super().__init__()  # AsyncTES.__new__ has already set the client up.
        self.token = token
        self.loop = asyncio.get_running_loop()
        self.request_id = REQUEST_ID.get()
        self.first_page = json.loads(self.transport.get("tasks"))


def test_connect_builds_subclasses_on_the_loop(config: Configuration) -> None:
    """`connect()` builds a subclass with its arguments, in the caller's thread and context."""

    async def main() -> tuple[TokenTES, asyncio.AbstractEventLoop]:
        REQUEST_ID.set("request-1")
        tes = await TokenTES.connect(config, token="secret")
        assert isinstance(tes, TokenTES)
        return tes, asyncio.get_running_loop()

    tes, loop = run(main)
    assert type(tes) is TokenTES
    assert tes.token == "secret"
    assert tes.loop is loop
    assert tes.request_id == "request-1"
    assert tes.first_page["next_page_token"] == "next"


class NamedTask(AsyncTask):
    """A task subclass with a further constructor argument."""

    def __init__(self, id: str, transport: Transport, label: str) -> None:
        """Keeps the label.

        Args:
            id: The task's ID.
            transport: The transport to the task's service.
            label: An extra argument.
        """
        super().__init__()
        self.label = label


def test_task_subclasses_take_further_arguments(config: Configuration) -> None:
    """`AsyncTask` subclasses work like `Task` subclasses; `create()` gives plain tasks."""

    async def main() -> tuple[TesState, AsyncTask]:
        task = NamedTask("task-1", Transport(config), "first")
        assert task.label == "first"
        return await task.status(), await AsyncTES(config).create(NEW_TASK)

    status, created = run(main)
    assert status is TesState.COMPLETE
    assert type(created) is AsyncTask


def test_subclasses_without_init_reject_further_arguments(config: Configuration) -> None:
    """As for the sync classes, unknown constructor arguments raise `TypeError`."""

    class Plain(AsyncTES):
        """A subclass without `__init__`."""

    with pytest.raises(TypeError, match="unexpected arguments"):
        Plain(config, "extra")  # type: ignore[call-arg]
    with pytest.raises(TypeError, match="unexpected arguments"):
        AsyncTask("task-1", Transport(config), "extra")  # type: ignore[call-arg]
