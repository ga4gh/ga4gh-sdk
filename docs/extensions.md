# Experimental native extensions

The Extension Manager installs artifacts, resolves compatible releases, loads
native modules, and provides independent instances and invocation handles.
This implements the v1 contract for
[#64](https://github.com/ga4gh/ga4gh-sdk/issues/64). The
[design record](extension-manager-v1.md) records the scope and concurrency rules.
Downstream crate composition remains described in [downstream.md](downstream.md).

Native loading is explicit and requires trusted code. The manager does not
sandbox a plugin. Library initializers can run before exported API validation.

## Manifest and installations

A v1 manifest describes one artifact:

```json
{
  "manifest_version": 1,
  "name": "example",
  "version": "1.0.0",
  "target": "aarch64-apple-darwin",
  "abi_version": 1,
  "digest": "<128 lowercase SHA-512 hexadecimal characters>",
  "capabilities": {"example": [1, 2]},
  "path": "./libexample.dylib"
}
```

Replace `target` with the plugin build's full Rust target triple, including the
platform ABI. Rust callers can read `extension::HOST_TARGET`; Python callers use
`ExtensionManager.host_target()`. Replace `digest` with the SHA-512 digest of the
library bytes. Release versions use SemVer. Capability interface revisions are
independent positive integers. Names contain ASCII letters, digits, `-`, `_`,
and `.`; empty names, `.` and `..` are rejected.

`path` accepts a local file relative to the manifest, an absolute local file, or
an HTTP(S) URL without userinfo or a fragment. Local files are copied into managed
storage; applications and Python packages need no download. Both source paths
verify the bytes before publication. Installation never executes the library.

Artifacts are published at
`<installation-root>/<name>/<digest>/library.<platform suffix>`. Published files
are never overwritten by manager operations. The installation directory must be
trusted: applications must not let untrusted writers replace its files or parent
paths. Checksums identify bytes; they do not authenticate a publisher.

Publishing an artifact uses the shared `JsonStore` lock mechanism for that
artifact. Registration and selection use one atomic registry update. Copying,
network I/O, and hashing do not hold the registry lock. Mutations reload the latest
registry, so stale managers preserve unrelated changes. `get_extensions()` is a
metadata snapshot; `refresh()` reloads it. Resolution and instance admission
always read current metadata.

A first installation starts disabled. Installing another artifact selects it
and preserves enablement. It keeps earlier artifacts registered. If persistence
fails after replacement, `JsonStoreIoError::replacement_completed()` remains
inspectable through the error source. Inspect the registry before retrying;
unregistered published artifacts may remain after an earlier failure.

## CLI management

```sh
ga4gh-cli extension add /path/to/manifest.json
ga4gh-cli extension enable example
ga4gh-cli extension list
ga4gh-cli extension select example <installed-sha512-digest>
ga4gh-cli extension disable example
ga4gh-cli extension remove example
```

`list` shows each registered artifact with its version, target, digest, and
selection status.

The default registry is `~/.ga4gh-cli/extensions.json`; installations are below
`~/.ga4gh-cli/extensions/`. A downstream `CliContext` uses its own configuration
directory. Profiles share installations. Management commands require no service
configuration and never load plugin code, including when enabling or selecting.

`disable` blocks new instance admissions. `remove` unregisters the extension.
**Removal retains artifact files and does not reclaim disk space.** Existing
instances and invocation handles continue working in both cases. Physical
cleanup is a separate, future mechanism because other processes may use files.

## Rust loading and invocation

```rust
use ga4gh_sdk::utils::extension::Capabilities;
use ga4gh_sdk::utils::extension_manager::ExtensionManager;

fn example() -> Result<(), Box<dyn std::error::Error>> {
let manager = ExtensionManager::with_paths("state/extensions.json", "native-artifacts")?;
let required: Capabilities = [("example".to_owned(), vec![1, 2])].into();
let resolved = manager.resolve("example", None, None, &required)?;
// SAFETY: the caller trusts this artifact to meet the complete C ABI v1 contract.
let instance = unsafe { manager.create_instance(&resolved, br#"{"client":"one"}"#)? };
let method = instance.method("example", "operation")?;
let result: Vec<u8> = method.call(b"adapter-defined input")?;
let _ = result;
Ok(())
}
```

With no version or digest requirement, resolution uses the explicitly selected
artifact. A release requirement such as `Some("^1.2")` chooses the highest
compatible installed release. A digest pins exact bytes. Matching releases with
the same precedence and different digests require a pin or return
`AmbiguousArtifact`. Each required capability negotiates its highest common
interface revision. Resolution never loads code.

`ResolvedExtension` freezes the version, target, digest, ABI, artifact location,
and negotiated interfaces. Creation uses exactly that result. Selecting another
artifact cannot retarget it. An old resolution remains usable while that exact
artifact is registered and enabled, subject to the process constraint below.
There is no fallback to another release after load or initialization failure.

Every `load` or `create_instance` checks the current registry under its short
writer lock and records admission. The lock is released before native execution.
A creation already admitted can finish after disable/removal; later admissions
fail, even through stale manager objects. A loaded module cannot create new
instances without manager admission.

One exact artifact per extension is allowed in a process, shared across managers
and registries. Loading another version or digest returns `RestartRequired` with
current and requested identities. Installing/selecting another version remains
allowed, and a new process can load it. The pin survives dropping every runtime
handle. It also survives failed API validation after the OS loaded the library,
because native initializers may have executed. First loading during application
runtime and reloading the same artifact are supported.

`manager.load(&resolved)` loads a module without creating an instance. Instances
retain their module; method handles retain their instance. Dropping the manager
therefore cannot invalidate a call. Each instance serializes invocation and
buffer release. Different instances, their creation, and destruction may overlap.
A single instance may be created, invoked, and destroyed on different OS threads.
Plugins requiring thread affinity do not meet v1.

## Shared application configuration

Read configuration with the existing Figment-backed `Configuration` APIs. A
service's `extensions` array stays in `Configuration::extra`:

```json
{
  "TES": {
    "base_path": "https://tes.example.org/ga4gh/tes/v1",
    "extensions": [{
      "name": "example",
      "required": true,
      "version": "^1",
      "capabilities": {"example": [1, 2]},
      "configuration": {"client": "one"}
    }]
  }
}
```

`unsafe { manager.from_configuration(&configuration) }` returns independent
instances. Only each entry's JSON `configuration` is passed to its creator;
service credentials are not forwarded. Missing or disabled optional entries are
skipped. A present enabled entry with incompatible requirements or failed
initialization returns an error. `required` enforces availability and loading,
including required capabilities; it adds no attestation or appraisal policy.

## Python

```python
import json
from ga4gh_sdk import ExtensionManager

manager = ExtensionManager("state/extensions.json", "native-artifacts")
# Install/enable first, or use a registry prepared by the application.
resolved = manager.resolve("example", capabilities={"example": [1, 2]})
# Explicitly execute trusted native code implementing C ABI v1.
instance = manager.create_instance(resolved, json.dumps({"client": "one"}).encode())
method = instance.method("example", "operation")
result: bytes = method.call(b"adapter-defined input")
```

Python wraps the same Rust manager and loader and exposes no raw pointers. Native
loading, creation, invocation, and resource release run without the GIL. Registry
and artifact paths belong to the caller. `install`, `list`, `select`, `enable`,
`disable`, and `remove` are metadata operations. `ExtensionError.kind`, `status`,
`current`, and `requested` provide classified failures. Its `private_payload`
contains plugin error bytes and may contain secrets; default diagnostics omit it.

## Plugin ABI and migration

The [small ABI crate](../extension-abi/src/lib.rs) and matching
[C header](../extension-abi/include/ga4gh_extension.h) define the entry point,
fixed table prefix, status codes, borrowed inputs, opaque instances, and
plugin-owned output buffers. The host copies results and calls the plugin's
`free_buffer` on success and failure before releasing the instance or module.
No Rust collections or dependency-specific values cross the native boundary.
Adapters define operation payloads; the manager treats them as opaque bytes.

The former Rust `init(Value)`/method ABI, manifest format, and registry format are
incompatible. Rebuild plugins against the C header, create a v1 manifest with a
fresh digest, and install into a new registry and artifact directory. Keep the
old registry and files until old processes stop using them. Unsupported metadata
is rejected without automatic conversion, rewriting, or deletion.

TLS handshakes, EA/EXPat, connection pools, request gates, appraisal policies,
TES task rules, and plugin-provided CLI commands remain separate adapter work.
