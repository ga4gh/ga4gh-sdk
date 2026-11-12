"""Type stubs for the GA4GH SDK's Python bindings.

`tests/test_stubs.py` checks that they declare every class and member of the module.
"""

import asyncio
import enum
import os
from collections.abc import Callable, Iterable, Mapping, Sequence
from typing import Any, Generic, Self, TypeAlias, TypeVar, final, overload

_JSONObject: TypeAlias = dict[str, Any]
"""A JSON object as a `dict`, for example a task's executor."""

_Params: TypeAlias = Mapping[str, Any] | str
"""Query parameters: a mapping or a JSON object string."""

@final
class ServiceType:
    """A built-in section of the configuration file, for `Configuration.from_file()`.

    Other sections are named by a `str`.
    """

    TES: ServiceType
    DRS: ServiceType
    TRS: ServiceType
    AAI: ServiceType
    WES: ServiceType
    REGISTRY: ServiceType

class Configuration:
    """The API root and settings for connecting to a GA4GH service.

    `from_file()` reads a section of `~/.ga4gh-cli/config.json`, named by a
    `ServiceType` or by a `str`; `get_extra()` returns the section's other keys.
    `set_timeouts()` and `get_timeouts()` set and give the time limits of
    requests in seconds; clients and transports read them when they are built.
    `set_token_provider()` sets a callable that gives a bearer token for each
    request.
    """

    def __init__(self, base_path: str) -> None: ...
    def set_base_path(self, base_path: str) -> None: ...
    def get_base_path(self) -> str: ...
    def set_basic_auth(self, username: str, password: str | None = None) -> None: ...
    def set_oauth_access_token(self, token: str) -> None: ...
    def set_bearer_access_token(self, token: str) -> None: ...
    def clear_auth(self) -> None: ...
    def get_extra(self) -> _JSONObject: ...
    def set_timeouts(
        self,
        *,
        connect: float | None = None,
        request: float | None = None,
        discovery: float | None = None,
        submission: float | None = None,
    ) -> None: ...
    def get_timeouts(self) -> dict[str, float]: ...
    def set_token_provider(
        self,
        provider: Callable[[str], str | tuple[str, float]] | None,
        *,
        audience: str | None = None,
    ) -> None: ...
    def from_file(self, service_type: ServiceType | str) -> None: ...

class ServiceInfo:
    """Reads a service's service info."""

    def __init__(self, py_config: Configuration) -> None: ...
    def get_service_info(self) -> str: ...

class DrsObject:
    """A DRS object's metadata, as returned by the server.

    Keeps the object's full JSON, including fields that the DRS models don't know.
    """

    @property
    def id(self) -> str: ...
    @property
    def name(self) -> str | None: ...
    @property
    def self_uri(self) -> str: ...
    @property
    def size(self) -> int: ...
    @property
    def created_time(self) -> str: ...
    @property
    def updated_time(self) -> str | None: ...
    @property
    def version(self) -> str | None: ...
    @property
    def mime_type(self) -> str | None: ...
    @property
    def checksums(self) -> list[_JSONObject] | None: ...
    @property
    def access_methods(self) -> list[_JSONObject] | None: ...
    @property
    def contents(self) -> list[_JSONObject] | None: ...
    @property
    def description(self) -> str | None: ...
    @property
    def aliases(self) -> list[str] | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class AccessURL:
    """An access URL of a DRS object. `repr()` hides the URL and headers."""

    @property
    def url(self) -> str: ...
    @property
    def headers(self) -> list[str] | None:
        """The headers to send with a request to `url`, as `"Name: value"` strings."""
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class BulkAccessURL:
    """An access URL from `DRS.get_bulk_access_urls()`, with the IDs it belongs to."""

    @property
    def drs_object_id(self) -> str | None: ...
    @property
    def drs_access_id(self) -> str | None: ...
    @property
    def url(self) -> str: ...
    @property
    def headers(self) -> list[str] | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class PendingResolution:
    """The server is still preparing an answer: ask again after `retry_after` seconds."""

    @property
    def api_root(self) -> str: ...
    @property
    def endpoint(self) -> str:
        """The endpoint to ask again; for a bulk call, call the bulk method again."""
    @property
    def retry_after(self) -> int: ...
    def to_dict(self) -> _JSONObject: ...

class BulkPolicy:
    """Limits of the bulk calls of `DRS`; checked when a call starts."""

    def __init__(
        self,
        chunk_size: int = 100,
        advertised_limit: int | None = None,
        concurrency: int = 4,
        max_requests: int = 1000,
        max_split_depth: int = 8,
        max_attempts: int = 1,
        initial_delay: float = 0.2,
        max_delay: float = 10.0,
    ) -> None: ...
    def with_service_info(self, service_info: Mapping[str, Any] | str) -> BulkPolicy: ...
    @property
    def chunk_size(self) -> int: ...
    @property
    def advertised_limit(self) -> int | None: ...
    @property
    def concurrency(self) -> int: ...
    @property
    def max_requests(self) -> int: ...
    @property
    def max_split_depth(self) -> int: ...
    @property
    def max_attempts(self) -> int: ...
    @property
    def initial_delay(self) -> float: ...
    @property
    def max_delay(self) -> float: ...

_Resolved = TypeVar("_Resolved", DrsObject, BulkAccessURL, _JSONObject)
"""What a bulk call resolves: a typed result or the complete JSON object."""

class BulkOutcome(Generic[_Resolved]):
    """The outcome for one requested ID (or ID pair) of a bulk call."""

    @property
    def object_id(self) -> str: ...
    @property
    def access_id(self) -> str | None: ...
    @property
    def resolved(self) -> list[_Resolved]: ...
    @property
    def failure_codes(self) -> list[int]: ...
    @property
    def pending(self) -> PendingResolution | None: ...
    @property
    def errors(self) -> list[str]: ...
    @property
    def is_success(self) -> bool: ...

class BulkReport(Generic[_Resolved]):
    """The outcomes, unattributed problems, and observed reply shapes of a bulk call."""

    @property
    def outcomes(self) -> list[BulkOutcome[_Resolved]]: ...
    @property
    def diagnostics(self) -> list[str]: ...
    @property
    def dialects(self) -> list[str]: ...

class DRS:
    """A DRS client."""

    def __init__(self, config: Configuration) -> None: ...
    @property
    def config(self) -> Configuration: ...
    @property
    def transport(self) -> Transport: ...
    def get_object(self, object_id: str) -> DrsObject: ...
    def get_object_raw(self, object_id: str) -> _JSONObject: ...
    def get_object_once(self, object_id: str) -> DrsObject | PendingResolution: ...
    def resolve_object(
        self, object_id: str, timeout: float = 60.0, max_attempts: int = 5
    ) -> DrsObject: ...
    def get_access_url(self, object_id: str, access_id: str) -> AccessURL: ...
    def get_access_url_raw(self, object_id: str, access_id: str) -> _JSONObject: ...
    def get_access_url_once(
        self, object_id: str, access_id: str
    ) -> AccessURL | PendingResolution: ...
    def resolve_access_url(
        self, object_id: str, access_id: str, timeout: float = 60.0, max_attempts: int = 5
    ) -> AccessURL: ...
    def get_bulk_objects(
        self,
        ids: Iterable[str],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[DrsObject]: ...
    def get_bulk_objects_raw(
        self,
        ids: Iterable[str],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[_JSONObject]: ...
    def get_bulk_access_urls(
        self,
        pairs: Mapping[str, str] | Iterable[tuple[str, str]],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[BulkAccessURL]: ...
    def get_bulk_access_urls_raw(
        self,
        pairs: Mapping[str, str] | Iterable[tuple[str, str]],
        passports: Iterable[str] | None = None,
        policy: BulkPolicy | None = None,
        timeout: float = 60.0,
    ) -> BulkReport[_JSONObject]: ...

class DrsUri:
    """A hostname-based DRS URI, such as `drs://drs.example.org/object-id`."""

    def __init__(self, uri: str) -> None: ...
    @property
    def authority(self) -> str: ...
    @property
    def object_id(self) -> str: ...
    def default_api_root(self) -> str: ...

class AccessPlan:
    """How to fetch a DRS object's bytes. `repr()` shows only the URL's origin."""

    @property
    def object_id(self) -> str: ...
    @property
    def method(self) -> str: ...
    @property
    def region(self) -> str | None: ...
    @property
    def url(self) -> str: ...
    @property
    def headers(self) -> list[tuple[str, str]]:
        """`(name, value)` pairs, unlike the `"Name: value"` strings of `AccessURL`."""

class DrsResolver:
    """Turns hostname-based DRS URIs into `AccessPlan`s."""

    def __init__(self) -> None: ...
    def with_service(self, authority: str, transport: Transport) -> Self: ...
    def access_plan_once(
        self,
        uri: str,
        protocols: Sequence[str] | None = None,
        regions: Sequence[str] | None = None,
        method_index: int | None = None,
        timeout: float = 60.0,
    ) -> AccessPlan | PendingResolution: ...
    def access_plan(
        self,
        uri: str,
        protocols: Sequence[str] | None = None,
        regions: Sequence[str] | None = None,
        method_index: int | None = None,
        timeout: float = 60.0,
        max_attempts: int = 5,
    ) -> AccessPlan: ...

class Transport:
    """Sends requests below the configured API root and returns response bodies as text."""

    def __init__(self, py_config: Configuration) -> None: ...
    def get(self, endpoint: str, params: _Params | None = None) -> str: ...
    def post(self, endpoint: str, data: Any = None, params: _Params | None = None) -> str: ...
    def put(self, endpoint: str, data: Any, params: _Params | None = None) -> str: ...
    def delete(self, endpoint: str, params: _Params | None = None) -> str: ...
    def post_multipart(
        self,
        endpoint: str,
        fields: list[tuple[str, str]] | None = None,
        files: list[tuple[str, str, bytes] | tuple[str, str, bytes, str]] | None = None,
        params: _Params | None = None,
    ) -> str: ...

class WesState(str, enum.Enum):
    """The state of a WES run. Members are strings equal to the state's name."""

    UNKNOWN = "UNKNOWN"
    QUEUED = "QUEUED"
    INITIALIZING = "INITIALIZING"
    RUNNING = "RUNNING"
    PAUSED = "PAUSED"
    COMPLETE = "COMPLETE"
    EXECUTOR_ERROR = "EXECUTOR_ERROR"
    SYSTEM_ERROR = "SYSTEM_ERROR"
    CANCELED = "CANCELED"
    CANCELING = "CANCELING"
    PREEMPTED = "PREEMPTED"

    def __str__(self) -> str: ...
    def __format__(self, format_spec: str) -> str: ...

class RunRequest:
    """A WES run request: a workflow to run, with its parameters.

    Keeps the request's full JSON, including fields that the WES models don't know.
    `WES.run()` sends those fields as further form parts: strings as they are, other
    values as JSON.
    """

    def __init__(self, request: RunRequest | Mapping[str, Any] | str) -> None: ...
    @classmethod
    def from_dict(cls, request: Mapping[str, Any]) -> Self: ...
    @classmethod
    def from_json(cls, text: str) -> Self: ...
    @classmethod
    def from_file(cls, path: str | os.PathLike[str]) -> Self: ...
    @property
    def workflow_type(self) -> str: ...
    @property
    def workflow_type_version(self) -> str: ...
    @property
    def workflow_url(self) -> str: ...
    @property
    def workflow_params(self) -> _JSONObject | None: ...
    @property
    def tags(self) -> dict[str, str] | None: ...
    @property
    def workflow_engine(self) -> str | None: ...
    @property
    def workflow_engine_version(self) -> str | None: ...
    @property
    def workflow_engine_parameters(self) -> dict[str, str] | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class RunLog:
    """A run's details, with its request, state, logs and outputs, as returned by the server."""

    @property
    def run_id(self) -> str | None: ...
    @property
    def request(self) -> RunRequest | None: ...
    @property
    def state(self) -> WesState: ...
    @property
    def run_log(self) -> _JSONObject | None: ...
    @property
    def task_logs_url(self) -> str | None: ...
    @property
    def task_logs(self) -> list[_JSONObject] | None: ...
    @property
    def outputs(self) -> Any: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class RunSummary:
    """A run in the list of `WES.list()`."""

    @property
    def run_id(self) -> str: ...
    @property
    def state(self) -> WesState: ...
    @property
    def start_time(self) -> str | None: ...
    @property
    def end_time(self) -> str | None: ...
    @property
    def tags(self) -> dict[str, str] | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class RunListResponse:
    """One page of runs from `WES.list()`."""

    @property
    def runs(self) -> list[RunSummary]: ...
    @property
    def next_page_token(self) -> str | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class TaskLog:
    """A task of a run, from `Run.tasks()` or `Run.task()` (WES 1.1)."""

    @property
    def id(self) -> str: ...
    @property
    def name(self) -> str: ...
    @property
    def cmd(self) -> list[str] | None: ...
    @property
    def start_time(self) -> str | None: ...
    @property
    def end_time(self) -> str | None: ...
    @property
    def stdout(self) -> str | None: ...
    @property
    def stderr(self) -> str | None: ...
    @property
    def exit_code(self) -> int | None: ...
    @property
    def system_logs(self) -> list[str] | None: ...
    @property
    def tes_uri(self) -> str | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class TaskListResponse:
    """One page of tasks from `Run.tasks()`."""

    @property
    def task_logs(self) -> list[TaskLog]: ...
    @property
    def next_page_token(self) -> str | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class Run:
    """A WES run on the server, addressed by its ID."""

    def __init__(self, id: str, transport: Transport) -> None: ...
    @property
    def id(self) -> str: ...
    @property
    def transport(self) -> Transport: ...
    def status(self) -> WesState: ...
    def cancel(self) -> None: ...
    def log(self) -> RunLog: ...
    def tasks(
        self, page_size: int | None = None, page_token: str | None = None
    ) -> TaskListResponse: ...
    def task(self, task_id: str) -> TaskLog: ...

_Attachments: TypeAlias = (
    Mapping[str, bytes | bytearray | os.PathLike[str]]
    | Iterable[tuple[str, bytes | bytearray | os.PathLike[str]]]
)
"""Workflow attachments: filenames (relative paths) mapped to contents or file paths."""

class WES:
    """A WES client."""

    def __init__(self, config: Configuration) -> None: ...
    @property
    def config(self) -> Configuration: ...
    @property
    def transport(self) -> Transport: ...
    def run(
        self,
        request: RunRequest | Mapping[str, Any] | str | os.PathLike[str],
        attachments: _Attachments | None = None,
    ) -> Run: ...
    def list(
        self, page_size: int | None = None, page_token: str | None = None
    ) -> RunListResponse: ...
    def get(self, id: str) -> RunLog: ...

class TesState(str, enum.Enum):
    """The state of a TES task. Members are strings equal to the state's name."""

    UNKNOWN = "UNKNOWN"
    QUEUED = "QUEUED"
    INITIALIZING = "INITIALIZING"
    RUNNING = "RUNNING"
    PAUSED = "PAUSED"
    COMPLETE = "COMPLETE"
    EXECUTOR_ERROR = "EXECUTOR_ERROR"
    SYSTEM_ERROR = "SYSTEM_ERROR"
    CANCELED = "CANCELED"
    PREEMPTED = "PREEMPTED"
    CANCELING = "CANCELING"

    def __str__(self) -> str: ...
    def __format__(self, format_spec: str) -> str: ...

class TesTask:
    """A TES task: one to create, or one as returned by the server.

    Keeps the task's full JSON, including fields that the TES models don't know.
    """

    def __init__(self, task: TesTask | Mapping[str, Any] | str) -> None: ...
    @classmethod
    def from_dict(cls, task: Mapping[str, Any]) -> Self: ...
    @classmethod
    def from_json(cls, text: str) -> Self: ...
    @classmethod
    def from_file(cls, path: str | os.PathLike[str]) -> Self: ...
    @property
    def id(self) -> str | None: ...
    @property
    def state(self) -> TesState: ...
    @property
    def status(self) -> TesState:
        """Same as `state`."""
    @property
    def name(self) -> str | None: ...
    @property
    def description(self) -> str | None: ...
    @property
    def creation_time(self) -> str | None: ...
    @property
    def inputs(self) -> list[_JSONObject] | None: ...
    @property
    def outputs(self) -> list[_JSONObject] | None: ...
    @property
    def resources(self) -> _JSONObject | None: ...
    @property
    def executors(self) -> list[_JSONObject] | None: ...
    @property
    def volumes(self) -> list[str] | None: ...
    @property
    def tags(self) -> dict[str, str] | None: ...
    @property
    def logs(self) -> list[_JSONObject] | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

PyTesTask = TesTask
"""Previous name of `TesTask`."""

class Task:
    """A TES task on the server, addressed by its ID."""

    def __init__(self, id: str, transport: Transport) -> None: ...
    @property
    def id(self) -> str: ...
    @property
    def transport(self) -> Transport: ...
    def status(self) -> TesState: ...
    def cancel(self) -> None: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: None = None,
    ) -> TesState: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str,
    ) -> TesTask: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str | None = None,
    ) -> TesState | TesTask:
        """Waits until the task ends; gives its final state, or the task in `view`.

        A task ends in `COMPLETE`, `EXECUTOR_ERROR`, `SYSTEM_ERROR`, `CANCELED` or
        `PREEMPTED`. Polls start `poll_interval` seconds apart; the interval doubles up to
        `max_interval` (by default 10 seconds, or `poll_interval` if that's longer).
        `timeout` is the time in seconds for the whole wait; `None` waits as long as it
        takes. Raises `WaitTimeoutError` when the time runs out, `RuntimeError` if a poll
        fails and `ValueError` for invalid arguments. Releases the GIL; Ctrl-C stops the
        wait with `KeyboardInterrupt`. None of these cancels the task.
        """

class WaitTimeoutError(TimeoutError):
    """Raised when `Task.wait()` or `AsyncTask.wait()` runs out of time.

    The task goes on running.
    """

    id: str | None
    """The task's ID; `None` on an error raised by other code."""
    last_state: TesState | None
    """The task's last polled state, or `None` if no poll succeeded."""

class ListTasksParams:
    """Filters and paging for `TES.list()`."""

    def __init__(
        self,
        name_prefix: str | None = None,
        state: TesState | str | None = None,
        tag_key: list[str] | None = None,
        tag_value: list[str] | None = None,
        page_size: int | None = None,
        page_token: str | None = None,
        view: str | None = None,
    ) -> None: ...
    @property
    def name_prefix(self) -> str | None: ...
    @property
    def state(self) -> TesState | None: ...
    @property
    def tag_key(self) -> list[str] | None: ...
    @property
    def tag_value(self) -> list[str] | None: ...
    @property
    def page_size(self) -> int | None: ...
    @property
    def page_token(self) -> str | None: ...
    @property
    def view(self) -> str | None: ...

class ListTasksResponse:
    """One page of tasks from `TES.list()`."""

    @property
    def tasks(self) -> list[TesTask]: ...
    @property
    def next_page_token(self) -> str | None: ...
    def json(self) -> str: ...
    def to_dict(self) -> _JSONObject: ...

class TES:
    """A TES client."""

    def __init__(self, config: Configuration) -> None: ...
    @property
    def config(self) -> Configuration: ...
    @property
    def transport(self) -> Transport: ...
    def create(self, task: TesTask | Mapping[str, Any] | str | os.PathLike[str]) -> Task: ...
    def get(self, view: str, id: str) -> TesTask: ...
    def list(self, params: ListTasksParams | None = None) -> ListTasksResponse: ...

class AsyncTES:
    """A TES client for `asyncio`: `create`, `get` and `list` return awaitables.

    The methods return `asyncio.Future`s of the running event loop, so they must be called
    from it. `AsyncTES(config)` doesn't contact the server; `await AsyncTES.connect(config)`
    first checks that `config` points at a TES service; otherwise the first call does.
    """

    def __init__(self, config: Configuration) -> None: ...
    @classmethod
    def connect(cls, config: Configuration, *args: Any, **kwargs: Any) -> asyncio.Future[Self]:
        """Builds `cls(config, *args, **kwargs)`; gives it once it has checked the service."""
    @property
    def config(self) -> Configuration: ...
    @property
    def transport(self) -> Transport: ...
    def create(
        self, task: TesTask | Mapping[str, Any] | str | os.PathLike[str]
    ) -> asyncio.Future[AsyncTask]: ...
    def get(self, view: str, id: str) -> asyncio.Future[TesTask]: ...
    def list(self, params: ListTasksParams | None = None) -> asyncio.Future[ListTasksResponse]: ...

class AsyncTask:
    """A TES task on the server, addressed by its ID, for `asyncio`."""

    def __init__(self, id: str, transport: Transport) -> None: ...
    @property
    def id(self) -> str: ...
    @property
    def transport(self) -> Transport: ...
    def status(self) -> asyncio.Future[TesState]: ...
    def cancel(self) -> asyncio.Future[None]: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: None = None,
    ) -> asyncio.Future[TesState]: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str,
    ) -> asyncio.Future[TesTask]: ...
    @overload
    def wait(
        self,
        timeout: float | None = None,
        *,
        poll_interval: float = 1.0,
        max_interval: float | None = None,
        view: str | None = None,
    ) -> asyncio.Future[TesState] | asyncio.Future[TesTask]:
        """Waits until the task ends, like `Task.wait()`.

        Invalid arguments raise `ValueError` when the method is called. Cancelling the
        awaiting coroutine stops polling; it doesn't cancel the task.
        """
