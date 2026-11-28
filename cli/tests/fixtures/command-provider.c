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
    int execute = revision == 1 && operation.len == 7 && !memcmp(operation.data, "execute", 7);
    int echo = execute
        && strstr(request, "\"profile\":\"remote\"") && strstr(request, "\"word\":[\"hello\"]");
    int submit = execute && strstr(request, "\"word\":[\"submit\"]");
    int outcome = revision == 1 && operation.len == 11 && !memcmp(operation.data, "wes_outcome", 11)
        && strstr(request, "\"submitted\"");
    int nonzero = outcome && strstr(request, "accepted-43");
    free(request);
    const char *reply = "PRIVATE-NATIVE-DETAIL";
    if (echo) reply = "{\"complete\":{\"stdout\":\"ok\\u001b\",\"stderr\":\"warning\\n\",\"exit_status\":3}}";
    if (submit) reply = "{\"wes_submit\":{\"request\":{\"workflow_params\":{},\"workflow_type\":\"CWL\",\"workflow_type_version\":\"v1.2\",\"workflow_url\":\"main.cwl\"},\"attachments\":[]}}";
    if (outcome) reply = "{\"stdout\":\"PRIVATE-INVALID-JSON\",\"json_stdout\":true,\"stderr\":\"\",\"exit_status\":0}";
    if (nonzero) reply = "{\"stdout\":\"\",\"stderr\":\"completion failed\",\"exit_status\":4}";
    output->len = strlen(reply);
    output->data = malloc(output->len);
    if (!output->data) { output->len = 0; return 1; }
    memcpy(output->data, reply, output->len);
    return echo || submit || outcome ? 0 : 9;
}

static void release(void *instance, Ga4ghOwnedBytes buffer) { free(buffer.data); }
static const Ga4ghExtensionApiV1 api = {
    {1, sizeof(Ga4ghExtensionApiV1)}, {(const uint8_t *)DESCRIPTOR, sizeof(DESCRIPTOR)-1},
    create, invoke, free, release
};

const Ga4ghExtensionApiV1 *ga4gh_extension_get_api_v1(void) { return &api; }
