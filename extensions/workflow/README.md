# Workflow CLI extension

`ga4gh-workflow-extension` provides the `ga4gh-cli workflow` commands. Use it
to check a workflow request against a pinned profile before submitting it, or
to verify and compare results you already have. Only `submit` contacts WES.
For ordinary WES operations without profile validation, use the built-in `wes`
commands instead.

## Install and enable

You need a CLI that supports `ga4gh.cli.commands` revision 1 and a trusted
native package built for the CLI's target triple. Keep the manifest beside its
library. From the unpacked package directory, run:

```sh
ga4gh-cli extension add manifest.json
ga4gh-cli extension enable ga4gh-workflow-extension
ga4gh-cli workflow --help
```

To build a package yourself, run `python3 extensions/workflow/package.py target/workflow-package`
from the repository root with Python 3 and the repository's Rust toolchain.
Use `target/workflow-package/manifest.json` for `extension add`. If the CLI is
not on your `PATH`, run `cargo run -p ga4gh-cli --` in place of `ga4gh-cli`
from the repository root.

Rebuilding in the same package directory publishes the manifest last. Earlier
libraries keep their digest-based names, so a reader that already opened the
old manifest can still install the matching library.

Install once per CLI configuration directory; profiles share the installation.
`extension list` shows the installed artifact and whether the provider is enabled.
Enabling authorizes native execution; displaying help does not load the library.
Use `ga4gh-cli extension disable ga4gh-workflow-extension` to stop new invocations.

## Commands

| Command | Required inputs | What it does |
| --- | --- | --- |
| `workflow prepare` | Profile schema, WES request, workflow identity, revision pins | Writes a validated invocation and its fingerprint as JSON. Does not submit a run. |
| `workflow submit` | Same inputs as `prepare` | Submits once via the host's WES client. Returns the run ID and invocation. Repeat `--attachment filename=local-path` to upload files. |
| `workflow results` | Profile schema, saved WES run log, run reference, revision pins | Projects the log's state and named outputs into a JSON result record. Does not fetch the log. |
| `workflow verify` | Result record, JSON pointer, local file | Checks bytes for the selected output and writes an updated record. Supply `--sha256` and `--evidence-source` together to check an expected digest. |
| `workflow compare` | Comparison policy, left result record, right result record | Writes a JSON comparison report under that policy. Does not rerun either workflow. |

The revision pins are `--profile-revision` and `--vocabulary-revision`. Use
`ga4gh-cli workflow <command> --help` for the full argument order and options.
Each input file is limited to 8 MiB; a host/extension message is limited to
64 MiB, including the request and Base64-encoded attachments. Command results
go to stdout; validation failures produce diagnostics on stderr and a nonzero
exit status.

## Try it without a WES service

With the extension enabled, run this from the repository root:

```sh
pin=45d41324353cb4900fdd999c307c2e36a3f2a410
ga4gh-cli workflow prepare examples/sequence-qc/profile.json \
  examples/sequence-qc/request.json public-sequence-qc@1 \
  --profile-revision "$pin" --vocabulary-revision "$pin"
```

The output contains `fingerprint`, not `run_id`. To submit, configure the `WES`
section for the intended CLI profile, then use `workflow submit` with the same
inputs. `--profile NAME` selects that configuration; the host handles WES
credentials. For an uploaded primary descriptor, `--descriptor-digest sha256:…` checks the exact
attachment bytes before submission. If WES might have accepted a request but
no run ID was received, check the service before retrying.

For result formats, worked examples and limits of the evidence, see the
[workflow contracts guide on Read the Docs](https://ga4gh-sdk.readthedocs.io/en/latest/workflow-contracts/).
