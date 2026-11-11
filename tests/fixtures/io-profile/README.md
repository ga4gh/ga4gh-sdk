# Upstream I/O Profile fixtures

These files are byte-for-byte copies of `tests/data/` in
[`uniqueg/ga4gh-io-schema` at `45d4132`](https://github.com/uniqueg/ga4gh-io-schema/tree/45d41324353cb4900fdd999c307c2e36a3f2a410/tests/data).
Each `.license` file accompanies its corresponding JSON file. Keep upstream
filenames and bytes unchanged. The offline contract tests in
[`lib/tests/workflow_contracts.rs`](../../../lib/tests/workflow_contracts.rs)
use all four JSON files; these are test inputs, not packaged runtime schemas.

| File | SHA-256 |
| --- | --- |
| `example_tool.schema.json` | `0713d70b03ccc43131428132e6309a49bb756e16e1496bfadcbfbf0584147c48` |
| `example_workflow.schema.json` | `c24771cca6d3de3d6e419e5e40a52a334b3449ccefbadd4ebf4225dcaebd12cf` |
| `invalid.schema.json` | `943e9ee63b9b0fb852e4305337ce70dea17875a08c60b3d053a619a7fc00080b` |
| `valid.schema.json` | `d414e27e9f83799301f481874665355c55c895c4b2abad206ffc8c12dcd2af37` |

All four `.license` companions have SHA-256
`cefbd08cfa756992b0397a22d66f6bd20040f36f4f23777682c13cdd3c3028c3`.

To verify provenance, check out the linked upstream commit and compare each
file and `.license` companion against its `tests/data/` counterpart. For a
revision update, copy the upstream files again, record their new hashes here,
and run `cargo test -p ga4gh-lib --features workflow-contracts --test workflow_contracts`.
The vendored runtime schemas and their revision are documented in
[`lib/src/workflow/schemas/README.md`](../../../lib/src/workflow/schemas/README.md).
