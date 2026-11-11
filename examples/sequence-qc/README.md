# Synthetic sequence QC example

This directory is the public, synthetic example for the optional SDK
`workflow-contracts` feature. Start from the repository root. The
[workflow contracts guide](../../docs/workflow-contracts.md) explains the API,
CLI commands, and limits of the evidence.

| File | Role |
| --- | --- |
| `profile.json` | Declares the accepted sequence input and named metrics/report outputs. |
| `request.json` | Supplies three valid synthetic sequences and the CWL descriptor name. |
| `invalid-request.json` | Demonstrates a local validation failure before submission. |
| `main.cwl` | Executes sequence QC with a Python image pinned by digest. |
| `policy.json` | Compares selected metrics and required identities from two results. |
| `view_report.py` | Feeds reports from the local test evidence to IPython's HTML viewer. |

The profile keeps its existing `urn:ga4gh:qc-demo:1` `$id`; moving the files
does not change the profile bytes or the recorded schema digest.

Preparation is offline and needs no WES service:

```sh
pin=45d41324353cb4900fdd999c307c2e36a3f2a410
cargo run -p ga4gh-cli -- workflow prepare \
  examples/sequence-qc/profile.json examples/sequence-qc/request.json public-sequence-qc@1 \
  --profile-revision "$pin" --vocabulary-revision "$pin"
```

For the optional two-server execution, run
`python3 tests/run-workflow-contracts-qc.py` from the repository root. It needs
Python 3.12+, Docker, and a Docker context that can mount this checkout. The
[test guide](../../tests/README.md#workflow-contracts-and-two-local-qc-executions)
describes its setup and output. The test starts two **local** Toil WES processes;
it does not test federation between external deployments. The script installs
directly pinned Toil and IPython versions, but their transitive Python packages
are not locked. The CWL execution image itself is pinned by digest.
