# Experimental native extensions

This branch implements the runtime loader and installation commands from
[#64](https://github.com/ga4gh/ga4gh-sdk/issues/64). This is separate from the
downstream crate composition described in [downstream.md](downstream.md).
The loader uses an experimental Rust ABI: host and library must use the same
compiler and compatible dependency builds. It is not a stable plugin ABI.

## Manage installations

Run these commands from any directory after building or installing `ga4gh-cli`:

```sh
ga4gh-cli extension add /path/to/manifest.json
ga4gh-cli extension enable example
ga4gh-cli extension list
ga4gh-cli extension disable example
ga4gh-cli extension remove example
```

The manifest contains a library URL and the expected SHA-512 hex digest:

```json
{
  "name": "example",
  "version": "1.0.0",
  "path": "https://example.org/example.so",
  "checksum": "<128 hexadecimal characters>",
  "description": "An experimental native extension"
}
```

Use a platform-specific shared library and replace the checksum placeholder
with the digest of its bytes. Names use ASCII letters, digits, `-`, `_` and
`.`; `.` and `..` are rejected. Installation starts disabled. The registry is
`~/.ga4gh-cli/extensions.json`, and libraries and manifests are installed below
`~/.ga4gh-cli/extensions/<name>/`. A downstream `CliContext` uses the directory
of its own configuration file. Profiles share this installation registry.
Management commands do not require a service configuration and never load code,
including when enabling an installation.

Downloads and checksums are verified before registering an installation. Registry
writes replace the file atomically. Use a single writer at a time; manager
instances are snapshots and do not merge concurrent edits.

## Load an extension in Rust

Read service configuration through the existing `Configuration::from_path` or
`from_path_with_profile` API. An optional service-section `extensions` array
remains in `Configuration::extra`:

```json
{
  "TES": {
    "base_path": "https://tes.example.org/ga4gh/tes/v1",
    "extensions": [
      {"name": "example", "required": true, "configuration": {}}
    ]
  }
}
```

`unsafe { ExtensionManager::from_configuration(registry_path, &config) }` loads
only selected, enabled libraries. A missing or disabled required extension
returns an error. An absent optional extension is skipped. Only each entry's
`configuration` value reaches its initializer; service credentials are not
forwarded. Loading native code is an explicit unsafe operation: the caller
must trust the library and satisfy the ABI and threading contract documented
on `InstalledExtension::load`.

The library exports `init(Value) -> Vec<&'static [&'static str]>` using the Rust
ABI. Every descriptor contains the method category, internal name and symbol
name. Method symbols use the Rust ABI and take and return `serde_json::Value`.
`lookup_extension_methods(category)` discovers methods;
`unsafe { method.call(input) }` invokes one under its extension-specific contract.
Retained methods and cloned records keep their library alive. Optional
`extern "C" fn deinit()` runs when the last handle is dropped.

The loader and manager are currently Rust-only. Python's existing configuration
and client APIs are preserved; they do not implicitly load native extensions.

## Integration with current main

| Original branch work | Current disposition |
| --- | --- |
| Dynamic loading and installation commands | Retained here, with explicit loading, shared library ownership, validated paths and current SDK errors. |
| Configuration paths, profiles, bearer auth, model constructors and CLI output | Use the implementations already merged into `main`. The old `~/.ga4gh` path and replacement constructor signatures are superseded. |
| TLS verification in `Transport::new` | Deferred to [#73](https://github.com/ga4gh/ga4gh-sdk/issues/73), as agreed in the [issue discussion](https://github.com/ga4gh/ga4gh-sdk/issues/64#issuecomment-2624490160). The certificate-fetching PoC and its disabled certificate validation are not restored. |
| Attested TLS provider | Owned by [#72](https://github.com/ga4gh/ga4gh-sdk/issues/72), after #73. |

For an old PoC installation, reinstall its manifest with the current CLI to
create paths under `~/.ga4gh-cli`; check the manifest digest before enabling it.
Move service settings to the current configuration format. `Transport::new`
keeps its current signature and behavior. The existing `Transport::with_client`
and `with_client_and_configured_auth` constructors remain the injection points
for caller-owned TLS policy. This branch does not claim to complete #73 or #72.

`cargo test -p ga4gh-lib --test extensions` covers installation failures,
path containment, required settings and a real shared library's lifetime.
`cargo test -p ga4gh-cli --test extensions` covers context-owned paths and
binary output and failure status. Both run in the default workspace tests.
