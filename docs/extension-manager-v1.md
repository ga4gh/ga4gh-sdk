# Extension Manager v1: contract and implementation plan

Status: experimental v1 implementation on `feat/extension-manager`.
The [extension guide](extensions.md) describes the public API and migration.
Baseline: local `feat/extension-manager` at `bfcfedd`, including the shared
configuration and persistence components merged in
[#350](https://github.com/ga4gh/ga4gh-sdk/pull/350).
This replaces the experimental Rust ABI implementation of
[#64](https://github.com/ga4gh/ga4gh-sdk/issues/64).

## Scope and decisions

The manager installs artifacts, resolves compatible versions, loads native
modules, and provides instances and invocation handles. Domain adapters define
the meaning of capability inputs and results. The existing checksum checks,
shared `JsonStore`, explicit native loading, and `Arc` ownership remain useful.

| Decision | v1 contract | Consequence |
| --- | --- | --- |
| Native interface | Versioned C ABI in a small separate crate, with a C header | Existing experimental Rust ABI plugins need rebuilding |
| Instance concurrency | Serialize each instance; allow different instances to run concurrently | Plugins must support use from different OS threads |
| Process versions | One artifact identity per extension per process | Selecting another artifact requires a restart before loading it |
| Removal | Remove registration; retain immutable artifacts | `remove` does not reclaim disk space; physical cleanup is deferred |
| Resolution | Return an immutable result identifying one artifact and negotiated interfaces | Creation cannot repeat version selection or silently fall back |

Per-instance OS-thread affinity, simultaneous versions of one extension in a
process, and automatic physical deletion are outside v1. These alternatives
require separate lifecycle and concurrency contracts.

## Ownership and threading

`ExtensionManager` owns registry operations and admission to instance creation.
`LoadedModule` owns a library and its validated API descriptor.
`ExtensionInstance` owns one plugin instance, its configuration, and an
`Arc<LoadedModule>`. Invocation handles retain an `Arc<ExtensionInstance>`.
Dropping a manager cannot invalidate an existing instance or handle.

Configuration belongs to the instance. Creating a second instance must not
replace the first instance's configuration or mutable state.

Operations on one instance are serialized, including copying and freeing its
returned buffers. Creation, invocation, and destruction may run on different OS
threads. Operations belonging to different instances, including creation and
resource release, may overlap. Plugin-global state must support this concurrency.
A mutex provides exclusion; it does not provide thread affinity. See the Rust
[`Mutex` contract](https://doc.rust-lang.org/std/sync/struct.Mutex.html).

`destroy_instance` runs exactly once, after all operations and returned-buffer
releases for that instance finish. The module remains alive throughout these
operations and destruction. The last module handle releases the library;
operating-system unloading is not a prerequisite for correctness.

## C ABI v1

The entry point returns a pointer to an immutable, module-owned API table:

```c
typedef struct Ga4ghExtensionApiV1 Ga4ghExtensionApiV1;
const Ga4ghExtensionApiV1 *ga4gh_extension_get_api_v1(void);
```

The table starts with a fixed header containing `uint32_t abi_version` and
`uint32_t struct_size`. The plugin guarantees a valid, correctly aligned pointer
to at least this header. The host reads the header first. It checks the version
and sufficient size before reading any remaining required field. It ignores
unknown trailing fields. Compatible additions append fields; they cannot change
the existing prefix. The table is never returned by value. Rust definitions use
`repr(C)` and the platform C calling convention; see the
[C representation rules](https://doc.rust-lang.org/reference/type-layout.html#the-c-representation).

The table describes extension identity, supported capabilities and interface
versions, and functions for `create_instance`, `invoke`, `destroy_instance`,
and `free_buffer`. Invocation identifies the capability, negotiated interface
version, and operation. The [C header](../extension-abi/include/ga4gh_extension.h) defines the complete
field layout, buffer definitions, and status convention: zero succeeds; every
nonzero status is retained as a plugin failure.

The ownership rules are fixed:

- Only C-compatible scalars, pointer/length byte buffers, and opaque handles
  cross the boundary. Rust collections, references, and `serde_json::Value`
  never cross it.
- Inputs are borrowed until the call returns. The plugin copies input that it
  retains. Configuration is supplied only to `create_instance`.
- The plugin allocates and destroys its instances. A failed creation leaves no
  live instance for the host to destroy.
- The plugin allocates returned success and error buffers. The host copies them
  and calls the table's `free_buffer` exactly once, before releasing the instance
  or module. Creation errors retain the module until their buffers are freed.
- Descriptor data is immutable and valid until the module is released. The host
  never frees plugin memory using the host allocator.
- All entry points return through the C boundary without unwinding. Plugin
  failures use status codes. Panic or foreign-exception containment belongs on
  the plugin side of that boundary.

Native loading remains an explicit trust boundary. Metadata validation cannot
make arbitrary native code safe. Library initializers may execute during loading,
before the exported API can be checked. No plugin code, including initializers,
entry-point queries, or destructors, runs while a registry file lock is held.

## Manifest and resolution

The manifest records these independent values:

| Value | Purpose |
| --- | --- |
| Manifest schema version | Parse and validate installation metadata |
| Extension name and semantic version | Identify the extension and its release |
| Target triple | Match OS, architecture, and relevant platform ABI |
| Content digest (SHA-512 fixed by manifest v1) | Identify the exact library bytes |
| Native ABI version | Select a supported loader contract |
| Capability identifiers and supported interface versions | Negotiate with an adapter |

An extension release version is separate from its capability interface versions.
The v1 interface scheme uses integer revisions and explicit supported
sets; the resolver chooses the highest common revision for each requested
capability. No common revision for a required capability is an error.

The resolver validates the manifest before native loading. It returns a
`ResolvedExtension` containing the exact name, release version, target, digest,
ABI, artifact location, and negotiated capability revisions. These fields are
immutable to callers. The public getters expose identity, library location, and negotiated revisions.

By default, resolution uses the registry's explicitly selected artifact. An
explicit version requirement can select the highest compatible installed release.
Use a maintained semantic-version parser for release requirements. Two matching
artifacts with the same release version and different digests require an exact
selection or digest pin; resolution otherwise returns `AmbiguousArtifact`.
When all matching artifacts are incompatible, classify the closest usable
candidate deterministically: `MissingCapability` takes precedence over
`TargetMismatch`, then `UnsupportedAbi`, then `NoMatchingArtifact`.
Installation order does not change the failure classification.

Before loading, verify the installed bytes against the resolved digest. After
loading, verify the actual exported ABI, identity, and capability interfaces
against that resolution, before creating an instance. The runtime descriptor
does not need to embed the library's own file digest. Any disagreement is an
error. Load, compatibility, initialization, and invocation failures do not
trigger selection of another artifact.

## Admission, disable, and remove

`create_instance(resolved, configuration)` uses precisely the supplied resolution:

1. Acquire the shared registry writer lock and read the current registry.
2. Check that the extension is still registered and enabled, and that the exact
   resolved artifact is still registered. Reject stale or invalid resolutions.
3. Record admission in an immutable operation-local permit for that resolution.
4. Release the registry lock before loading or calling any plugin code.
5. Check the process version constraint, obtain the validated module, and create
   the instance with its configuration.

Admission is fixed at step 3. Creation admitted before `disable` or `remove`
may finish afterward. Later admissions observe the changed registry and fail,
including through managers opened before that change. The permit records a
decision; it does not hold a file lock for the operation's duration.

Changing the selected artifact does not rewrite a retained resolution. If the
old artifact remains registered and enabled, its resolution can still be used,
subject to the process version constraint. If it is no longer registered,
creation fails rather than resolving again. Callers request a new resolution
explicitly when they want the newly selected artifact.

`disable` blocks new creation. `remove` removes the logical extension's
registration and also blocks new creation. Existing instances and their handles
continue working in both cases. A public module handle cannot bypass manager
admission to create new instances.

## One loaded artifact per process

All managers and language wrappers in one host runtime share a process-wide
coordinator, keyed by extension identity. Different registry paths do not allow
different artifacts of the same extension to be loaded side by side.

The pinned artifact identity includes the release version, target, digest, and
ABI. A different digest counts as a different artifact even if the release
version is unchanged. Requests for that other artifact return `RestartRequired`,
with structured current and requested identities.

Pinning survives dropping all instances, handles, and managers. Reloading the
same artifact is allowed. The first dynamic load of an extension remains allowed
after process startup. Installation and registry selection of another artifact
are also allowed while the process is running.

Concurrent loads for one extension must coordinate so only one artifact wins.
The registry lock is released before waiting for this coordination. Loading one
extension must not hold a global coordinator lock across plugin execution.

The failure rule is conservative: once the OS has successfully loaded
a library, its identity stays pinned, even if API validation or instance creation
then fails. Native initializers may already have changed process state. A load
that never obtains a library does not establish a pin.

## Installations and shared configuration

The caller supplies registry and installation-root paths. Neither path depends
on CLI defaults, the current directory, or service credentials. Installation
accepts a downloaded artifact or a local artifact copied into managed storage.
Both paths verify metadata and digest before registration; a network download
is not required for Rust applications or Python packages. A manifest is a trusted
installation instruction: the caller approves its local source or network
destination, including private repositories. The downloader rejects redirects
and limits each artifact to 512 MiB, checking both declared and streamed sizes.
Installed-library hashing also enforces the limit and uses bounded chunks.
Persisted manifests replace source URLs/paths with the managed library filename;
query tokens and other source details are not retained. Registry paths resolve
existing symlinks and parent aliases before locking or atomic replacement.

Artifacts use separate immutable paths identified by digest. Installing another
release never overwrites a library that a process might use. Preparation happens
outside the registry lock. Publishing registration and its selected-artifact
pointer uses one atomic `JsonStore` update under the lock. Installing the first
artifact leaves the extension disabled; changing versions preserves enablement.
On Unix, staged contents and the published artifact directory and its ancestors
are synced before the registry update. Other platforms retain JsonStore's
file-sync behavior without directory sync; power-loss durability depends on the
filesystem and platform. Publication or sync failure leaves artifacts unregistered
or retained; it never deletes a previously published library.

`remove` retains artifact files, including files potentially used by another
process. CLI output and documentation must state that removal does not reclaim
their disk space. A future cleanup mechanism needs its own cross-process policy;
`Arc` counts cannot establish that another process has stopped using a file.

Continue using shared configuration sources and profiles for application
configuration, and `JsonStore` for locked JSON persistence. The manager owns
extension-specific validation and selection policy. It does not introduce a
second configuration reader, merger, file-lock protocol, or atomic-write helper.
Preserve `JsonStore`'s distinction between failure before and after replacement;
an error after replacement must not trigger a blind rollback or retry.

## States, failures, and public surfaces

Registration, selection, and enablement are persistent state. Module loading and
instances are process state. A disabled extension can still have live instances.
Compatibility, initialization, and invocation failures are operation results;
they do not become an ambiguous stored `status` field.

Errors distinguish missing or disabled registration, unsupported manifest/ABI,
target mismatch, digest mismatch, missing/incompatible capability, ambiguous or
stale resolution, `RestartRequired`, load failure, initialization failure, and
invocation failure. Typed details preserve actionable identity and causes.
Default error displays and debug output exclude configuration and raw plugin
error payloads, which can contain secrets.

`required` means the configured extension must be installed, enabled, compatible
with requested capabilities, and successfully loaded. Successful creation is
also necessary when an instance is requested. It does not require successful
attestation or appraisal. Missing or disabled optional entries may be skipped;
failures of an enabled, present entry are reported without hidden fallback.

Rust exposes the manager, resolved artifacts, instances, and owned invocation
results. Python wraps the same implementation without exposing raw pointers and
releases the GIL around native operations where needed for the concurrency
contract. CLI management commands install, list, select, enable, disable, and
remove registrations without executing plugin code. Native loading is explicit
in both Rust and Python.

The experimental Rust ABI is not accepted as C ABI v1. Legacy manifests and
registry data need explicit conversion or reinstall instructions. Unsupported
formats produce actionable errors and are not guessed, silently rewritten, or
deleted.

TLS handshakes, exporters, EA/EXPat exchange, HTTP connection pools, request
gates, Confido appraisal policy, TES task/result rules, and registration of CLI
commands from plugins remain outside the manager. Adapters can consume its
capabilities without moving their domain policy into the loader.

## Implementation stages and acceptance evidence

Each stage includes its tests, affected examples, and documentation updates.
The boundary suites are `lib/tests/extensions.rs`, `cli/tests/extensions.rs`,
and `python/tests/test_extensions.py`; both languages use the same C fixture.
The table records acceptance requirements, not a claim of coverage on every OS.

| Stage | Deliverable | Acceptance evidence |
| --- | --- | --- |
| 1. ABI and instances | Separate ABI crate and matching C header; module/instance ownership; serialized instance operations | Small or unsupported API tables fail before instance creation; appended fields work; configurations remain independent; one instance never overlaps its operations; different instances overlap; create/call/drop work on different OS threads; buffers are freed before destruction; handles outlive the manager |
| 2. Manifest and compatibility | Versioned metadata; immutable resolution; runtime validation; structured failures | ABI, target, digest, and required-interface mismatches fail; exported metadata disagreements fail; selection changes never retarget an existing resolution; ambiguous artifacts require a pin; initialization failure never selects a fallback |
| 3. Versioned installations and admission | Immutable local/downloaded artifacts; atomic selection; live registry admission; process coordinator; unregister-only removal | Stale managers observe disable/remove; an admitted creation can finish afterward; subsequent admissions fail; plugin code can acquire the registry lock; racing different-artifact loads have one winner; dropping handles does not clear the pin; updating preserves active calls; another process can load the newly selected artifact; removal retains files |
| 4. CLI and Python | Thin management commands; Python instances and invocation using the Rust loader | Management commands never trigger a native-load marker; Rust and Python call the same fixture; Python instances preserve isolation and concurrent progress; no raw pointers reach Python callers; local installation works without HTTP |

Stage 1 finalizes the actual C declarations and the matching Rust wrapper API
before downstream implementation. Stages 2 and 3 connect the loader to manager
policy; they must not introduce a public path that bypasses admission. Finish
with the workspace, downstream, Python, and documentation checks appropriate to
the changed surfaces and the repository's pre-PR review.
