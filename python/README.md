## GA4GH-SDK Python Bindings

This directory provides Python bindings for the GA4GH SDK using [PyO3](https://docs.rs/pyo3/latest/pyo3).
Start setup from the repository root; after `cd python`, run later commands
from `python/` with the virtual environment active.

## Prerequisites

- Python 3.12 or newer
- [Maturin](https://github.com/PyO3/maturin)
- [PyTest](https://docs.pytest.org/en/stable/)

## Installation

1. **Enter the Python package and create a virtual environment:**

    ```bash
    cd python
    python3 -m venv .venv
    . .venv/bin/activate
    ```

    Use a Python 3.12+ executable if `python3` points to an older version.

2. **Install dependencies:**

    ```bash
    python -m pip install 'maturin>=1.7,<2' pytest
    ```

## Development

To start development, use the following command to build and install the package in development mode:

```bash
maturin develop
```

## Build

Build the Python library wheel:

```bash
maturin build
```

## Running Tests

After `maturin develop`, run the Python binding tests with:

```bash
pytest
```

`Configuration.from_file()` raises `FileNotFoundError` when the config file is
missing and `ValueError` when its contents are malformed. Invalid URLs and JSON
arguments also raise `ValueError`.

## Example 

You can try an example Python app that leverages the GA4GH-SDK Python library to create task, get its ID and status, as well as retrieveing the list of all tasks from the configured TES instance.

```bash
python3 ./tests/example.py
```

The output should look like:

```bash
Creating task...Task created successfully:
TASK ID:  csg0d0ljt0vv1kstlqg0
STATUS:   Queued
Canceling task...
Task canceled successfully

Listing tasks...
TASK ID                   STATUS         
csg0d0ljt0vv1kstlqg0      COMPLETE       
csfvvidjt0vv1kstlqfg      COMPLETE       
csfvuhdjt0vv1kstlqf0      INITIALIZING   
csfv61djt0vv1kstlqeg      CANCELED       
csfv5ktjt0vv1kstlqe0      RUNNING        
csfv5fdjt0vv1kstlqdg      RUNNING        
csfv54ljt0vv1kstlqd0      CANCELED       
csfv4m5jt0vv1kstlqcg      QUEUED         
csfv455jt0vv1kstlqc0      QUEUED         
csfv3o5jt0vv1kstlqbg      SYSTEM_ERROR   
```

## Extending the bindings

The bindings are meant as a base for clients of services that add endpoints or fields to
the GA4GH APIs. All classes can be subclassed.

### Further endpoints

`Transport` sends requests below the configured API root and returns the response body
as text. `TES` and `Task` give theirs as `.transport`, so a subclass reuses the client's
configuration:

```python
import json

from GA4GH import Configuration, TES, Transport


class MyTES(TES):
    def provenance(self, task_id):
        return json.loads(self.transport.get(f"tasks/{task_id}/provenance"))


class MyTransport(Transport):
    def enable_plugin(self, plugin_id):
        return self.put(f"plugins/{plugin_id}", {"enabled": True}, params={"force": True})


config = Configuration("http://localhost:8000/ga4gh/tes/v1")
print(MyTES(config).provenance("task-id"))
```

The methods are `get(endpoint, params=None)`, `post(endpoint, data=None, params=None)`,
`put(endpoint, data, params=None)`, `delete(endpoint, params=None)` and
`post_multipart(endpoint, fields=None, files=None, params=None)`:

- `endpoint` is relative to the API root, for example `tasks/123`. Encode IDs that you
  put into it yourself, for example with `urllib.parse.quote(task_id, safe="")`.
- `data` is a JSON string or any object that `json.dumps` accepts.
- `params` holds query parameters, as a `dict` or a JSON object string. Values are
  strings, numbers, booleans or lists of these; a list repeats the parameter
  (`{"tag_key": ["a", "b"]}` gives `tag_key=a&tag_key=b`), and `None` leaves it out.
- `fields` is a list of `(name, value)` text parts; `files` is a list of
  `(name, filename, content)` or `(name, filename, content, content_type)` tuples, with
  `content` as `bytes`. Names may repeat, as for WES `workflow_attachment`:

```python
transport.post_multipart(
    "runs",
    fields=[("workflow_type", "CWL"), ("workflow_type_version", "v1.2")],
    files=[("workflow_attachment", "main.cwl", open("main.cwl", "rb").read())],
)
```

A failed request raises `RuntimeError`; invalid arguments raise `ValueError`.

### Fields the models don't know

`TES.get()` and the tasks of `TES.list()` are `TesTask` objects. Besides `id` and
`status`, they keep the task as the server sent it, including fields that the TES models
don't know: `task.json()` returns it as JSON text, `task.to_dict()` as a `dict`.
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
example with `#[pyclass(extends = PyTES, unsendable)]`; the `inner` field of each class
holds the library type. The extension module gets its own copies of the base classes,
which don't mix with those of the `GA4GH` module: a `GA4GH.Configuration` can't be passed
to it. So register every base class that its users need (`m.add_class::<PyConfiguration>()`
and so on) in the extension module, and import everything from there.

Further bindings in this crate, such as WES, DRS and TRS, follow the same pattern.

## Documentation

For more information on PyO3, visit the [PyO3 documentation](https://docs.rs/pyo3/latest/pyo3).

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.
