# Exact deterministic result reuse

From the repository root, run `python3 examples/result-reuse/run.py` with the
pinned Python image already cached. The script starts two loopback TES adapters,
executes the allowlisted k-mer recipe once in provider A's container, and uses the
optional Rust SDK reuse module to replace provider B's prepared execution.
Provider B's measured submission count stays zero.

The run writes a typed result receipt and measured JSON evidence. Changed
parameters, revoked access, corrupt bytes, deleted artifacts and untrusted
provenance must each reject substitution. `--work PATH` selects a workspace that
the Docker daemon can bind mount. The adapter uses Docker context `default`;
if its daemon is unavailable, set `GA4GH_REUSE_DOCKER_CONTEXT` to a trusted
local context such as `desktop-linux`. The selected context must already
contain the pinned image.

See [the contract and commands](../../docs/result-reuse.md) for identity, current
trust/access requirements, measured evidence, prerequisites and limitations.
