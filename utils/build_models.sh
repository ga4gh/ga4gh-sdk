#!/bin/bash

# Generates Rust models from a GA4GH OpenAPI specification.
#
# Usage:
#   utils/build_models.sh [-m MODULE_PATH] [-p PATCH_FILE] [-n] \
#       <OPENAPI_SPEC_PATH> <API_NAME> <DESTINATION_DIR> [-- GENERATOR_ARGS...]
#
# Example:
#   utils/build_models.sh -p utils/patches/wes.patch \
#       https://example.org/openapi.yaml wes lib/src/clients/wes/
#
# Arguments:
# - OPENAPI_SPEC_PATH: URL or file path of the OpenAPI specification.
# - API_NAME: name of the API, used for the default module path.
# - DESTINATION_DIR: directory that receives the `models` folder. An existing
#   `models` folder in it is replaced.
# - GENERATOR_ARGS: extra arguments for `openapi-generator-cli generate`.
#
# Options:
# - -m MODULE_PATH: Rust path of the generated `models` module, used in the
#   `use` statements of the models. Default: `crate::clients::<API_NAME>::models`.
#   Set it when generating models for another crate.
# - -p PATCH_FILE: unified diff with the hand edits of the models, applied to
#   the generated `models` folder with `patch -p1` (paths `a/models/...`).
# - -n: don't format the models with `rustfmt`. Patches apply to the
#   unformatted output, so use this to write them.
#
# Environment:
# - OPENAPI_GENERATOR_VERSION: generator version (default: 7.9.0). If
#   `openapi-generator-cli` is on the PATH, it is used (the npm wrapper is
#   pinned to this version) and must report it; otherwise the script runs the
#   generator's Docker image.
#
# `rustfmt` runs in the current directory, so from the repository root it
# uses the toolchain pinned in rust-toolchain.toml.
#
# The generator uses the model template in utils/templates/rust/, which keeps
# unknown fields and enum values (see docs/autogenerate_OAI_models.md).
#
# Steps: generate, fix the `use` statements, apply the patch, format with
# `rustfmt` (if installed), replace `<DESTINATION_DIR>/models`.

set -euo pipefail

usage() {
    echo "Usage: $0 [-m MODULE_PATH] [-p PATCH_FILE] [-n] <OPENAPI_SPEC_PATH> <API_NAME> <DESTINATION_DIR> [-- GENERATOR_ARGS...]" >&2
    exit 1
}

MODULE_PATH=""
PATCH_FILE=""
FORMAT=true
while getopts ":m:p:n" opt; do
    case "$opt" in
        m) MODULE_PATH="$OPTARG" ;;
        p) PATCH_FILE="$OPTARG" ;;
        n) FORMAT=false ;;
        *) usage ;;
    esac
done
shift $((OPTIND - 1))

if [ "$#" -lt 3 ]; then
    usage
fi
OPENAPI_SPEC_PATH="$1"
API_NAME="$2"
DESTINATION_DIR="$3"
shift 3
if [ "$#" -gt 0 ]; then
    if [ "$1" != "--" ]; then
        usage
    fi
    shift
fi
GENERATOR_ARGS=("$@")

MODULE_PATH="${MODULE_PATH:-crate::clients::$API_NAME::models}"
if ! [[ "$MODULE_PATH" =~ ^[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)*$ ]]; then
    echo "Error: invalid module path: $MODULE_PATH" >&2
    exit 1
fi
if [ -n "$PATCH_FILE" ]; then
    if [ ! -f "$PATCH_FILE" ]; then
        echo "Error: patch file not found: $PATCH_FILE" >&2
        exit 1
    fi
    PATCH_FILE="$(cd "$(dirname "$PATCH_FILE")" && pwd)/$(basename "$PATCH_FILE")"
fi
OPENAPI_GENERATOR_VERSION="${OPENAPI_GENERATOR_VERSION:-7.9.0}"
TEMPLATE_DIR="$(cd "$(dirname "$0")" && pwd)/templates/rust"
if [ ! -f "$TEMPLATE_DIR/model.mustache" ]; then
    echo "Error: model template not found: $TEMPLATE_DIR/model.mustache" >&2
    exit 1
fi
IMPORT_PATTERN='^use[[:space:]]+crate::models[[:space:]]*;'
SED_RULE="s/$IMPORT_PATTERN/#![allow(unused_imports)]\n#![allow(clippy::empty_docs)]\nuse $MODULE_PATH;/"

TEMP_OUTPUT_DIR=$(mktemp -d)
trap 'rm -rf "$TEMP_OUTPUT_DIR"' EXIT

run_generator() {
    local spec="$OPENAPI_SPEC_PATH"
    if command -v openapi-generator-cli >/dev/null 2>&1; then
        # The npm wrapper (@openapitools/openapi-generator-cli) reads the
        # generator version from openapitools.json in its working directory.
        local cli_dir="$TEMP_OUTPUT_DIR/cli"
        mkdir -p "$cli_dir"
        printf '{"generator-cli": {"version": "%s"}}\n' "$OPENAPI_GENERATOR_VERSION" \
            > "$cli_dir/openapitools.json"
        if [ -f "$spec" ]; then
            spec="$(cd "$(dirname "$spec")" && pwd)/$(basename "$spec")"
        fi
        local version
        version=$(cd "$cli_dir" && openapi-generator-cli version | tail -n 1)
        if [ "$version" != "$OPENAPI_GENERATOR_VERSION" ]; then
            echo "Error: openapi-generator-cli is $version, expected $OPENAPI_GENERATOR_VERSION" >&2
            exit 1
        fi
        (cd "$cli_dir" && openapi-generator-cli generate -g rust -i "$spec" -o "$TEMP_OUTPUT_DIR" \
            -t "$TEMPLATE_DIR" \
            --additional-properties=useSingleRequestParameter=true \
            ${GENERATOR_ARGS[@]+"${GENERATOR_ARGS[@]}"})
    elif command -v docker >/dev/null 2>&1; then
        local mounts=(-v "$TEMP_OUTPUT_DIR:/out" -v "$TEMPLATE_DIR:/templates:ro")
        if [ -f "$spec" ]; then
            local spec_dir
            spec_dir="$(cd "$(dirname "$spec")" && pwd)"
            mounts+=(-v "$spec_dir:/spec:ro")
            spec="/spec/$(basename "$spec")"
        fi
        docker run --rm -u "$(id -u):$(id -g)" "${mounts[@]}" \
            "openapitools/openapi-generator-cli:v$OPENAPI_GENERATOR_VERSION" \
            generate -g rust -i "$spec" -o /out -t /templates \
            --additional-properties=useSingleRequestParameter=true \
            ${GENERATOR_ARGS[@]+"${GENERATOR_ARGS[@]}"}
    else
        echo "Error: neither openapi-generator-cli nor docker is installed" >&2
        exit 1
    fi
}

run_generator

MODELS_DIR="$TEMP_OUTPUT_DIR/src/models"
if [ ! -d "$MODELS_DIR" ]; then
    echo "Error: the generator wrote no models" >&2
    exit 1
fi

while IFS= read -r -d '' file; do
    if ! grep -Eq "$IMPORT_PATTERN" "$file"; then
        continue
    fi
    if [[ "$OSTYPE" == "darwin"* ]]; then
        sed -E -i '' "$SED_RULE" "$file"
    else
        sed -E -i "$SED_RULE" "$file"
    fi
    if ! grep -Fq '#![allow(clippy::empty_docs)]' "$file" ||
       ! grep -Fq "use $MODULE_PATH;" "$file"; then
        echo "Error: failed to rewrite generated model import in $file" >&2
        exit 1
    fi
done < <(find "$MODELS_DIR" -name '*.rs' -print0)

if [ -n "$PATCH_FILE" ]; then
    patch -p1 --batch --forward --fuzz=0 --no-backup-if-mismatch -d "$TEMP_OUTPUT_DIR/src" -i "$PATCH_FILE"
fi

if [ "$FORMAT" = true ]; then
    if command -v rustfmt >/dev/null 2>&1; then
        find "$MODELS_DIR" -name '*.rs' -exec rustfmt --edition 2021 {} +
    else
        echo "Warning: rustfmt not found; run 'cargo fmt --all' afterwards" >&2
    fi
fi

rm -rf "$DESTINATION_DIR/models"
mkdir -p "$DESTINATION_DIR"
cp -r "$MODELS_DIR" "$DESTINATION_DIR"

echo "Models for $API_NAME generated in $DESTINATION_DIR/models"
