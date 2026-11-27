#ifndef GA4GH_EXTENSION_H
#define GA4GH_EXTENSION_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* All structs use the target's C layout and calling convention. */
#define GA4GH_EXTENSION_ABI_V1 1u
#define GA4GH_EXTENSION_OK 0

typedef struct {
    const uint8_t *data;
    size_t len;
} Ga4ghBytes;
typedef struct {
    uint8_t *data;
    size_t len;
} Ga4ghOwnedBytes;
typedef struct {
    uint32_t abi_version;
    uint32_t struct_size;
} Ga4ghHeader;
typedef struct Ga4ghExtensionApiV1 {
    Ga4ghHeader header;
    /* Borrowed UTF-8 JSON: name, version, target, abi_version, capabilities.
       capabilities maps identifiers to arrays of positive integer revisions. */
    Ga4ghBytes descriptor;
    int32_t (*create_instance)(Ga4ghBytes config, void **instance, Ga4ghOwnedBytes *error);
    int32_t (*invoke)(void *instance, Ga4ghBytes capability, uint32_t revision,
                      Ga4ghBytes operation, Ga4ghBytes input, Ga4ghOwnedBytes *output);
    void (*destroy_instance)(void *instance);
    void (*free_buffer)(void *instance, Ga4ghOwnedBytes buffer);
} Ga4ghExtensionApiV1;

const Ga4ghExtensionApiV1 *ga4gh_extension_get_api_v1(void);

/* The returned pointer is aligned, non-null, immutable, and valid until unload.
   The fixed header is always readable. The host checks version and size before
   reading the v1 fields. Compatible extensions append fields only.
   All function pointers are required. No panic/exception may unwind across ABI.

   Inputs are borrowed for the call. The plugin copies anything it retains.
   Outputs start as {NULL,0}. A nonzero status returns private error bytes.
   create_instance failure leaves *instance NULL; success returns a non-null
   instance and an empty error buffer. The host calls free_buffer exactly once
   for every output, including empty outputs, on both success and failure.
   free_buffer receives NULL instance for create_instance outputs. The plugin
   must accept {NULL,0}. No host allocator frees plugin memory.

   Operations of one instance (including output copying/freeing) are serialized.
   destroy_instance runs once after all its operations and buffer releases.
   Different instances, creation, destruction, and free_buffer may run in parallel.
   Every operation may run on a different OS thread. Thread affinity is unsupported.
   Descriptor storage remains immutable. Plugin-global state must be thread-safe.
   The host holds the module alive throughout all calls and buffer releases.
   Native code is trusted: this interface does not sandbox invalid pointers or code.
*/
#ifdef __cplusplus
}
#endif
#endif
