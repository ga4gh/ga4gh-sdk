# Pinned I/O Profile schemas

These Apache-2.0 schemas are byte-for-byte copies from
[`uniqueg/ga4gh-io-schema` at `45d4132`](https://github.com/uniqueg/ga4gh-io-schema/tree/45d41324353cb4900fdd999c307c2e36a3f2a410/schemas).
Copyright 2026 ELIXIR On-Cloud / GA4GH Contributors.

| Local file | Upstream file | SHA-256 |
| --- | --- | --- |
| `ga4gh-io-profile.schema.json` | `schemas/ga4gh-io-profile.schema.json` | `9ba7bad2a4a5388110331a00641ca410264c71c369b8a8cb9f3dd2b8b3a42ba0` |
| `ui-widgets.schema.json` | `schemas/vocabularies/ui-widgets.schema.json` | `c5ede316274c17f9467043f1d9f3983612f5481746340853a60c35e642329b96` |

`Profile::from_slice` embeds these files and resolves their `$id` values from a
local registry. The `main` URLs in the schemas are registry keys, not runtime
network fetches. `PROFILE_REVISION` in `lib/src/workflow/mod.rs` pins the accepted
profile and vocabulary revision. The matching upstream test fixtures are in
[`tests/fixtures/io-profile/`](../../../../tests/fixtures/io-profile/README.md).

For a revision update, copy both files from the new upstream commit without
rewriting their bytes, update `PROFILE_REVISION` and the fixture provenance, then
run the workflow contract tests. Compare each copied file with its upstream
counterpart before committing. The exact-file pre-commit exceptions preserve
upstream trailing newlines where needed.
