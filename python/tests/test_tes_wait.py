"""Waiting for TES tasks: `Task.wait()` and `AsyncTask.wait()`."""

import _thread
import asyncio
import math
import threading
import time
from collections.abc import Awaitable, Callable
from typing import Any, TypeVar

import pytest

from ga4gh_sdk import (
    AsyncTask,
    Configuration,
    Task,
    TesState,
    TesTask,
    Transport,
    WaitTimeoutError,
)

from .conftest import MockServer

T = TypeVar("T")

TERMINAL_STATES = ["COMPLETE", "EXECUTOR_ERROR", "SYSTEM_ERROR", "CANCELED", "PREEMPTED"]

# Short intervals, so that the tests poll several times within a fraction of a second.
FAST = {"poll_interval": 0.01, "max_interval": 0.02}


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


def requests_to(server: MockServer, path: str) -> int:
    """Counts the requests that the mock server received for a path.

    Args:
        server: The mock server.
        path: The request path, such as `/api/tasks/progress`.

    Returns:
        The number of requests for the path so far.
    """
    return server.paths().count(path)


def task(config: Configuration, task_id: str) -> Task:
    """Builds a `Task` with an ID on the mock TES.

    Args:
        config: The mock TES's configuration.
        task_id: The task's ID.

    Returns:
        The task.
    """
    return Task(task_id, Transport(config))


def async_task(config: Configuration, task_id: str) -> AsyncTask:
    """Builds an `AsyncTask` with an ID on the mock TES.

    Args:
        config: The mock TES's configuration.
        task_id: The task's ID.

    Returns:
        The task.
    """
    return AsyncTask(task_id, Transport(config))


def test_timeout_error_is_a_timeout_error_of_the_package() -> None:
    """`WaitTimeoutError` is a `TimeoutError` of `ga4gh_sdk`; its fields default to `None`."""
    assert issubclass(WaitTimeoutError, TimeoutError)
    assert WaitTimeoutError.__module__ == "ga4gh_sdk"
    error = WaitTimeoutError("message")
    assert (str(error), error.id, error.last_state) == ("message", None, None)


@pytest.mark.parametrize("state", TERMINAL_STATES)
def test_wait_returns_the_terminal_state(
    server: MockServer, config: Configuration, state: str
) -> None:
    """Each terminal state ends the wait and comes back as a `TesState`.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
        state: The task's terminal state.
    """
    assert task(config, f"state-{state}").wait(**FAST) is TesState[state]
    request = server.last()
    assert request["path"] == f"/api/tasks/state-{state}"
    assert request["query"] == {"view": ["MINIMAL"]}


def test_wait_polls_until_the_task_ends(server: MockServer, config: Configuration) -> None:
    """The wait polls a queued, then running task until it completes.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    before = requests_to(server, "/api/tasks/progress")
    assert task(config, "progress").wait(**FAST) is TesState.COMPLETE
    assert requests_to(server, "/api/tasks/progress") - before == 3


def test_wait_returns_the_task_in_a_view(server: MockServer, config: Configuration) -> None:
    """With a view, the wait gets the ended task in that view once.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    result = task(config, "state-COMPLETE").wait(view="full", **FAST)
    assert isinstance(result, TesTask)
    assert result.state is TesState.COMPLETE
    assert result.to_dict()["extension_field"] == {"nested": [1, 2, 3]}
    assert server.last()["query"] == {"view": ["FULL"]}


def test_wait_in_the_minimal_view_reuses_the_last_poll(
    server: MockServer, config: Configuration
) -> None:
    """The minimal view is what the wait polls, so it sends no further request.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    before = requests_to(server, "/api/tasks/state-CANCELED")
    result = task(config, "state-CANCELED").wait(view="MINIMAL", **FAST)
    assert isinstance(result, TesTask)
    assert (result.id, result.state) == ("state-CANCELED", TesState.CANCELED)
    assert requests_to(server, "/api/tasks/state-CANCELED") - before == 1


def test_wait_raises_a_timeout_error(server: MockServer, config: Configuration) -> None:
    """A task that keeps running ends the wait with `WaitTimeoutError` after the timeout.

    The error is a `TimeoutError`, names the task and its last state, and the task is not
    cancelled.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    started = time.monotonic()
    with pytest.raises(WaitTimeoutError) as raised:
        task(config, "state-RUNNING").wait(0.2, **FAST)
    assert 0.15 < time.monotonic() - started < 5
    assert isinstance(raised.value, TimeoutError)
    assert raised.value.id == "state-RUNNING"
    assert raised.value.last_state is TesState.RUNNING
    assert "state-RUNNING" in str(raised.value)
    assert not [path for path in server.paths() if path.endswith(":cancel")]


def test_wait_with_no_time_left_does_not_poll(server: MockServer, config: Configuration) -> None:
    """A timeout of zero ends the wait at once, without a last state.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    before = len(server.paths())
    with pytest.raises(WaitTimeoutError) as raised:
        task(config, "state-COMPLETE").wait(timeout=0)
    assert raised.value.last_state is None
    assert len(server.paths()) == before


def test_wait_keeps_polling_states_it_does_not_know(config: Configuration) -> None:
    """A state that TES 1.1 doesn't define is not an end; it reads as `UNKNOWN`.

    Args:
        config: The mock TES's configuration.
    """
    with pytest.raises(WaitTimeoutError) as raised:
        task(config, "new-state").wait(0.1, **FAST)
    assert raised.value.last_state is TesState.UNKNOWN


def test_wait_raises_runtime_error_when_a_poll_fails(config: Configuration) -> None:
    """A failed poll ends the wait with `RuntimeError`.

    Args:
        config: The mock TES's configuration.
    """
    with pytest.raises(RuntimeError, match="Failed to wait for task"):
        task(config, "missing").wait(**FAST)


@pytest.mark.parametrize(
    "arguments",
    [
        {"timeout": -1},
        {"timeout": math.nan},
        {"poll_interval": 0},
        {"poll_interval": -1},
        {"poll_interval": math.inf},
        {"poll_interval": 2, "max_interval": 1},
        {"view": "EVERYTHING"},
    ],
)
def test_wait_rejects_invalid_arguments(config: Configuration, arguments: dict[str, Any]) -> None:
    """Invalid timeouts, intervals and views raise `ValueError`, sync and async.

    Args:
        config: The mock TES's configuration.
        arguments: The invalid arguments.
    """
    with pytest.raises(ValueError):
        task(config, "state-COMPLETE").wait(**arguments)

    async def main() -> None:
        """Calls `AsyncTask.wait()` on a running loop."""
        async_task(config, "state-COMPLETE").wait(**arguments)

    with pytest.raises(ValueError):
        run(main)


def test_wait_releases_the_gil(config: Configuration) -> None:
    """Other Python threads run while a thread waits.

    Args:
        config: The mock TES's configuration.
    """
    ticks = 0
    done = threading.Event()

    def count() -> None:
        """Counts until the wait ends."""
        nonlocal ticks
        while not done.is_set():
            ticks += 1
            time.sleep(0.001)

    counter = threading.Thread(target=count)
    counter.start()
    try:
        with pytest.raises(WaitTimeoutError):
            task(config, "state-RUNNING").wait(0.3, **FAST)
    finally:
        done.set()
        counter.join()
    assert ticks > 10


def test_keyboard_interrupt_stops_the_wait_only(
    server: MockServer, config: Configuration
) -> None:
    """Ctrl-C raises `KeyboardInterrupt` and stops polling; the task is not cancelled.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    timer = threading.Timer(0.2, _thread.interrupt_main)
    timer.start()
    started = time.monotonic()
    try:
        with pytest.raises(KeyboardInterrupt):
            task(config, "state-RUNNING").wait(**FAST)
    finally:
        timer.cancel()
    assert time.monotonic() - started < 5
    polls = requests_to(server, "/api/tasks/state-RUNNING")
    time.sleep(0.2)
    assert requests_to(server, "/api/tasks/state-RUNNING") == polls
    assert not [path for path in server.paths() if path.endswith(":cancel")]


@pytest.mark.parametrize("state", TERMINAL_STATES)
def test_async_wait_returns_the_terminal_state(config: Configuration, state: str) -> None:
    """`AsyncTask.wait()` gives the terminal state as a `TesState`.

    Args:
        config: The mock TES's configuration.
        state: The task's terminal state.
    """

    async def main() -> TesState:
        """Waits for the task.

        Returns:
            The task's final state.
        """
        return await async_task(config, f"state-{state}").wait(**FAST)

    assert run(main) is TesState[state]


def test_async_wait_returns_the_task_in_a_view(server: MockServer, config: Configuration) -> None:
    """With a view, `AsyncTask.wait()` gives the ended task in that view.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """

    async def main() -> TesTask:
        """Waits for the task.

        Returns:
            The task in the basic view.
        """
        return await async_task(config, "state-EXECUTOR_ERROR").wait(view="BASIC", **FAST)

    result = run(main)
    assert isinstance(result, TesTask)
    assert result.state is TesState.EXECUTOR_ERROR
    assert server.last()["query"] == {"view": ["BASIC"]}


def test_async_wait_raises_a_timeout_error(server: MockServer, config: Configuration) -> None:
    """`AsyncTask.wait()` raises `WaitTimeoutError` after the timeout, without cancelling.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """

    async def main() -> None:
        """Waits for a task that keeps running."""
        await async_task(config, "state-RUNNING").wait(timeout=0.2, **FAST)

    with pytest.raises(WaitTimeoutError) as raised:
        run(main)
    assert raised.value.last_state is TesState.RUNNING
    assert not [path for path in server.paths() if path.endswith(":cancel")]


def test_async_wait_raises_runtime_error_when_a_poll_fails(config: Configuration) -> None:
    """A failed poll ends `AsyncTask.wait()` with `RuntimeError`.

    Args:
        config: The mock TES's configuration.
    """

    async def main() -> None:
        """Waits for a task that the server doesn't know."""
        await async_task(config, "missing").wait(**FAST)

    with pytest.raises(RuntimeError, match="Failed to wait for task"):
        run(main)


def test_cancelling_the_async_wait_stops_polling_only(
    server: MockServer, config: Configuration
) -> None:
    """Cancelling the awaiting coroutine stops polling and leaves the task running.

    The event loop runs on while the wait polls.

    Args:
        server: The mock server.
        config: The mock TES's configuration.
    """
    ticks = 0

    async def main() -> int:
        """Starts a wait, lets it poll, then cancels it.

        Returns:
            The number of polls before the cancellation took effect.
        """
        nonlocal ticks
        waiting = asyncio.ensure_future(async_task(config, "state-RUNNING").wait(**FAST))
        for _ in range(20):
            await asyncio.sleep(0.01)
            ticks += 1
        waiting.cancel()
        with pytest.raises(asyncio.CancelledError):
            await waiting
        await asyncio.sleep(0.1)  # Lets a poll already in flight end.
        return requests_to(server, "/api/tasks/state-RUNNING")

    polls = run(main)
    assert polls > 0
    assert ticks == 20
    time.sleep(0.2)
    assert requests_to(server, "/api/tasks/state-RUNNING") == polls
    assert not [path for path in server.paths() if path.endswith(":cancel")]
