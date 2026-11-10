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
- Follow the root guide's Rust checks for changes to Rust code or the workspace
  dependency graph.

The test suite protects the Python behavior already claimed in the guide. It
cannot detect a new Rust API that nobody has chosen to bind; the coverage
decision above is part of review.
