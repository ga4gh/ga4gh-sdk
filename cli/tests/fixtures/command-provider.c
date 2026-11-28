#include "ga4gh_extension.h"
#include "fixture.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

__attribute__((constructor)) static void loaded(void) {
    FILE *marker = fopen(MARKER, "w");
    if (marker) fclose(marker);
}

static int32_t create(Ga4ghBytes config, void **instance, Ga4ghOwnedBytes *error) {
    *instance = malloc(1);
    return *instance ? 0 : 1;
}

static int32_t invoke(void *instance, Ga4ghBytes capability, uint32_t revision,
                      Ga4ghBytes operation, Ga4ghBytes input, Ga4ghOwnedBytes *output) {
    char *request = calloc(input.len + 1, 1);
    if (!request) return 1;
    memcpy(request, input.data, input.len);
    int success = revision == 1 && operation.len == 7 && !memcmp(operation.data, "execute", 7)
        && strstr(request, "\"profile\":\"remote\"") && strstr(request, "\"word\":[\"hello\"]");
    free(request);
    const char *reply = success ? "{\"complete\":{\"stdout\":\"ok\\u001b\",\"stderr\":\"warning\\n\",\"exit_status\":3}}" : "PRIVATE-NATIVE-DETAIL";
    output->len = strlen(reply);
    output->data = malloc(output->len);
    if (!output->data) { output->len = 0; return 1; }
    memcpy(output->data, reply, output->len);
    return success ? 0 : 9;
}

static void release(void *instance, Ga4ghOwnedBytes buffer) { free(buffer.data); }
static const Ga4ghExtensionApiV1 api = {
    {1, sizeof(Ga4ghExtensionApiV1)}, {(const uint8_t *)DESCRIPTOR, sizeof(DESCRIPTOR)-1},
    create, invoke, free, release
};

const Ga4ghExtensionApiV1 *ga4gh_extension_get_api_v1(void) { return &api; }
