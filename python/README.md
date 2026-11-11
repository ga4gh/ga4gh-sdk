## GA4GH-SDK Python Bindings

This directory provides Python bindings for the GA4GH SDK using [PyO3](https://docs.rs/pyo3/latest/pyo3).
These instructions build the bindings from this repository. Start at the
repository root; after `cd python`, run later commands from `python/` with the
virtual environment active.

## Prerequisites

- CPython 3.11 or newer (PyPy and free-threaded builds are not supported)
- The [Rust toolchain](../README.md#installation)

## Installation

From the repository root, create an environment, build the bindings, and check
the import:

```sh
cd python
python3 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.7,<2'
maturin develop --locked
python -c 'from GA4GH import TES; print(TES.__name__)'
```

Use a Python 3.11+ executable if `python3` points to an older version. The
distribution is named `GA4GH-SDK`; the import name is `GA4GH`.

## Build a wheel

Build the Python library wheel:

```bash
maturin build --locked
```

The bindings use Python's stable ABI (`abi3`), so a `cp311-abi3` wheel can be
installed on CPython 3.11 and newer on the same platform.

## Running Tests

After `maturin develop`, install pytest and run the Python binding tests with:

```bash
python -m pip install pytest
pytest
```

## Supported surface

| Python API | Current behavior |
| --- | --- |
| `Configuration` | Set one service API root or read it from `~/.ga4gh-cli/config.json`. |
| `ServiceInfo` | Fetch service info as a formatted string; it is not yet a typed Python model. |
| `TES`, `Task`, `TesTask`, `ListTasksParams` | Create, inspect, list, poll, and cancel TES tasks; read full task JSON including unknown fields. |
| `AsyncTES`, `AsyncTask` | The same TES calls for `asyncio`, as awaitables; see [asyncio](#asyncio). |
| `Transport` | Send relative HTTP requests with JSON, query parameters, or multipart bodies; responses are text. |
| `WES`, `Run`, `RunRequest` | Start runs (with workflow attachments), list, inspect, poll, and cancel WES runs, and read their tasks; read full JSON including unknown fields. See [Using WES](#using-wes). |
| DRS, TRS, Service Registry | Available in the Rust SDK, not yet as Python client classes. |

The Python API is maintained by hand; a new Rust SDK method or model does not
appear in Python automatically. For each SDK change, decide whether to expose
it here, then update the wrapper, this table, and the mock-backed tests and
examples. CI checks the documented Python behavior, but it cannot infer that
a new Rust API needs a Python counterpart. The SDK currently stores configured
credentials without sending them in requests (see [#31](https://github.com/ga4gh/ga4gh-sdk/issues/31)); use an anonymous service or an authenticating proxy.

## Usage

`Configuration` takes the API root, including the service's path prefix (see
the [usage guide](../docs/usage.md#api-root-base_path)).

Run this example from `python/` after starting Funnel and Docker as described
in the [quick start](../README.md#quick-start). The checked-in task is
[`examples/tes-hello.json`](../examples/tes-hello.json) from the repository
root, so its path from `python/` is `../examples/tes-hello.json`. It runs
`echo` in an Alpine container without external storage.

```python
from GA4GH import Configuration, ServiceInfo, ServiceType, TES

config = Configuration("http://localhost:8000/ga4gh/tes/v1")
# Or read the TES section of ~/.ga4gh-cli/config.json:
# config.from_file(ServiceType.TES)

print(ServiceInfo(config).get_service_info())

tes = TES(config)
task = tes.create("../examples/tes-hello.json")
print(task.id, task.status())

for t in tes.list(None).tasks:
    print(t.id, t.status)

print(tes.get("FULL", task.id).status)
```

`TES.create()` accepts a `dict`, a `TesTask`, or a path to a JSON task file.
Use `TesTask.from_json(text)` when the task starts as JSON text.
For a task that is still running, call `task.cancel()`.
`from_file` replaces the API root. `Task.status()` and `Task.cancel()` are methods;
`id` and `status` on tasks returned by `list()` and `get()` are attributes.

## Network behavior

Synchronous network methods release the Python GIL while Rust waits for I/O.
They share a process-lifetime multi-thread Tokio runtime, so a client's HTTP
connections remain reusable between calls. Inputs are decoded before releasing
the GIL and Python results/exceptions are constructed after reacquiring it.
No Python callback runs on the runtime. The [asyncio](#asyncio) classes run
their requests on the same runtime. Use multiprocessing's `spawn` mode rather
than forking a process after starting threaded I/O.

`tests/test_io_runtime.py` exercises all synchronous network wrappers against a
Python-thread HTTP server, including a heartbeat during delayed I/O, keep-alive
connection reuse, and error recovery. Each scenario runs in a subprocess with a
15-second timeout so a GIL regression cannot hang the test worker.

`Configuration.from_file()` raises `FileNotFoundError` when the config file is
missing and `ValueError` when its contents are malformed. Invalid URLs and JSON
arguments also raise `ValueError`.

## Using TES

`TES` is a client for a Task Execution Service. It takes tasks as `dict`s, so Python apps
need neither task files on disk nor JSON parsing:

```python
from GA4GH import Configuration, ListTasksParams, TES, TesState, TesTask

tes = TES(Configuration("http://localhost:8000/ga4gh/tes/v1"))

task = tes.create({
    "name": "hello",
    "executors": [{"image": "alpine", "command": ["echo", "hello"]}],
})
print(task.id)

if task.status() is TesState.QUEUED:
    task.cancel()

details = tes.get("FULL", task.id)
print(details.state, details.executors, details.to_dict())

page = tes.list(ListTasksParams(state=TesState.RUNNING, page_size=50))
for listed in page.tasks:
    print(listed.id, listed.state)
```

- `TES.create(task)` takes a `dict` (or other mapping), a `TesTask`, or the path of a JSON
  file (`str` or `os.PathLike`). A `str` is always a path, unlike for `TesTask(task)`; pass
  JSON text as `TesTask.from_json(text)`. A `str` that starts with `{` raises `ValueError`;
  pass such a path as a `pathlib.Path`. It sends the task's JSON as given, including
  fields that the TES models don't know, and returns a `Task`.
- `Task` addresses a task on the server by its `id`. `Task.status()` returns the current
  state as a `TesState`; `Task.cancel()` cancels the task and returns `None`.
- `TES.get(view, id)` returns a `TesTask`; `view` is `"MINIMAL"`, `"BASIC"` or `"FULL"`
  (case-insensitive). Invalid views raise `ValueError` before a request.
- `TES.list(params=None)` returns a `ListTasksResponse` with `tasks` (a list of
  `TesTask`) and `next_page_token`; see [Listing tasks](#listing-tasks).
- Errors: invalid tasks and arguments raise `ValueError`, a missing task file
  `FileNotFoundError` (an `OSError`). Failed requests and responses that don't fit the TES
  models raise `RuntimeError`. If task creation fails after submission, the error may
  mean the server created the task but its acknowledgement was lost or malformed;
  do not retry automatically.

### Task objects

`TesTask` holds a task: one to create, or one as returned by the server. Build one with
`TesTask(task)` from a `dict` or JSON text, or with `TesTask.from_dict(d)`,
`TesTask.from_json(text)` or `TesTask.from_file(path)`; all of them check the task
against the TES models and raise `ValueError` if it doesn't fit. The classmethods call
`cls(task)`, so further constructor arguments of a subclass need defaults. Tasks can be
pickled and copied, including the attributes of subclass instances.

Its fields are properties: `id`, `state`, `name`, `description`, `creation_time`,
`inputs`, `outputs`, `resources`, `executors`, `volumes`, `tags` and `logs`. Missing
fields are `None`, except `state`, which is `TesState.UNKNOWN` then. Nested values, such
as executors, come as `dict`s and `list`s. The `status` property is the same as `state` and
is kept for compatibility. Data objects such as `TesTask` have properties; `Task`, which
asks the server, has methods (`Task.status()`).

`task.to_dict()` returns the whole task as a new `dict` and `task.json()` as JSON text,
both including fields that the TES models don't know. `ListTasksResponse` has the same
two methods for the whole page.

### Task states

`TesState` is a Python `enum.Enum` of the TES states (`UNKNOWN`, `QUEUED`,
`INITIALIZING`, `RUNNING`, `PAUSED`, `COMPLETE`, `EXECUTOR_ERROR`, `SYSTEM_ERROR`,
`CANCELED`, `PREEMPTED`, `CANCELING`). Its members are also strings:
`TesState.RUNNING == "RUNNING"`, and `str()`, f-strings and `json.dumps()` give the
state's name. `TesState("RUNNING")` looks a state up by name. Members can be pickled.

### asyncio

`AsyncTES` and `AsyncTask` are the TES client and task for `asyncio` apps. They have the
same methods as `TES` and `Task`, take the same arguments and give the same results
(`TesTask`, `ListTasksResponse`, `TesState`), but each method returns an awaitable:

```python
import asyncio

from GA4GH import AsyncTES, Configuration, TesState


async def main():
    tes = await AsyncTES.connect(Configuration("http://localhost:8000/ga4gh/tes/v1"))
    task = await tes.create({"executors": [{"image": "alpine", "command": ["echo", "hello"]}]})
    if await task.status() is TesState.QUEUED:
        await task.cancel()
    details, page = await asyncio.gather(tes.get("FULL", task.id), tes.list())
    print(details.to_dict(), [listed.id for listed in page.tasks])


asyncio.run(main())
```

- Like `TES(config)`, an `AsyncTES` reads the service info and checks that the service
  is a TES before its first request. `AsyncTES(config)` doesn't contact the server: its
  first call checks the service, once for concurrent calls. If the check fails, the
  calls that waited for it raise its `RuntimeError`, and a later call tries again.
  `await AsyncTES.connect(config)` checks it right away and gives the
  client. On a subclass, `connect()` builds `cls(config, *args, **kwargs)` at once, in
  the caller's thread and context, and gives it once the check is done.
- `AsyncTES.create()` gives an `AsyncTask`; `AsyncTask(id, transport)` addresses an
  existing task, like `Task`. `config`, `transport` and `id` are plain properties.
- The requests run on the runtime of the sync API without the GIL, so they don't block
  the event loop, and several of them run at the same time. Because the SDK's futures
  aren't `Send` yet, each pending call takes a thread of a pool of its own: up to 256
  calls run at once, and further ones wait for a thread (cancelled ones never start).
- The methods return `asyncio.Future`s of the running event loop, so call them from a
  coroutine or a callback of the loop; outside of one, they raise `RuntimeError`.
- Cancelling an awaitable, for example with `asyncio.wait_for()` or
  `asyncio.Task.cancel()`, returns at once and drops its request: the connection is
  closed. Whether the server already acted on a request that was sent is unknown then.
- Invalid arguments raise `ValueError` (or `OSError` for task files) when the method is
  called; failed requests and invalid responses raise `RuntimeError` when awaited.
- `transport` gives a `Transport` for further endpoints. Its methods are synchronous:
  they release the GIL but block the event loop while a request runs, so run them with
  `await asyncio.to_thread(...)` in a coroutine. There is no async `Transport` yet.

### Type hints

The package ships type stubs (`GA4GH.pyi`, with a `py.typed` marker), so editors and
type checkers such as mypy see every class and method. `tests/test_stubs.py` checks that
the stubs declare every class and member of the module.

## Using WES

`WES` is a client for a Workflow Execution Service (WES 1.1). Like `TES`, it takes run
requests as `dict`s, and workflow files as `bytes` or paths:

```python
from pathlib import Path
from GA4GH import Configuration, WES, WesState

wes = WES(Configuration("https://your-wes.example/ga4gh/wes/v1"))

run = wes.run(
    {
        "workflow_type": "CWL",
        "workflow_type_version": "v1.2",
        "workflow_url": "main.cwl",
        "workflow_params": {"message": "hello"},
        "tags": {"project": "demo"},
    },
    attachments={"main.cwl": Path("main.cwl"), "tools/echo.cwl": b"cwlVersion: v1.2\n..."},
)
print(run.id)

if run.status() is WesState.RUNNING:
    run.cancel()

details = wes.get(run.id)  # or run.log()
print(details.state, details.outputs, details.to_dict())

page = wes.list(page_size=50)
for summary in page.runs:
    print(summary.run_id, summary.state)
more = wes.list(page_token=page.next_page_token)

tasks = run.tasks(page_size=10)
for task in tasks.task_logs:
    print(task.id, task.name, task.exit_code)
print(run.task(tasks.task_logs[0].id).stderr)
```

- `WES(config)` reads the service info and raises `RuntimeError` if the service isn't
  reachable or isn't a WES.
- `WES.run(request, attachments=None)` starts a run and returns a `Run`. `request` is a
  `dict` (or other mapping), a `RunRequest`, or the path of a JSON file (`str` or
  `os.PathLike`); as for `TES.create()`, a `str` is always a path, so pass JSON text as
  `RunRequest.from_json(text)`. `workflow_type`, `workflow_type_version`, `workflow_url`
  and `workflow_params` are required; `workflow_params` must be a JSON object.
  The request goes to the server as the fields of
  the WES `RunRequest` form; fields that the WES models don't know are kept in the
  `RunRequest` but not sent.
- `attachments` maps filenames to contents, as `bytes` (or `bytearray`) or as the path of
  a file (`os.PathLike`, such as `pathlib.Path`), which is streamed rather than read into
  memory. A list of `(filename, content)` pairs works too. Filenames are relative paths
  with `/` separators, such as `tools/echo.cwl`; absolute paths, `..`, backslashes,
  colons and repeated filenames raise `ValueError` before a request. Refer to an
  attachment in `workflow_url` by its filename. The upload and the server's answer must
  finish within 60 seconds.
- `Run` addresses a run on the server by its `id`. `Run.status()` returns a `WesState`,
  `Run.cancel()` cancels the run and returns `None`, and `Run.log()` returns a `RunLog`.
  `Run.tasks(page_size=None, page_token=None)` returns a `TaskListResponse` with
  `task_logs` (a list of `TaskLog`) and `next_page_token`; `Run.task(task_id)` returns one
  `TaskLog`. The task endpoints are new in WES 1.1.
- `WES.get(id)` returns a `RunLog`, like `Run(id, wes.transport).log()`.
- `WES.list(page_size=None, page_token=None)` returns a `RunListResponse` with `runs` (a
  list of `RunSummary`) and `next_page_token`. A bare array of runs, as WES 0.2.0 servers
  (such as the GA4GH Starter Kit) send it, becomes a page without a token.
- Errors: invalid requests, attachments and arguments (such as an empty ID, `.` or
  `..`) raise `ValueError`; request or attachment files that are missing or unreadable
  raise an `OSError` with the path, such as `FileNotFoundError`. Failed requests and responses that don't fit the WES models raise `RuntimeError`. If
  starting a run fails after submission, the error may mean the server started the run
  but its acknowledgement was lost or malformed; do not retry automatically.

`RunRequest` holds a run request, like `TesTask` holds a task: build one with
`RunRequest(request)` from a `dict` or JSON text, or with `RunRequest.from_dict(d)`,
`RunRequest.from_json(text)` or `RunRequest.from_file(path)`. Unlike `WES.run()`, which
reads a `str` as a path, `RunRequest(text)` parses a `str` as JSON. Its fields are properties:
`workflow_type`, `workflow_type_version`, `workflow_url`, `workflow_params`, `tags`,
`workflow_engine`, `workflow_engine_version` and `workflow_engine_parameters`. Requests
can be pickled and copied.

The results are typed objects with properties, and keep the JSON that the server sent,
including fields that the WES models don't know: `to_dict()` returns it as a new `dict`
and `json()` as JSON text.

- `RunLog`: `run_id`, `request` (a `RunRequest`, or `None`), `state`, `run_log`,
  `task_logs_url`, `task_logs` and `outputs`.
- `RunSummary`: `run_id`, `state`, `start_time`, `end_time` and `tags`.
- `TaskLog`: `id`, `name`, `cmd`, `start_time`, `end_time`, `stdout`, `stderr`,
  `exit_code`, `system_logs` and `tes_uri`.

Missing fields are `None`, except `state`, which is `WesState.UNKNOWN` then. Nested
values, such as `run_log`, come as `dict`s and `list`s. `WesState` is a `str` enum of the
WES states (`UNKNOWN`, `QUEUED`, `INITIALIZING`, `RUNNING`, `PAUSED`, `COMPLETE`,
`EXECUTOR_ERROR`, `SYSTEM_ERROR`, `CANCELED`, `CANCELING`, `PREEMPTED`) that works like
`TesState`, but is a separate class.

To read a WES section of `~/.ga4gh-cli/config.json`, use
`config.from_file(ServiceType.WES)`.

The checked-in [WES example](examples/wes_run.py) uploads a workflow file, polls the
run and prints its details. Run it from `python/` against a WES service:

```sh
python examples/wes_run.py https://your-wes.example/ga4gh/wes/v1 main.cwl CWL v1.2
```

The Python test suite runs this same file against a mock service.

## Runnable example

After installing the bindings with `maturin develop`, run the checked-in
[Python script](tests/example.py) from `python/` with Funnel and Docker running:

```sh
python tests/example.py
```

The output depends on server timing, for example:

```text
TASK ID: <task-id>
STATE:   COMPLETE
TASK:    {'id': '<task-id>', 'state': 'COMPLETE', ...}
```

The ID, state, and number of listed tasks depend on the server and timing.
The script creates a task from a Python `dict` and also works when invoked
from the repository root. Set `GA4GH_TES_URL` to use another TES API root.
CI runs this exact script against a mock service and checks the submitted task body.

## Extending the bindings

The bindings are meant as a base for clients of services that add endpoints or
fields to the GA4GH APIs. `Configuration`, `ServiceInfo`, `Transport`, `TES`,
`Task`, `TesTask`, `ListTasksParams`, and `ListTasksResponse` can be subclassed,
as can `WES`, `Run`, `RunRequest`, `RunLog`, `RunSummary`, `RunListResponse`,
`TaskLog`, and `TaskListResponse`.
`ServiceType`, `TesState`, and `WesState` are enums and are not subclassable.

The base classes initialize in `__new__`. A subclass that needs extra
constructor arguments can define `__init__` and call `super().__init__()`
without arguments. For `TES` and `WES`, service-info discovery happens before the
subclass's `__init__`, so those extra arguments cannot change its first
request. A subclass without its own `__init__` accepts only the base arguments.

### Further endpoints

`Transport` sends requests below the configured API root and returns the
response body as text. `TES` and `Task` expose their `.transport`, so a
subclass reuses the client's configuration. The checked-in
[extension example](examples/tes_extension.py) adds a service-specific
`provenance` endpoint, encodes the task ID as one URL path segment, and parses
the response as JSON. Run it from `python/` against a TES service that provides
that extra endpoint:

```sh
python examples/tes_extension.py https://your-server.example/ga4gh/tes/v1 task-id
```

The Python test suite runs this same file against a mock service.

The methods are `get(endpoint, params=None)`, `post(endpoint, data=None, params=None)`,
`put(endpoint, data, params=None)`, `delete(endpoint, params=None)` and
`post_multipart(endpoint, fields=None, files=None, params=None)`:

- `endpoint` is relative to the API root, for example `tasks/123`. Encode IDs that you
  put into it yourself, for example with `urllib.parse.quote(task_id, safe="")`.
- `data` is any object that `json.dumps` accepts. A `str` is taken as JSON text, so
  pass `json.dumps(text)` to send a JSON string.
- `params` holds query parameters, as a `dict` or a JSON object string. Values are
  strings, numbers, booleans or lists of these; a list repeats the parameter
  (`{"tag_key": ["a", "b"]}` gives `tag_key=a&tag_key=b`), and `None` leaves it out.
- `fields` is a list of `(name, value)` text parts; `files` is a list of
  `(name, filename, content)` or `(name, filename, content, content_type)` tuples, with
  `content` as `bytes`. Names may repeat, as for WES `workflow_attachment`.
  This illustrates the multipart API; to start a WES run, use
  [`WES.run()`](#using-wes) instead:

```python
from pathlib import Path
from GA4GH import Configuration, Transport

transport = Transport(Configuration("https://your-wes.example/ga4gh/wes/v1"))
transport.post_multipart(
    "runs",
    fields=[("workflow_type", "CWL"), ("workflow_type_version", "v1.2")],
    files=[("workflow_attachment", "main.cwl", Path("main.cwl").read_bytes())],
)
```

A failed request raises `RuntimeError`; invalid values raise `ValueError`, and arguments
of the wrong type (for example a `dict` for `fields`) raise `TypeError`.

### Fields the models don't know

`TES.get()` and the tasks of `TES.list()` are `TesTask` objects. Besides their typed
fields, they keep the task's JSON as the server sent it, including fields that the TES
models don't know: `task.json()` returns it as JSON text (object keys may come in another
order), `task.to_dict()` as a `dict`. `TES.create()` sends such fields as given, too.
`TesTask` was called `PyTesTask` before; the old name still works.

### Listing tasks

`ListTasksParams` takes the filters of the TES `ListTasks` request as keyword arguments:

```python
from GA4GH import ListTasksParams

params = ListTasksParams(state="running", tag_key=["project"], tag_value=["demo"], page_size=50)
page = tes.list(params)
for task in page.tasks:
    print(task.id, task.state)
next_page = tes.list(ListTasksParams(page_token=page.next_page_token))
```

`state` accepts a `TesState` or any TES state name in any case, and raises `ValueError`
for others; the `state` property returns a `TesState`.
`view` accepts `"MINIMAL"`, `"BASIC"` or `"FULL"` in any case; the property and
request use the uppercase TES spelling. Invalid views raise `ValueError` before a request.

### Rust crates

The `ga4gh` crate in this directory builds as a Python module and as a Rust library. A
PyO3 crate (same PyO3 version) can depend on it and extend the classes in Rust, for
example with `#[pyclass(extends = PyTES)]`; the `inner` field of each class holds the
library type (for `PyAsyncTES`, a `LazyTES` that connects on first use). The TES calls
that the sync and the async classes share are public functions in `clients::tes`, such as
`get_task()` and `task_status()`; an async extension method can run them with
`clients::tes_async::spawn_io` inside `clients::tes_async::future_into_py`, as
`clients::tes_async` does. The extension module gets its own copies of the base classes,
which don't mix with those of the `GA4GH` module: a `GA4GH.Configuration` can't be passed
to it. So register every base class that its users need
(`m.add_class::<PyConfiguration>()` and so on, and `m.add("TesState",
tes_state_type(py)?)` for the state enum) in the extension module, and import everything
from there. The default feature `extension-module` builds the Python module; a dependent
crate can turn it off with `default-features = false` and enable it in its own features
instead.

The WES classes (`PyWES`, `PyRun` and so on, in `clients::wes`) follow the same pattern;
`clients::wes::register(py, m)` registers all of them, with the `WesState` enum. Further
bindings, such as DRS and TRS, will too.

## Documentation

For more information on PyO3, visit the [PyO3 documentation](https://docs.rs/pyo3/latest/pyo3).

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.
