# Downstream SDK example

An example of a crate that builds on this SDK through its public APIs: a
client for an additional endpoint, a model that extends `TesTask`, and a CLI
with its own name, configuration file and token variable and added
subcommands. The guide [Build on this SDK](../../docs/downstream.md) walks
through it.

Check it from the repository root with `make downstream`.

The `workflow-prepare` example opens a trusted installed provider, prepares
the sequence-QC request in memory, prints its fingerprint and releases the
plan without WES configuration. See the command in the downstream guide.
