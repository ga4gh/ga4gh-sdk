# GA4GH native extension ABI v1

This crate contains only C-compatible declarations and depends on `core`.
Plugins do not need the SDK, CLI, or Python bindings. The matching declarations
and ownership/threading requirements are in
[`include/ga4gh_extension.h`](include/ga4gh_extension.h).

Export `ga4gh_extension_get_api_v1` with C linkage and return a module-lifetime
pointer to an immutable `ApiV1`. Set `header.abi_version` to `ABI_VERSION` and
`header.struct_size` to the complete table size. Provide all four function
pointers and an immutable UTF-8 JSON descriptor, for example:

```json
{"name":"example","version":"1.0.0","target":"aarch64-apple-darwin","abi_version":1,"capabilities":{"example":[1,2]}}
```

The descriptor's identity and interface sets must match the installation manifest.
Inputs are borrowed until return. The plugin allocates instances and output
buffers and provides their destruction functions. The host copies output and
frees it through the plugin on both success and failure. No exception or panic
may cross the boundary. Calls on different instances and resource releases may
run concurrently, on arbitrary OS threads.

The repository's C fixture in
`lib/tests/fixtures/native-extension/extension.c` demonstrates the ABI and is
called by both Rust and Python tests. The SDK
[extension guide](https://github.com/ga4gh/ga4gh-sdk/blob/main/docs/extensions.md)
describes installation, resolution, and migration from the experimental Rust ABI.
