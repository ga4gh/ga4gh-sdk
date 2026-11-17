# Client state ownership

This page records the decision for
[#183](https://github.com/ga4gh/ga4gh-sdk/issues/183): which representation of
duplicated client state is authoritative, how callers may change it, and how
the SDK moves from public fields to controlled access. It is a design record
for maintainers and for authors of downstream Rust crates and Python
extension crates. It does not describe completed changes. The
[follow-ups](#follow-ups-and-deferrals) below track the implementation under
[#176](https://github.com/ga4gh/ga4gh-sdk/issues/176).

The analysis is a static inspection of commit
[`a5d0175`](https://github.com/ga4gh/ga4gh-sdk/tree/a5d017513fd9091619a5feb338008c6f8fb02df4),
with two reproductions (see [Divergence cases](#divergence-cases)). It
follows finding 7 of the
[2026-11-10 assessment](assessments/2026-11-10-engineering.md). The rules
apply the [ownership principles](engineering_principles.md#give-state-and-policy-a-clear-owner).

## Scope

In scope:

- Client configuration, transport configuration, and discovered service
  metadata of the TES, WES, DRS, and TRS clients, and of `Discovered<T>`.
- The API root of DRS resolution references compared with the root of DRS
  requests.
- Typed and raw views in the Python bindings (`inner` and `raw` fields) and
  the duplicated state of `AsyncTES`.

Other issues own related work:

- [#100](https://github.com/ga4gh/ga4gh-sdk/issues/100): typed errors, and a
  `service` field that holds a `Service` instead of a `Result`. Constructors
  then return the error.
- [#97](https://github.com/ga4gh/ga4gh-sdk/issues/97): `with_artifact`
  constructors and `*_as<T>` methods. The service type check moves from the
  methods to the constructors.
- [#173](https://github.com/ga4gh/ga4gh-sdk/issues/173): transport and
  operation policy, including time budgets, through discovery and submission.
- [#78](https://github.com/ga4gh/ga4gh-sdk/issues/78): downstream
  extensibility in general.

The rules below do not depend on the exact code of #100 or #97. They use
"the discovered service" for the value that the `service` field holds after
either change.

## Current state

### Client configuration, transport, and discovery

| Type | Fields | Set by | Read by | Public mutation |
| --- | --- | --- | --- | --- |
| `TES`, `WES`, `DRS` | `config: Configuration`, `service: Result<Service, _>`, `transport: Transport` | `new`, `with_transport`, `discover*`; `config` is a clone of `transport.config` | Requests: `transport`. TES and WES submissions check `service` first. DRS references and the Python `config` getters: `config` | All three fields are `pub` |
| `Transport` | `config: Configuration` (public); HTTP client and authentication mode (private) | `new`, `with_client`, `with_client_and_configured_auth`, `with_timeouts` | API root, credentials, user agent, and the `discovery` and `submission` limits: `config` at request time. Connect and request timeouts: the HTTP client, fixed at construction | `config` is `pub` |
| `Discovered<T>` | `client: T`, `capabilities: CapabilitySnapshot` | `TES::discover*`, `WES::discover*`; `refresh` replaces `client.service` and `capabilities` together | Preflight validation: `capabilities`. Submission: `client` | Both fields are `pub`; `capabilities.raw`, `fetched_at`, and `max_age` are `pub` |
| `TRS` | `transport`, `service: Option<Service>`, `compatibility` | `new`, `with_options`, `with_transport*`, `without_discovery` | Accessors `configuration()`, `transport()`, `discovered_service_info()`, `compatibility()` | None; `with_tag_filter_override` consumes the client |
| `ServiceRegistry`, `ServiceInfo` | `transport` | `new` | Requests | None |
| `tes::Task`, `wes::Run` | `id`, `transport` | `create`, `run_workflow*`, `new` | Requests for that task or run | Both fields are `pub` |

`TRS` already follows the rules chosen below. Its documentation states that a
new client is needed to change the API root.

Sources:
[TES](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/clients/tes/mod.rs#L174-L211),
[WES](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/clients/wes/mod.rs#L143-L176),
[DRS](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/clients/drs/mod.rs#L47-L88),
[TRS](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/clients/trs/mod.rs#L219-L302),
[`Transport`](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/utils/transport.rs#L315-L408),
[`Discovered`](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/clients/capabilities.rs#L225-L353).

### DRS references

`DRS::resolution_reference` builds the API root of a `ResolutionReference`
from `self.config.base_path`. `DRS::resolve_reference_raw` compares a
reference with that root and then sends the request through `self.transport`,
which resolves the endpoint below `self.transport.config.base_path`. A
`PendingResolution` contains the reference, so a caller can store it and
resume later with another client. See
[the resolution code](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/lib/src/clients/drs/resolution.rs#L107-L156).

### Python typed and raw views

Python code cannot change these values: the classes have no setters, and the
`config` and `transport` getters return copies. Rust extension crates that
depend on the `ga4gh` crate can change the public Rust fields.

| Class | Fields | Accessors that read `inner` | Accessors and operations that read `raw` |
| --- | --- | --- | --- |
| `PyTesTask` | `inner: TesTask`, `raw: Value` | `id`, `state`, `status`, `name`, `description`, `creation_time`, `__repr__` | `inputs`, `outputs`, `resources`, `executors`, `volumes`, `tags`, `logs`, `json()`, `to_dict()`, pickling, and the body that `TES.create()` and `AsyncTES.create()` send |
| `PyRunRequest` | `inner: RunRequest`, `raw: Value` | Scalar fields, the completeness check, and the form that `WES.run()` sends | `workflow_params`, `tags`, `workflow_engine_parameters`, `json()`, `to_dict()`, pickling |
| `PyRunLog`, `PyTaskLog`, `PyRunSummary` | `inner`, `raw` | Scalar fields | Nested fields, `json()`, `to_dict()` |
| `PyTesListTasksResponse`, `PyRunListResponse`, `PyTaskListResponse` | `inner`, a vector of item views, `raw` | Page token | Items: the vector; `json()`, `to_dict()`: `raw` |
| `PyDrsObject`, `PyAccessURL`, `PyBulkAccessURL` | `inner`, `raw` | Scalar fields | Nested fields, `json()`, `to_dict()` |
| `PyTool`, `PyToolVersion`, `PyFileWrapper`, `PyToolFile` | `inner`, `raw` | Scalar fields | Nested fields, `json()`, `to_dict()` |
| `PyAsyncTES` | `config`, `transport`, `inner: LazyTES` | — | The `config` and `transport` getters read the fields; the first call connects a new `TES` from the `config` that it receives |
| `PyTES`, `PyWES`, `PyDRS`, `PyTRS`, `PyTask`, `PyRun`, `PyTransport`, `PyConfiguration`, and others | `inner` only | — | — |

The constructors (`from_json`, `from_value`, `parse`) derive `inner` from
`raw`, and the field documentation of `PyTesTask` and `PyRunRequest` asks
callers to build both with them. Nothing enforces that after construction.
Sources:
[`PyTesTask`](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/python/src/clients/tes.rs#L515-L529),
[TES submission body](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/python/src/clients/tes.rs#L144-L150),
[WES submission](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/python/src/clients/wes/client.rs#L104-L118),
[`PyAsyncTES`](https://github.com/ga4gh/ga4gh-sdk/blob/a5d017513fd9091619a5feb338008c6f8fb02df4/python/src/clients/tes_async.rs#L74-L96).

### Affected public surfaces

A change to the rules below affects these surfaces:

- Rust: the public fields of `TES`, `WES`, `DRS`, `Discovered<T>`,
  `Transport` (`config`), and the struct-literal construction of these types.
  The SDK's own tests and the
  [downstream example](downstream.md) use both. The downstream test assigns
  `tes.transport` after `TES::new`.
- Python extension crates: the public `inner` and `raw` fields of the view
  classes above, and the `config`, `transport`, and `inner` fields of
  `PyAsyncTES`. The [Python bindings guide](../python/README.md#rust-crates)
  documents `inner` as the extension API.
- Python users: no API change. The values of the `config` and `transport`
  getters, and of the view accessors, can change if a fix selects another
  authoritative source.
- CLI: no direct effect. The CLI builds clients per command and does not
  change their fields.

## Divergence cases

The cases below use only public fields and methods. Cases 1 and 6 were run
against `a5d0175`; the others follow from the code paths cited above.

1. **DRS reference root and request root.** A reference names one service,
   and the request goes to another:

   ```rust
   // A client from `DRS::with_transport` behaves the same; the
   // reproduction used a struct literal to skip discovery.
   let transport = Transport::new(&Configuration::new(request_root));
   let mut drs = DRS {
       config: transport.config.clone(),
       service: Ok(Service::default()),
       transport,
   };
   drs.config.base_path = Url::parse("https://other.example/ga4gh/drs/v1/")?;

   // Sent to request_root. A 202 response gives a PendingResolution
   // whose reference names https://other.example/ga4gh/drs/v1/.
   let pending = drs.get_object_raw("id").await.unwrap_err();
   ```

   Output of the reproduction: the reference's `api_root` was
   `https://other.example/ga4gh/drs/v1/`, and the mock server at the request
   root received the request. A caller that resumes this reference with a
   client for `other.example` asks a different service.

2. **Client configuration and transport configuration.** After
   `tes.transport = Transport::with_client(&other_config, client)`, `tes.config`
   still describes the original configuration. The downstream test uses this
   pattern with the same configuration. The Python `TES.config`,
   `WES.config`, and `DRS.config` getters return that stale copy, while requests use the new transport.

3. **Transport configuration and HTTP client.** `Transport::new` copies
   `config.timeouts.connect` and `request` into the HTTP client. A later
   assignment to `transport.config.timeouts.request` changes the value that
   callers read, but not the limit that applies. The `discovery` and
   `submission` limits are read at request time, so they do change. A change
   to `transport.config.base_path` redirects all later requests of a client
   whose discovered service describes the old root.

4. **Discovered client and capability snapshot.** An assignment to
   `discovered.client` keeps `discovered.capabilities` from the previous
   service. `create_validated` then validates against one service and submits
   to another. `refresh` replaces both values together and is not affected.

5. **Stored discovery failure.** A client built as a struct literal with
   `service: Err(_)` fails every submission at the service check. The
   constructors never store `Err`. This applies to TES and WES, whose
   submissions check the stored service. #100 removes this state.

6. **Python typed and raw views.** A Rust extension changes `inner`, and the
   accessors disagree with the submitted body:

   ```rust
   let mut task = PyTesTask::from_value(json!({
       "name": "original",
       "executors": [{"image": "alpine", "command": ["true"]}]
   }))?;
   task.inner.name = Some("changed".into());
   task.inner.id = Some("task-1".into());
   // task.id() == Some("task-1"); task.json() still has "name": "original"
   // and no "id". TES.create(task) sends the "original" JSON.
   ```

   For `PyRunRequest`, the direction is reversed: `WES.run()` sends `inner`,
   and `json()` returns `raw`.

7. **`AsyncTES` state.** `PyAsyncTES` builds `transport` from `config` at
   construction. The first call builds a third copy: `TES::new(config)`
   creates its own transport. A Rust extension that changes `config` before
   the first call changes the connected client but not `transport`. A change
   after the first call has no effect on requests.

### Snapshots and invariants

| State | Kind | What callers may rely on |
| --- | --- | --- |
| `Configuration` passed to a constructor | Snapshot: copied at construction | Later changes to the caller's value do not change the client. |
| Discovered service of `TES`, `WES`, `DRS`, `TRS` | Snapshot of one discovery request | It describes the service at the client's API root at construction time. It is not refreshed. |
| `CapabilitySnapshot` | Snapshot with `fetched_at` and `max_age` | It comes from the same response as the discovered service of `Discovered::client`. `is_fresh` reports its age. |
| `Task` and `Run` handles | Snapshot of the creating client's transport | Requests for the task or run use that transport. |
| `ResolutionReference` | Credential-free snapshot of the API root and endpoint | Invariant: its `api_root` is the root that received the request. |
| Client identity: API root, credentials, user agent, time limits | Invariant | Every request, discovery request, reference, and `configuration()` value uses the same configuration. |
| Python view: `inner`, accessors, `json()`, submitted body | Invariant | All of them describe the same JSON value. |

## Decision

### Client state

1. **The transport owns the client's identity.** The configuration of a
   client's `Transport` determines its API root, credentials, user agent, and
   time limits. Every other read of the configuration, including DRS
   references and the Python `config` getters, uses
   `transport.config`. The separate `config` field becomes a deprecated copy,
   then goes away.
2. **A client is immutable after construction.** To change the API root,
   credentials, HTTP client, or time limits, build a new client, for example
   with `with_transport`. The SDK adds no setters for these values: a new
   identity needs a new discovery request and service type check.
3. **Discovery results are snapshots.** The discovered service and a
   `CapabilitySnapshot` come from one response and stay together. Only a
   constructor or `Discovered::refresh` replaces them, and `refresh` replaces
   both in one step.
4. **Read access goes through accessors.** `TES`, `WES`, and `DRS` get the
   accessors that `TRS` has: `configuration()`, `transport()`, and
   `discovered_service_info()`. `TRS` returns `Option<&Service>` because
   `without_discovery` exists; the other clients return the type that the
   `service` field has after #100. `Discovered<T>` gets `client()`,
   `capabilities()`, and `into_parts()`, and keeps `Deref`. `Transport` gets
   `configuration()`.
5. **The transport's configuration is fixed after construction.** The values
   that the HTTP client copied must not change without the client. Callers
   that need other limits build a transport with `with_timeouts` or
   `with_client`.
6. **Passive records and handles keep public fields.** `Configuration`,
   `Timeouts`, `Task`, `Run`, `ResolutionReference`, and `PendingResolution`
   have no relationship between fields that the SDK must maintain.
   `Configuration` is copied at construction, and a handle's transport is a
   deliberate snapshot.

### Python views

1. **`raw` is authoritative.** For each view class, `raw` holds the JSON as
   given or received. `inner` is a projection derived from `raw`. All
   accessors, `json()`, `to_dict()`, pickling, and submission describe that
   value. Either representation can be sent if tests prove that they agree,
   including unknown fields and explicit `null`.
2. **Views are immutable.** Extension crates read `inner()` and `raw()`
   accessors and build a changed view from JSON with the existing
   constructors (`from_value`, `from_json`). No setter changes one field.
3. **`AsyncTES` has one transport.** The lazy client connects with
   `TES::with_transport(self.transport.clone())`, so `config()`,
   `transport()`, and the connected client share one configuration.
4. **Client wrappers keep `inner`.** `PyTES`, `PyWES`, `PyDRS`, `PyTRS`, and
   the other single-field wrappers hold one value. Their `inner` field stays
   public; the library type protects its own invariants.

### Compatibility path

The SDK is pre-1.0, and its [versioning](../README.md#versioning) allows
breaking changes. The path below still avoids a breaking change without
notice:

1. **Additive release.** Add the accessors, `Transport::configuration()`, and
   `Discovered` accessors. Fix the DRS reference root and the `AsyncTES`
   transport. Read the authoritative source internally. Add a constructor
   for a client without discovery where tests or offline use need one, as
   `TRS::without_discovery` does.
2. **Deprecation release.** Mark the public fields `#[deprecated]` with a
   note that names the accessor or constructor to use. The SDK's own code
   uses the accessors. Remaining internal uses, such as the constructors,
   need a scoped `#[allow(deprecated)]`, because CI denies warnings. Update the downstream example,
   the [downstream guide](downstream.md), and the
   [Python bindings guide](../python/README.md#rust-crates).
3. **Breaking release.** Make the fields private. Release notes list each
   field with its replacement.

Methods and fields can have the same name in Rust, so `tes.transport`
becomes `tes.transport()` and `task.inner` becomes `task.inner()`. The
`config` field maps to `configuration()`, as on `TRS`.

### Alternatives considered

| Option | Result |
| --- | --- |
| Accessors | Chosen. They allow reads without allowing independent changes. |
| Controlled mutation (setters that rediscover) | Rejected for identity: a setter would need network access, error handling, and a partial-failure state. No caller needs it. |
| Rebuild with a constructor | Chosen for every identity change. It reuses existing constructors. |
| Immutable snapshots | Chosen for discovery results, handles, and Python views. |
| Make the fields private at once | Rejected: it breaks the downstream example, the SDK's tests, and extension crates without a deprecation period. |
| A shared client trait or base type | Rejected: no caller needs it, and #97 already adds the generic constructors and methods that downstream crates asked for. |

### Tradeoffs

- **Less flexibility.** A caller can no longer swap a transport into a
  client that already exists. The replacement is one constructor call, which
  sends one more service-info request.
- **Migration cost.** Downstream code that assigns or reads fields must
  change once. The deprecation release gives compiler warnings first.
- **Unchanged snapshots.** A client does not notice that its service changed
  after discovery. `Discovered::refresh` covers callers that need fresh
  capabilities; other clients are rebuilt.
- **Python views cost one parse.** A changed view is parsed again from
  JSON. The views are small, and this preserves unknown fields.

## Downstream usage

The downstream test currently assigns a transport after construction. Under
these rules, it builds the client with the transport instead:

```rust
use ga4gh_sdk::clients::tes::TES;
use ga4gh_sdk::utils::transport::Transport;

let transport = Transport::with_client(&config, client);
let tes = TES::with_transport(transport).await?;

// Share the client's transport with a client for an added endpoint.
let usage = UsageClient::from_transport(tes.transport().clone());
let root = &tes.configuration().base_path;
```

A Python extension crate builds a changed task from JSON:

```rust
let mut raw = task.raw().clone();
raw["name"] = json!("changed");
let task = PyTesTask::from_value(raw)?;
```

## Regression evidence

Each accepted invariant needs the evidence below. The test names are
proposals for the follow-ups.

| Invariant | Evidence |
| --- | --- |
| A DRS reference names the root that received the request | `lib/tests/drs_resolution.rs`: `reference_root_is_transport_root`. Build a DRS whose transport root differs from a stale client configuration, receive a 202 response from the mock server at the transport root, and compare the reference's `api_root` with that root. Keep `immediate_200_and_resume_validation`, which rejects a reference for another root. |
| `configuration()` is the transport's configuration | Unit tests in `tes`, `wes`, and `drs`: `configuration_is_transport_configuration`, with a transport from `Transport::with_timeouts` and a non-default API root. |
| Discovery results stay together | `lib/tests/capabilities.rs`: `refresh_replaces_service_and_snapshot_together`; the existing `discovery_caches_raw_extensions_and_refreshes_once` stays. |
| Fields cannot change after construction | `compile_fail` documentation tests on `TES`, `DRS`, `Discovered`, and `Transport` that assign a field, after the breaking release. |
| A Python view describes one JSON value | Rust tests in the `ga4gh` crate for each view class: build from JSON with an unknown field and an explicit `null`, then compare each accessor, `json()`, and the submitted body with `raw`. Python tests: `test_tes.py::test_task_accessors_match_json`, `test_wes.py::test_run_sends_request_json`. |
| `AsyncTES` uses one transport | `tes_async.rs` unit test `lazy_client_shares_transport`: a transport with a custom user agent reaches the mock server for discovery and for `create`. |
| Downstream code builds with the accessors | `make downstream` (CI job `downstream`) after the example is updated. |

## Follow-ups and deferrals

The client-state and Python-view changes can ship independently. The
Python follow-ups that use client accessors depend on the first client
follow-up.

| Follow-up | Area | Depends on |
| --- | --- | --- |
| Build DRS resolution references from the transport root | Client state; correctness fix | — |
| Add accessors to the TES, WES, DRS, `Discovered`, and `Transport` types | Client state; additive | #100 for the discovered service type |
| Deprecate and then make private the client, `Discovered`, and `Transport` fields | Client state; breaking | The accessors; #100; #97 |
| Make Python typed and raw views read-only for extension crates | Python views; additive, then breaking | — |
| Connect `AsyncTES` with its own transport | Python views; correctness fix | — |

Explicit deferrals and decisions without a change:

- `Configuration`, `Timeouts`, `Task`, `Run`, `ResolutionReference`, and
  `PendingResolution` keep public fields (see rule 6).
- A refresh method on `TES`, `WES`, or `DRS` outside `Discovered`: no caller
  needs one. Rebuild the client.
- Transport injection for `ServiceInfo`, and time budgets during discovery
  and submission: #173.
- Typed errors and removal of the stored discovery error: #100.
- Generic constructors and methods for other service types: #97.
- Credential rotation through a token provider: the token provider is
  shared, dynamic state by design; the
  [authentication guide](authentication.md) describes it.
- Setters on Python classes: not planned. Python code builds a new object.
- The experimental `workflow` module was not part of this analysis.
