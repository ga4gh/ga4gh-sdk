# Python bindings work

Read the [root guide](../AGENTS.md) and the [Python bindings guide](README.md).
This file applies to work under `python/` and to review of Rust SDK changes that
may affect Python users.

## Keep the supported API explicit

The PyO3 wrappers are maintained by hand. New Rust methods and generated models
do not appear in Python automatically. For a changed SDK client or model, check
the [supported surface](README.md#supported-surface). Decide whether Python
should expose the new behavior now; update the wrapper, table, and examples, or
record that it remains Rust-only. Do not infer full Rust/Python feature parity
from a successful build.

Trace the user path through Python arguments, the PyO3 wrapper, the Rust client,
and the HTTP request or response. Keep endpoint paths relative to the configured
API root. Preserve useful Python exception types and the full response data
where the public API promises it. Synchronous network calls must continue to
release the GIL and use the shared runtime in `src/utils/runtime.rs`.

## Lay out a service binding

A service binding with more than one class lives in a folder under
`src/clients/`, as `drs/` and `wes/` do:

- `mod.rs` has the module docs, which list the files, and `register()`, which
  adds the classes to the Python module. It re-exports every class and every
  other public item with `pub use`.
- One file per topic holds the classes, such as `client.rs` for the client and
  one file for each group of result classes. These submodules are private
  (`mod`, not `pub mod`), so the re-exports in `mod.rs` stay the only Rust paths
  for extension crates, such as `clients::wes::PyWES`.
- `helpers.rs` holds the functions that only this binding's files share, as
  `pub(super)`; it defines no Python classes.
- Helpers that another binding needs in the same form go to `src/utils/`
  instead, such as `utils::json` (`field()`, `invalid_response()`,
  `parse_body()`) and `utils::validate` (`require_id()`). Pass the API name
  where a message names it, as in `Invalid WES response: …`.

Moving code between these files must not change the Python API. Keep the doc
comments word for word: PyO3 turns them into the docstrings that `help()`
shows. Keep the order of `register()`, which is the order of the classes in the
Python module, the `name` and `module` of each `#[pyclass]`, the type stub
`ga4gh_sdk.pyi` and the error messages. Bindings that are still single files,
such as `tes.rs`, move to this layout one API at a time.

## Verify user behavior

- Build the bindings in a virtual environment as described in `README.md`.
- Run `python -m pytest` from `python/`. The mock-backed tests cover public
  methods, error paths, subclassing, and the checked-in examples. Add focused
  tests for observable behavior when extending that surface.
- Keep commands, import names, task files, and paths in the guide executable
  from the documented working directory. Test a user-facing example when it
  changes.
- If the Rust extension API changes, compile a small downstream PyO3 crate with
  `default-features = false`; a workspace build alone does not prove that use.
- When code moves between binding files, compare the `__doc__`, member docs,
  text signatures and `__module__` of the affected classes in wheels built from
  `main` and from the branch; the two must be identical. For example, unpack
  each wheel, print `cls.__module__`, `cls.__doc__`, `cls.__text_signature__`
  and, for each entry of `vars(cls)`, its `__doc__` and `__text_signature__`,
  and `diff` the two outputs.
- Follow the root guide's Rust checks for changes to Rust code or the workspace
  dependency graph.

The test suite protects the Python behavior already claimed in the guide. It
cannot detect a new Rust API that nobody has chosen to bind; the coverage
decision above is part of review.
