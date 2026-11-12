# API version compatibility

The SDK can connect to deployments that expose different GA4GH API versions.
Version support is explicit and local to a client connection. It is not HTTP
content negotiation, server conformance certification, or a promise to accept
all older or newer versions. [#217](https://github.com/ga4gh/ga4gh-sdk/issues/217)
tracks adoption across APIs; TRS is the first implementation.

## Supported profiles

| Client | Selected response contracts | Current policy |
| --- | --- | --- |
| TRS | [2.0.1](https://github.com/ga4gh/tool-registry-service-schemas/tree/2.0.1), [2.0.2](https://github.com/ga4gh/tool-registry-service-schemas/tree/2.0.2) | Per-client selection; tested with local HTTP fixtures for both contracts. |
| TES, WES, DRS, Service Info, Service Registry | See [pinned model schemas](autogenerate_OAI_models.md#pinned-specs). | Existing behavior; these clients do not yet use shared version selection. A pinned schema is not a tested compatibility range. |

Both TRS contracts use the same `/ga4gh/trs/v2` API root and operations. The
SDK keeps that root under caller control. TRS 2.0.2 adds an optional version
`description` and a `tags` filter; ZIP responses already exist in 2.0.1.
The 2.0.2 schema tag contains an inconsistent `info.version: 2.1.0`. Selection
uses the tag's actual contract name, `2.0.2`; it does not alias a deployment
that advertises `2.1.0` to this contract.

The operation contract is:

| Operations | 2.0.1 profile | 2.0.2 profile |
| --- | --- | --- |
| Tool, version, lists, and tool pages | Shared public models; unknown fields stay in `extra`. | Same models and decoding; the optional version description is available as a string through `description()` when valid. |
| Tag-filtered tools/pages | Rejected unless the caller enables a known backport. | Allowed unless the caller disables support. |
| Tool classes, descriptors, file lists, test files | Shared request and model paths. | Same paths and models; these contracts did not change. |
| ZIP download | Available with bounded bytes and a signature check. | Same behavior. |

Local HTTP tests exercise common operations under both profiles, version
boundaries, and nonconforming optional descriptions. Public registry smoke tests
cover selected read operations; they do not establish tag filtering or every
response shape on those servers.

## Selection and capabilities

`TRS::new` uses `TrsOptions::default()` and checks service identity through
`service-info`. It reads the **API** version in `type.version`, not the server
software version in the top-level `version` field.

| Version policy | Known declaration | Unknown declaration |
| --- | --- | --- |
| `VersionPolicy::Auto` | Select that profile. | Use the 2.0.1 read contract; record `SelectionSource::Baseline` and unknown tag-filter support. |
| `VersionPolicy::RequireKnown` | Select that profile. | Return an inspectable `CompatibilityError::UnknownVersion`. |
| `VersionPolicy::Explicit(profile)` | Use the caller's profile; retain the declaration. | Same; record `SelectionSource::Caller`. |

Default fallback retains interoperability with registries that advertise an
unrecognized version. It is unverified interpretation, not inferred semantic
version compatibility. Applications that require a recognized declaration should use
`RequireKnown`. Reading a response never changes the selection or retries an
operation after a decoding failure.

Missing required service-info fields, malformed JSON, authentication failures,
and timeouts remain errors, including with `Explicit`. For a known deployment
without usable discovery, construct `TRS::without_discovery(transport, profile)`.
It requires a profile and does not verify the service identity. `service_info()`
can fetch a fresh document but does not change the selected profile; construct
a new client when the endpoint or version policy changes.

Capabilities are separate from versions. By default, `tags` is `Specified` for
2.0.2, `NotSpecified` for 2.0.1, and `Unknown` for a fallback selection. A request
with tags is rejected before the tools request unless support is specified or
explicitly enabled. A caller can override it with `CapabilityOverride::Enabled`
or `Disabled`, including for provider backports or incomplete implementations.
`Specified` describes the selected contract, not a probe of the server. HTTP 200
alone does not prove that a server applied a requested filter.

```rust,no_run
use ga4gh_sdk::clients::trs::{responses, ToolFilter, TrsOptions, TrsVersion, TRS};
use ga4gh_sdk::compatibility::{CapabilityOverride, VersionPolicy};
use ga4gh_sdk::utils::{configuration::Configuration, transport::BodyLimits};

# async fn example(config: &Configuration) -> Result<(), Box<dyn std::error::Error>> {
let options = TrsOptions::default()
    .with_version(VersionPolicy::Explicit(TrsVersion::V2_0_1))
    // Enable only after establishing support on this specific deployment.
    .with_tag_filter(CapabilityOverride::Enabled);
let client = TRS::with_options(config, options).await?;
let selection = client.compatibility().selection();
assert_eq!(selection.profile(), TrsVersion::V2_0_1);
let filter = ToolFilter { tags: vec!["genomics".into()], ..Default::default() };
let page = responses::list_tools_page(&client, filter.query()?, BodyLimits::default()).await?;
if let Some(next) = page.next_page()? {
    let next_page = responses::next_tools_page(&client, next, BodyLimits::default()).await?;
    println!("{} tools", next_page.tools.len());
}
# Ok(())
# }
```

Pass `&TRS` to typed pagination helpers to keep the same capability policy on
every page. Their older `&Transport` form remains available; it uses the shared
models without an established tag-filter capability. Both forms reject pagination
URLs outside the configured tools endpoint and retain opaque query tokens.

The [CLI](../cli/README.md#trs-examples) exposes the same options. Python has no
TRS client wrapper yet; this change does not add one or alter other bindings.

## Models and failures

The public `trs::models` path re-exports the generated 2.0.1 models stored under
`trs/wire/v2_0_1/models/`. Both profiles decode into these types. TRS 2.0.2
adds an optional `ToolVersion.description`; it stays in `extra` and is
available through `description()` when it is a string. Unknown fields and
string enum variants retain the existing extension-preservation behavior.

For example, a registry may send `description: {"provider": 42}` even though
2.0.2 specifies a string. The SDK keeps that raw optional value in `extra`
and returns the tool or list; `description()` returns `None`. This applies to
single versions, versions nested in tools, lists, and paginated tools. It does
not drop an item or claim that the server response conforms to the schema.
`null` is also retained as an optional value. `RequireKnown` checks only the
declared API version; it does not make response parsing strict.

TRS JSON reads decode the bounded original response bytes. Invalid UTF-8,
malformed JSON, missing required fields, and incompatible represented field
types return errors. Non-success HTTP responses retain an inspectable
`HttpError`; they are not disguised as model errors. No fabricated required
field or automatic replay turns an invalid response into success. The existing
Dockstore exception for non-string service-info timestamps stays limited to
that discovery path.

Generated deserialization checks required fields and Rust-represented types;
it is not a complete OpenAPI validator. The non-string optional description is
a documented interoperability exception, not a claim of schema conformance.
Format constraints and every semantic relationship are not checked. Typed
serialization preserves unknown JSON values but does not promise byte-for-byte
preservation. Text and raw ZIP helpers retain their existing decoding and
streaming contracts.

## Rust source migration

- `TRS` connection fields are private so callers cannot change the transport
  while retaining another server's discovery result and profile. Use
  `configuration()`, `transport()`, and `discovered_service_info()` instead of
  direct field reads. Use constructors instead of a `TRS { ... }` literal; use
  `without_discovery` for an explicitly configured connection.
- Existing `ToolVersion` struct literals remain valid. Use `description()` or
  `extra["description"]` for the newly standardized property.
- Construct `TrsOptions` with `Default` and `with_version`/`with_tag_filter`;
  it is non-exhaustive so future capability options need not break callers.
- `ToolFilter` gains `tags`. Existing full struct literals must add this field
  or use `..Default::default()`. Other filter values and method outputs retain
  their existing shapes.

## Implementation and extension

The design separates two responsibilities:

- `lib/src/compatibility.rs` owns generic selection policy, provenance,
  capability overrides, and inspectable selection errors. It performs no I/O
  and has no GA4GH-specific version ordering assumptions. Its public types are
  provisional while TRS is the only adopter; [#217](https://github.com/ga4gh/ga4gh-sdk/issues/217)
  tracks adoption by other clients.
- `clients/trs/compatibility.rs` defines supported TRS profiles, the fallback,
  and their capabilities. `TRS` owns this state together with its transport and
  discovery snapshot. CLI flags translate to these SDK options. TRS operations
  and typed pagination helpers decode into the shared public models. The two
  current profiles differ in tag-filter support, not response representation.

### Model layout

TRS keeps the following layout:

```text
lib/src/clients/trs/
├── mod.rs                  Re-exports wire/v2_0_1/models as trs::models
├── compatibility.rs        Version profiles and capability rules
└── wire/v2_0_1/models/     Generated from the pinned 2.0.1 schema
```

The physical location identifies the pinned wire schema; the public import path
remains `trs::models`. The 2.0.2 schema adds only an optional description to
generated response models, so a second 16-file model set and a second decode
pass would add maintenance cost without changing the public result. There is
no version-specific decoder until a contract needs one. This choice is local
to TRS: other APIs can choose a different public representation when their
contracts diverge. If a future profile needs distinct wire types, generate
its own `wire/vX/models/` set and define an explicit conversion to the public
API; do not discard information or silently change a public type's meaning.

`./utils/build_models_wrapper.sh trs` regenerates the 2.0.1 set. Keep future
adapters and current accessors outside generated directories. See the
[model generation guide](autogenerate_OAI_models.md#pinned-specs) for the
pinned source and regeneration procedure.

### Adding a profile

1. Compare the new contract's requests, responses, paths, and capabilities with
   retained profiles. Reuse unchanged behavior only where that comparison
   establishes compatibility; a newer version number is not evidence.
2. Add a pinned versioned model set only if the existing types cannot represent
   and check the new responses. Give it a separate destination and module path;
   verify that regeneration reproduces every retained set.
3. Add the profile and capability rules in the service's compatibility module.
   Add adapters only for actual contract differences that the shared models
   cannot represent. Keep common HTTP behavior in the client and transport;
   do not copy entire clients or create version directories for identical
   logic. Never select a fallback decoder because a response failed to decode.
4. Test the new and retained profiles through the public client, including
   changed fields, extension preservation, malformed responses, capabilities,
   and pagination where applicable. Update the support table, user-facing
   options, and migration notes when those contracts change.

Regression tests live in `lib/tests/trs_compatibility.rs` and
`cli/tests/trs_compatibility.rs`. They exercise actual HTTP responses and CLI
subprocesses, including optional metadata tolerance, extensions, tag guards,
pagination, discovery failures, malformed data, and bounded HTTP errors.
Live registry tests in `lib/tests/trs_public_registries.rs` provide separate
interoperability evidence; they do not prove conformance of every deployment.
