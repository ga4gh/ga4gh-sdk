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
| `Transport` | Send relative HTTP requests with JSON, query parameters, or multipart bodies; responses are text. |
| `DRS`, `DrsResolver` | Read DRS objects and access URLs, one at a time or in bulk, wait while the server prepares them, and plan access for `drs://` URIs; read full JSON including unknown fields. See [Using DRS](#using-drs). |
| WES, TRS, Service Registry | Available in the Rust SDK, not yet as Python client classes. |

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
No asyncio API or Python callback runs on the runtime. Use multiprocessing's
`spawn` mode rather than forking a process after starting threaded I/O.

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

### Type hints

The package ships type stubs (`GA4GH.pyi`, with a `py.typed` marker), so editors and
type checkers such as mypy see every class and method. `tests/test_stubs.py` checks that
the stubs declare every class and member of the module.

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

## Using DRS

`DRS` is a client for a Data Repository Service (DRS 1.4). It reads object metadata and
access URLs, one at a time or in bulk. It never downloads object bytes:

```python
from GA4GH import Configuration, DRS, PendingResolution

drs = DRS(Configuration("https://your-drs.example/ga4gh/drs/v1"))

obj = drs.get_object("my-object-id")
print(obj.id, obj.size, obj.checksums, obj.to_dict())
for method in obj.access_methods or []:
    print(method["type"], method.get("access_id"))

access = drs.get_access_url("my-object-id", "my-access-id")
# Fetch the bytes from access.url with access.headers; don't log signed URLs.

# The server may still be preparing an answer (202 Accepted):
answer = drs.get_object_once("my-object-id")
if isinstance(answer, PendingResolution):
    print("ask again in", answer.retry_after, "seconds")
obj = drs.resolve_object("my-object-id", timeout=120, max_attempts=10)

report = drs.get_bulk_objects(["id-1", "id-2"])
for outcome in report.outcomes:
    if outcome.is_success:
        print(outcome.object_id, outcome.resolved[0].size)
    else:
        print(outcome.object_id, outcome.failure_codes, outcome.pending, outcome.errors)
urls = drs.get_bulk_access_urls({"id-1": "my-access-id"})
```

- `DRS(config)` reads the service info and raises `RuntimeError` if the service isn't
  reachable or isn't a DRS.
- `DRS.get_object(object_id)` returns a `DrsObject`. `DRS.get_access_url(object_id,
  access_id)` returns an `AccessURL`; `access_id` is the `access_id` of one of the
  object's `access_methods`. IDs are sent as one URL path segment each.
- A DRS server can answer `202 Accepted` with a `Retry-After` header while it prepares
  an object or URL. `get_object()` and `get_access_url()` then raise `RuntimeError`.
  `get_object_once()` and `get_access_url_once()` send one request and return a
  `PendingResolution` instead, with `retry_after` (seconds), `endpoint` and `api_root`
  (without credentials). `resolve_object()` and `resolve_access_url()` wait and ask
  again as the server says, up to `max_attempts` requests (default 5) and `timeout`
  seconds in all (default 60), then raise `RuntimeError`.
- `DRS.get_bulk_objects(ids, passports=None, policy=None, timeout=60.0)` asks for several
  objects with `POST objects`; `DRS.get_bulk_access_urls(pairs, ...)` asks for access
  URLs with `POST objects/access`. `pairs` maps object IDs to access IDs, or is a list of
  `(object_id, access_id)` pairs, with one access ID per object per call. Repeated IDs
  are asked for once. `passports` is a list of GA4GH Passport tokens to send in the
  request body. Both calls return a `BulkReport` with one `BulkOutcome` per distinct ID
  (or pair), sorted by ID, and `diagnostics` for problems with the server's answer that
  belong to no ID.
- A `BulkOutcome` has `object_id`, `access_id`, `resolved` (the `DrsObject`s or
  `BulkAccessURL`s that the server returned for the ID), `failure_codes` (such as
  `[404]`), `pending` (a `PendingResolution` if the server is still preparing the
  answer), `errors` and `is_success` (exactly one result and nothing else). Failures of
  single IDs and of requests are reported in the outcomes, not raised. For a pending
  outcome, call the bulk method again for its IDs; its `endpoint` is the bulk endpoint.
- `BulkPolicy(chunk_size=100, advertised_limit=None, concurrency=4, max_requests=1000,
  max_split_depth=8, max_attempts=1, initial_delay=0.2, max_delay=10.0)` limits a bulk
  call: IDs per request, parallel requests, requests in all, halvings of a request that
  the server rejects as too large (413), and retries of network errors, 429 and 503.
  `policy.with_service_info(info)` also keeps to the `maxBulkRequestLength` of the
  service info, for example `json.loads(drs.transport.get("service-info"))`.
- Errors: invalid IDs (empty, `.` or `..`), timeouts, attempt counts, bulk inputs and
  policies raise `ValueError` before a request. Failed requests and responses that don't
  fit the DRS models raise `RuntimeError`; its message names a missing field, but quotes
  no values from the response, as they may be credentials.
- The waiting calls block for up to `timeout` seconds without the GIL; Ctrl+C takes
  effect after they return.

`DrsResolver` turns hostname-based DRS URIs (`drs://<host>/<object-id>`) into an
`AccessPlan`: the access method to use, its `url` and `headers` (a list of `(name,
value)` pairs, unlike the `"Name: value"` strings of `AccessURL`), and its `region`:

```python
from GA4GH import Configuration, DrsResolver, DrsUri, Transport

resolver = DrsResolver().with_service(
    "drs.example.org", Transport(Configuration("https://drs.example.org/ga4gh/drs/v1"))
)
plan = resolver.access_plan("drs://drs.example.org/my-object-id", protocols=["https", "s3"])
print(plan.method, plan.region)  # use plan.url and plan.headers to fetch the bytes

uri = DrsUri("drs://drs.example.org/my-object-id")
print(uri.authority, uri.object_id, uri.default_api_root())
```

- `with_service(authority, transport)` asks the DRS service of `transport` for URIs with
  that host, using the transport's API root and settings. Other hosts are asked
  anonymously at `https://<host>/ga4gh/drs/v1`; settings are never sent to them.
  Add services before sharing a resolver between threads: while another thread plans
  access with it, `with_service()` raises `RuntimeError`.
- `access_plan(uri, protocols=None, regions=None, method_index=None, timeout=60.0,
  max_attempts=5)` reads the object, picks an available access method (by `protocols`
  and `regions`, in order of preference, or the one at `method_index`), and reads its
  access URL if the method has none. It waits while the server prepares an answer, as
  `resolve_object()` does. `access_plan_once()` sends each request once and returns a
  `PendingResolution` instead.
- Compact identifiers (`drs://prefix:accession`) are not supported; they and malformed
  URIs raise `ValueError`. No usable access method raises `RuntimeError`.

The results are typed objects with properties, and `DrsObject`, `AccessURL` and
`BulkAccessURL` keep the JSON that the server sent, including fields that the DRS models
don't know: `to_dict()` returns it as a new `dict` and `json()` as JSON text.

- `DrsObject`: `id`, `name`, `self_uri`, `size`, `created_time`, `updated_time`,
  `version`, `mime_type`, `checksums`, `access_methods`, `contents`, `description` and
  `aliases`. Nested values, such as `access_methods`, come as `dict`s and `list`s.
- `AccessURL`: `url` and `headers` (a list of `"Name: value"` strings).
- `BulkAccessURL`: `drs_object_id`, `drs_access_id`, `url` and `headers`.

Access URLs and headers may hold credentials, such as signatures or bearer tokens. The
`repr()` of `AccessURL`, `BulkAccessURL`, `BulkOutcome` and `AccessPlan` doesn't show
them.

To read a DRS section of `~/.ga4gh-cli/config.json`, use
`config.from_file(ServiceType.DRS)`. The Rust methods behind these classes are
listed in the [usage guide](../docs/usage.md#drs); `DRS::resolve_reference`, which
resumes from a stored reference, is not bound yet.

## Extending the bindings

The bindings are meant as a base for clients of services that add endpoints or
fields to the GA4GH APIs. `Configuration`, `ServiceInfo`, `Transport`, `TES`,
`Task`, `TesTask`, `ListTasksParams`, and `ListTasksResponse` can be subclassed.
`ServiceType` and `TesState` are enums and are not subclassable.

The base classes initialize in `__new__`. A subclass that needs extra
constructor arguments can define `__init__` and call `super().__init__()`
without arguments. For `TES`, service-info discovery happens before the
subclass's `__init__`, so those extra arguments cannot change its first
request. A subclass without its own `__init__` accepts only the base arguments.

`DRS`, `DrsResolver`, `BulkPolicy` and `DrsUri` can be subclassed too. Like `TES`,
`DRS` reads the service info before a subclass's `__init__` runs.
`DrsResolver.with_service()` returns the resolver itself, so it keeps its subclass. The
result classes (`DrsObject`, `AccessURL`, `BulkAccessURL`, `PendingResolution`,
`BulkOutcome`, `BulkReport` and `AccessPlan`) allow subclasses as well, for Rust crates
that extend them, but the clients return instances of the base classes.

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
  This illustrates the multipart API; a real WES run also needs a complete
  workflow request and an existing `main.cwl` file:

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
example with `#[pyclass(extends = PyTES)]`; the `inner` field of each class
holds the library type. The extension module gets its own copies of the base classes,
which don't mix with those of the `GA4GH` module: a `GA4GH.Configuration` can't be passed
to it. So register every base class that its users need (`m.add_class::<PyConfiguration>()`
and so on, and `m.add("TesState", tes_state_type(py)?)` for the state enum) in the
extension module, and import everything from there. The default
feature `extension-module` builds the Python module; a dependent crate can turn it off
with `default-features = false` and enable it in its own features instead.

The DRS classes (`PyDRS`, `PyDrsResolver` and so on, in `clients::drs`) follow the same
pattern; `clients::drs::register(py, m)` registers all of them. Further bindings, such
as WES and TRS, will too.

## Documentation

For more information on PyO3, visit the [PyO3 documentation](https://docs.rs/pyo3/latest/pyo3).

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.
