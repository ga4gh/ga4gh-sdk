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

## Documentation

For more information on PyO3, visit the [PyO3 documentation](https://docs.rs/pyo3/latest/pyo3).

## Contributing

Contributions are welcome! Please open an issue or submit a pull request.
