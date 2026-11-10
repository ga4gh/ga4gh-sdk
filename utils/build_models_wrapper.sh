#!/bin/bash

# *Developer note*: Add entries to the SPECS associative array as shown in the examples below.
# Each entry should map an API name to a URL and a destination directory.
#
# Example:
# SPECS["serviceinfo"]="https://raw.githubusercontent.com/ga4gh-discovery/ga4gh-service-info/develop/service-info.yaml lib/src/clients/serviceinfo/"
# SPECS["tes"]="https://raw.githubusercontent.com/ga4gh/task-execution-schemas/develop/openapi/task_execution_service.openapi.yaml lib/src/clients/tes/"
#
# To add a new entry, follow this format:
# SPECS["<api_name>"]="<url> <destination_directory>"
#
# Replace <api_name> with the name of the API, <url> with the URL to the OpenAPI specification,
# and <destination_directory> with the path to the directory where the models should be generated.
#
# Example of adding a new entry:
# SPECS["newapi"]="https://example.com/path/to/newapi.yaml lib/src/clients/newapi/"


# Directory of the build script
BUILD_SCRIPT="./utils/build_models.sh"

# Iterate over the specifications and call the build script for each
for API_NAME in "${!SPECS[@]}"; do
    SPEC_URL=$(echo ${SPECS[$API_NAME]} | awk '{print $1}')
    DESTINATION_DIR=$(echo ${SPECS[$API_NAME]} | awk '{print $2}')

    echo "Generating models for $API_NAME from $SPEC_URL into $DESTINATION_DIR"
    $BUILD_SCRIPT "$SPEC_URL" "$API_NAME" "$DESTINATION_DIR"
done
