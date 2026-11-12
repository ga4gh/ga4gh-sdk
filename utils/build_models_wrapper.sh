#!/bin/bash

# Regenerates the models of all GA4GH APIs in this repository from their pinned
# specifications.
#
# Usage:
#   utils/build_models_wrapper.sh [API_NAME...]
#
# Without arguments, all APIs in SPECS are regenerated; otherwise only the
# named ones. Run it from the repository root. The result must not differ from
# the committed models; a diff means that a spec, the generator version or a
# patch changed.
#
# Each SPECS entry is "<api_name>|<spec_url>|<destination_dir>|<generator_args>|<module_path>".
# An API may have several entries for retained versions when their models
# differ. Selecting that API regenerates every entry; each owns a distinct
# models directory. A supported profile with a compatible shape can reuse an
# existing generated set through a hand-written protocol adapter.
# Pin every spec to a tag. Hand edits of the generated models go into
# utils/patches/<api_name>.patch, which build_models.sh applies after
# generation. Hand-written types don't go into `models/`, because that folder
# is replaced on every run.

set -euo pipefail

RAW=https://raw.githubusercontent.com
SPECS=(
    "serviceinfo|$RAW/ga4gh-discovery/ga4gh-service-info/v1.0.0/service-info.yaml|lib/src/clients/serviceinfo/|"
    "registry|$RAW/ga4gh-discovery/ga4gh-service-registry/1.0.0/service-registry.yaml|lib/src/clients/registry/|"
    "tes|$RAW/ga4gh/task-execution-schemas/v1.1/openapi/task_execution_service.openapi.yaml|lib/src/clients/tes/|"
    "drs|$RAW/ga4gh/data-repository-service-schemas/drs-1.4.0/openapi/data_repository_service.openapi.yaml|lib/src/clients/drs/|--skip-validate-spec"
    "trs|$RAW/ga4gh/tool-registry-service-schemas/2.0.1/openapi/openapi.yaml|lib/src/clients/trs/wire/v2_0_1/||crate::clients::trs::wire::v2_0_1::models"
    "wes|$RAW/ga4gh/workflow-execution-service-schemas/1.1.0/openapi/workflow_execution_service.openapi.yaml|lib/src/clients/wes/|"
)

BUILD_SCRIPT="./utils/build_models.sh"
PATCH_DIR="./utils/patches"

selected() {
    local api="$1"
    shift
    if [ "$#" -eq 0 ]; then
        return 0
    fi
    local name
    for name in "$@"; do
        if [ "$name" = "$api" ]; then
            return 0
        fi
    done
    return 1
}

for name in "$@"; do
    found=false
    for entry in "${SPECS[@]}"; do
        if [ "${entry%%|*}" = "$name" ]; then
            found=true
        fi
    done
    if [ "$found" = false ]; then
        echo "Error: unknown API: $name" >&2
        exit 1
    fi
done

for entry in "${SPECS[@]}"; do
    IFS='|' read -r API_NAME SPEC_URL DESTINATION_DIR GENERATOR_ARGS MODULE_PATH <<< "$entry"
    if ! selected "$API_NAME" "$@"; then
        continue
    fi
    OPTIONS=()
    if [ -n "$MODULE_PATH" ]; then
        OPTIONS+=(-m "$MODULE_PATH")
    fi
    if [ -f "$PATCH_DIR/$API_NAME.patch" ]; then
        OPTIONS+=(-p "$PATCH_DIR/$API_NAME.patch")
    fi
    EXTRA=()
    if [ -n "$GENERATOR_ARGS" ]; then
        read -r -a EXTRA <<< "$GENERATOR_ARGS"
    fi
    echo "Generating models for $API_NAME from $SPEC_URL into $DESTINATION_DIR"
    "$BUILD_SCRIPT" ${OPTIONS[@]+"${OPTIONS[@]}"} "$SPEC_URL" "$API_NAME" "$DESTINATION_DIR" \
        -- ${EXTRA[@]+"${EXTRA[@]}"}
done
