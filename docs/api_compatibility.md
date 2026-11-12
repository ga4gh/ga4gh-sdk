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
| Tool, version, lists, and tool pages | 2.0.1 decoding; unknown fields stay in `extra`. | 2.0.2 validation, including nested version descriptions; same public result types. |
| Tag-filtered tools/pages | Rejected unless the caller enables a known backport. | Allowed unless the caller disables support. |
| Tool classes, descriptors, file lists, test files | Shared request and model paths. | Same paths and models; these contracts did not change. |
| ZIP download | Available with bounded bytes and a signature check. | Same behavior. |

Local HTTP tests exercise common operations under both profiles and the version
boundaries above. Public registry smoke tests cover selected read operations;
they do not establish tag filtering or every response shape on those servers.

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
`RequireKnown`. Reading a response never changes the selection or retries with
an older decoder after a failure.

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
let client = TRS::with_options(config, TrsOptions {
    version: VersionPolicy::Explicit(TrsVersion::V2_0_1),
    // Enable only after establishing support on this specific deployment.
    tag_filter: CapabilityOverride::Enabled,
}).await?;
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

Pass `&TRS` to typed pagination helpers to keep the same profile and capability
policy on every page. Their older `&Transport` form remains available; it uses
the unverified 2.0.1 baseline and disallows tags. Both forms reject pagination
URLs outside the configured tools endpoint and retain opaque query tokens.

The [CLI](../cli/README.md#trs-examples) exposes the same options. Python has no
TRS client wrapper yet; this change does not add one or alter other bindings.

## Models and failures

The public `trs::models` structs retain their 2.0.1 shape. The 2.0.2 decoder
validates its wire model, then projects the original JSON into those public
structs. New fields remain in `extra`; `ToolVersion::description()` is a string
accessor over the preserved `description` value. Unknown fields and string enum
variants follow the existing extension-preservation contract.

For example, `description: {"provider": 42}` is extension data under 2.0.1,
but an error under 2.0.2, where the property is a string. As with other optional
fields, the SDK also accepts `null`; the original description value stays in
`extra` and in serialized JSON. This holds for single
versions, version lists, nested versions inside tools, and paginated tools.
Direct `serde_json::from_value::<ToolVersion>` uses the public model only;
version-specific validation is a client operation.

TRS JSON reads decode the bounded original response bytes. Invalid UTF-8,
malformed JSON, missing required fields, and incompatible field types return
errors. Non-success HTTP responses retain an inspectable `HttpError`; they are
not disguised as model errors. No fallback decoder, fabricated required field,
or automatic replay turns an invalid response into success. The existing
Dockstore exception for non-string service-info timestamps stays limited to
that discovery path.

Generated deserialization checks required fields and Rust-represented types.
It is not a complete OpenAPI validator: format constraints and every semantic
relationship are not checked. Existing optional fields accept JSON `null`;
typed serialization is not byte-for-byte preservation of a response. Text
and raw ZIP helpers retain their existing decoding/streaming contracts.

## Rust source migration

- `TRS` connection fields are private so callers cannot change the transport
  while retaining another server's discovery result and profile. Use
  `configuration()`, `transport()`, and `discovered_service_info()` instead of
  direct field reads. Use constructors instead of a `TRS { ... }` literal; use
  `without_discovery` for an explicitly configured connection.
- Existing `ToolVersion` struct literals remain valid. Use `description()` or
  `extra["description"]` for the newly standardized property.
- `ToolFilter` gains `tags`. Existing full struct literals must add this field
  or use `..Default::default()`. Other filter values and method outputs retain
  their existing shapes.

## Implementation and extension

The design separates three responsibilities:

- `lib/src/compatibility.rs` owns generic selection policy, provenance,
  capability overrides, and inspectable selection errors. It performs no I/O
  and has no GA4GH-specific version ordering assumptions.
- `clients/trs/compatibility.rs` defines supported TRS profiles, the fallback,
  and their capabilities. `TRS` owns this state together with its transport and
  discovery snapshot. CLI flags translate to these SDK options.
- `clients/trs/protocol/` contains hand-written response adapters. The `TRS`
  client and typed pagination helpers call the selected adapter and return
  public models. The adapters do not own transport or discovery policy.

### Public models and wire models

TRS keeps the following layout:

```text
lib/src/clients/trs/
├── models/                 Generated 2.0.1 models; public SDK result types
├── wire/
│   └── v2_0_2/models/      Generated 2.0.2 models; private response checks
└── protocol/
    ├── mod.rs              Dispatch by the selected TRS profile
    ├── v2_0_1.rs           Decode directly into public models
    └── v2_0_2.rs           Check 2.0.2 models, then produce public models
```

**Decision:** retain the existing `trs::models` types as the public interface
for both profiles. The layout is deliberately asymmetric: the 2.0.1 models
also represent that version's wire contract. A separate copy under
`wire/v2_0_1/` would duplicate those types without changing current behavior.
The private 2.0.2 models let the client check the newer response contract
without changing the fields of public Rust structs. Replacing the public
models with 2.0.2 models would break downstream full struct literals. Returning
different public types for each profile would pass version branching to callers.

For the affected tool and version responses, the 2.0.2 adapter first
deserializes into its private model to check the represented field types and
required fields. It then converts the **original JSON value** into the public
model. This preserves extension data without a serialization round trip
through the private model. New `description` values stay in `extra`, with a
hand-written `description()` accessor. The checks and their limits are defined
under [Models and failures](#models-and-failures).

The tradeoff is a second generated model set and an extra deserialization pass
for these 2.0.2 responses. Both sets use the same generation wrapper, template,
and TRS patch; `./utils/build_models_wrapper.sh trs` regenerates both. Keep
hand-written adapters and accessors outside generated directories. See the
[model generation guide](autogenerate_OAI_models.md#pinned-specs) for sources
and commands.

This is the current TRS decision, not a requirement to keep every API's public
models fixed at its oldest version. Other clients can reuse the shared policy
types while choosing their own models and adapters. If a future contract cannot
be represented faithfully by existing public types, make an explicit public API
and migration decision rather than silently discarding or changing its meaning.

### Adding a profile

1. Compare the new contract's requests, responses, paths, and capabilities with
   retained profiles. Reuse unchanged behavior only where that comparison
   establishes compatibility; a newer version number is not evidence.
2. Add any required model set to the pinned generation inputs with its own
   destination and module path. Preserve the inputs for retained versions and
   verify that regeneration reproduces all affected sets.
3. Add the profile and capability rules in the service's compatibility module.
   Add adapters for actual contract differences. Keep common HTTP behavior in
   the client and transport; do not copy entire clients or create version
   directories for identical logic. Never select a fallback decoder because
   a response failed to decode.
4. Test the new and retained profiles through the public client, including
   changed fields, extension preservation, malformed responses, capabilities,
   and pagination where applicable. Update the support table, user-facing
   options, and migration notes when those contracts change.

Regression tests live in `lib/tests/trs_compatibility.rs` and
`cli/tests/trs_compatibility.rs`. They exercise actual HTTP responses and CLI
subprocesses, including decoder differences, extensions, tag guards,
pagination, discovery failures, malformed data, and bounded HTTP errors.
Live registry tests in `lib/tests/trs_public_registries.rs` provide separate
interoperability evidence; they do not prove conformance of every deployment.
