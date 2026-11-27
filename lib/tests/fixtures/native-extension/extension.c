/* Shared fixture used through the C header by Rust and Python boundary tests. */
#include "ga4gh_extension.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <windows.h>
#define EXPORT __declspec(dllexport)

static void pause_ms(void) {
    Sleep(2);
}
#else
#include <unistd.h>
#define EXPORT __attribute__((visibility("default")))

static void pause_ms(void) {
    usleep(2000);
}
#endif

#ifndef PLUGIN_NAME
#define PLUGIN_NAME "fixture"
#endif
#ifndef PLUGIN_VERSION
#define PLUGIN_VERSION "1.0.0"
#endif
#ifndef TABLE_ABI
#define TABLE_ABI 1
#endif
#ifndef TABLE_SIZE
#define TABLE_SIZE sizeof(Ga4ghExtensionApiV1)
#endif
#ifndef TARGET_TRIPLE
#error Define TARGET_TRIPLE to the host Rust target triple
#endif

static const char descriptor[] =
    "{\"name\":\"" PLUGIN_NAME "\",\"version\":\"" PLUGIN_VERSION
    "\",\"target\":\"" TARGET_TRIPLE "\",\"abi_version\":1,\"capabilities\":{\"test\":[1,2]}}";

typedef struct {
    unsigned char *configuration;
    size_t configuration_len;
    size_t pending_buffers;
    char drop_path[2048];
} Instance;

static Ga4ghOwnedBytes copy_bytes(const unsigned char *data, size_t len) {
    Ga4ghOwnedBytes output = {NULL, len};
    if (len) {
        output.data = (unsigned char *)malloc(len);
        if (!output.data) {
            abort();
        }
        memcpy(output.data, data, len);
    }
    return output;
}

static int bytes_equal(Ga4ghBytes bytes, const char *text) {
    return bytes.len == strlen(text) &&
        (!bytes.len || memcmp(bytes.data, text, bytes.len) == 0);
}

static void path_string(char *output, size_t capacity, Ga4ghBytes input, const char *suffix) {
    size_t len = input.len;
    if (len + strlen(suffix) >= capacity) {
        abort();
    }
    if (len) {
        memcpy(output, input.data, len);
    }
    strcpy(output + len, suffix);
}

static void touch_file(const char *path, const char *text) {
    FILE *file = fopen(path, "wb");
    if (!file) {
        abort();
    }
    fputs(text, file);
    fclose(file);
}

static int file_exists(const char *path) {
    FILE *file = fopen(path, "rb");
    if (!file) {
        return 0;
    }
    fclose(file);
    return 1;
}

static int wait_for_release(const char *path) {
    time_t deadline = time(NULL) + 15;
    while (!file_exists(path)) {
        if (time(NULL) > deadline) {
            return 0;
        }
        pause_ms();
    }
    return 1;
}

static int32_t create_instance(Ga4ghBytes configuration, void **output, Ga4ghOwnedBytes *error) {
    if (configuration.len >= 4 && memcmp(configuration.data, "fail", 4) == 0) {
        *error = copy_bytes(configuration.data, configuration.len);
        return 7;
    }
    Instance *instance = (Instance *)calloc(1, sizeof(Instance));
    if (!instance) {
        abort();
    }
    Ga4ghOwnedBytes copied = copy_bytes(configuration.data, configuration.len);
    instance->configuration = copied.data;
    instance->configuration_len = copied.len;
    *output = instance;
    return 0;
}

static int32_t invoke(
    void *handle,
    Ga4ghBytes capability,
    uint32_t revision,
    Ga4ghBytes operation,
    Ga4ghBytes input,
    Ga4ghOwnedBytes *output
) {
    Instance *instance = (Instance *)handle;
    int32_t status = 0;
    if (!bytes_equal(capability, "test") || (revision != 1 && revision != 2)) {
        return 9;
    }
    if (bytes_equal(operation, "configuration")) {
        *output = copy_bytes(instance->configuration, instance->configuration_len);
    } else if (bytes_equal(operation, "set_drop_path")) {
        path_string(instance->drop_path, sizeof(instance->drop_path), input, "");
    } else if (bytes_equal(operation, "wait")) {
        char entered[2048];
        char release[2048];
        path_string(entered, sizeof(entered), input, ".entered");
        path_string(release, sizeof(release), input, ".release");
        touch_file(entered, "entered");
        if (!wait_for_release(release)) {
            return 10;
        }
        *output = copy_bytes(input.data, input.len);
    } else if (bytes_equal(operation, "touch")) {
        char path[2048];
        path_string(path, sizeof(path), input, "");
        touch_file(path, "touched");
    } else {
        *output = copy_bytes(input.data, input.len);
        if (bytes_equal(operation, "fail")) {
            status = 8;
        }
    }
    if (output->data) {
        instance->pending_buffers++;
    }
    return status;
}

static void destroy_instance(void *handle) {
    Instance *instance = (Instance *)handle;
    if (instance->drop_path[0]) {
        const char *state = instance->pending_buffers ? "unreleased" : "destroyed";
        touch_file(instance->drop_path, state);
    }
    free(instance->configuration);
    free(instance);
}

static void free_buffer(void *handle, Ga4ghOwnedBytes buffer) {
    if (handle && buffer.data) {
        ((Instance *)handle)->pending_buffers--;
    }
    free(buffer.data);
}

/* Extra bytes exercise compatible extension of the API table. */
static const struct {
    Ga4ghExtensionApiV1 api;
    uint64_t future_field;
} table = {
    {
        {TABLE_ABI, TABLE_SIZE},
        {(const uint8_t *)descriptor, sizeof(descriptor) - 1},
        create_instance,
        invoke,
        destroy_instance,
        free_buffer
    },
    42
};

EXPORT const Ga4ghExtensionApiV1 *ga4gh_extension_get_api_v1(void) {
#ifdef ENTRY_WAIT_PREFIX
    touch_file(ENTRY_WAIT_PREFIX ".entered", "entered");
    if (!wait_for_release(ENTRY_WAIT_PREFIX ".release")) {
        abort();
    }
#endif
    return &table.api;
}
