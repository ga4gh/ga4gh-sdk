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

## Usage

The module is called `GA4GH` and currently covers TES and service info.
`Configuration` takes the API root, including the service's path prefix (see
the [usage guide](../docs/usage.md#api-root-base_path)):

```python
import json
from GA4GH import Configuration, ServiceInfo, ServiceType, TES

config = Configuration("http://localhost:8000/ga4gh/tes/v1")
# Or read the TES section of ~/.ga4gh-cli/config.json:
# config.from_file(ServiceType.TES)

print(ServiceInfo(config).get_service_info())

tes = TES(config)
task = tes.create(json.dumps({"executors": [{"image": "alpine", "command": ["true"]}]}))
print(task.id, task.status())

for t in tes.list(None).tasks:
    print(t.id, t.status)

print(tes.get("FULL", task.id).status)
task.cancel()
```

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

## Documentation

For more information on PyO3, visit the [PyO3 documentation](https://docs.rs/pyo3/latest/pyo3).

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.
