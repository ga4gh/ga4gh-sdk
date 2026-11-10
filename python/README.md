## GA4GH-SDK Python Bindings

This directory provides Python bindings for the GA4GH SDK using [PyO3](https://docs.rs/pyo3/latest/pyo3).
These instructions build the bindings from this repository. Start at the
repository root; after `cd python`, run later commands from `python/` with the
virtual environment active.

## Prerequisites

- Python 3.12 or newer
- The [Rust toolchain](../README.md#installation)

## Installation

From the repository root, create an environment, build the bindings, and check
the import:

```sh
cd python
python3.12 -m venv .venv
. .venv/bin/activate
python -m pip install 'maturin>=1.7,<2'
maturin develop --locked
python -c 'from GA4GH import TES; print(TES.__name__)'
```

Use another Python 3.12+ executable if `python3.12` is unavailable. The
distribution is named `GA4GH-SDK`; the import name is `GA4GH`.

## Build a wheel

Build the Python library wheel:

```bash
maturin build --locked
```

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
| DRS, WES, TRS | Available in the Rust SDK, not yet as Python client classes. |

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

`TES.create()` accepts a path to a JSON task file or the JSON text itself.
For a task that is still running, call `task.cancel()`.
`from_file` replaces the API root. `Task.status()` and `Task.cancel()` are methods; `id`
and `status` on the tasks returned by `list()` and `get()` are attributes.

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

## Runnable example

After installing the bindings with `maturin develop`, run the checked-in
[Python script](tests/example.py) from `python/` with Funnel and Docker running:

```sh
python tests/example.py
```

It submits the same file-based hello task, prints its ID and current status,
and lists tasks. For example:

```text
Created task <task-id>: Queued
Tasks:
<task-id>: QUEUED
```

The ID, state, and number of listed tasks depend on the server and timing.
The script finds its task file relative to itself, so it also works when
invoked from the repository root. Set `GA4GH_TES_URL` to use another TES API
root. CI runs this exact script against a mock service and checks the submitted
task body.

## Extending the bindings

The bindings are meant as a base for clients of services that add endpoints or
fields to the GA4GH APIs. `Configuration`, `ServiceInfo`, `Transport`, `TES`,
`Task`, `TesTask`, `ListTasksParams`, and `ListTasksResponse` can be subclassed.
`ServiceType` is an enum and is not subclassable.

The base classes initialize in `__new__`. A subclass that needs extra
constructor arguments can define `__init__` and call `super().__init__()`
without arguments. For `TES`, service-info discovery happens before the
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

`TES.get()` and the tasks of `TES.list()` are `TesTask` objects. Besides `id` and
`status`, they keep the task's JSON as the server sent it, including fields that the TES
models don't know: `task.json()` returns it as JSON text (object keys may come in another
order), `task.to_dict()` as a `dict`.
`TesTask` was called `PyTesTask` before; the old name still works.

### Listing tasks

`ListTasksParams` takes the filters of the TES `ListTasks` request as keyword arguments:

```python
from GA4GH import ListTasksParams

params = ListTasksParams(state="running", tag_key=["project"], tag_value=["demo"], page_size=50)
page = tes.list(params)
for task in page.tasks:
    print(task.id, task.status)
next_page = tes.list(ListTasksParams(page_token=page.next_page_token))
```

`state` accepts any TES state in any case and raises `ValueError` for others.

### Rust crates

The `ga4gh` crate in this directory builds as a Python module and as a Rust library. A
PyO3 crate (same PyO3 version) can depend on it and extend the classes in Rust, for
example with `#[pyclass(extends = PyTES)]`; the `inner` field of each class
holds the library type. The extension module gets its own copies of the base classes,
which don't mix with those of the `GA4GH` module: a `GA4GH.Configuration` can't be passed
to it. So register every base class that its users need (`m.add_class::<PyConfiguration>()`
and so on) in the extension module, and import everything from there. The default
feature `extension-module` builds the Python module; a dependent crate can turn it off
with `default-features = false` and enable it in its own features instead.

Further bindings in this crate, such as WES, DRS and TRS, follow the same pattern.

## Documentation

For more information on PyO3, visit the [PyO3 documentation](https://docs.rs/pyo3/latest/pyo3).

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.
