#!/usr/bin/env bash
# Runs the TRS integration suite against a temporary TRS-Filer.
#
# Starts TRS-Filer and its MongoDB on a private Docker network, publishes
# TRS-Filer on a free loopback port, waits for its service info, registers
# the tool in tests/fixtures/trs-filer/tool.json, runs the library and CLI
# tests with `trs_filer_integration_tests`, and removes the containers and the
# network again.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# TRS-Filer at commit d0671d3 of `dev` (the first one that serves TRS
# 2.0.2; amd64 only) and MongoDB 7.0, pinned by tag and digest.
trs_filer_image=elixircloud/trs-filer:sha-d0671d35207327ed2abf05af3b459aa776ca6b39@sha256:af44a7d6b08f04ed329269f6a9ef0f4729d0554f951ba265f1fc031833bcaa12
mongodb_image=mongo:7.0@sha256:c53852c00c23ca9f23fc39690350ffa9a80e3642fe8f96879726d8da0abe80a7
startup_timeout=${TRS_FILER_STARTUP_TIMEOUT_SECONDS:-120}
if ! [[ $startup_timeout =~ ^[0-9]+$ ]]; then
  echo 'TRS_FILER_STARTUP_TIMEOUT_SECONDS must be a non-negative integer' >&2
  exit 2
fi

if ! command -v docker >/dev/null || ! docker info >/dev/null 2>&1; then
  echo 'Docker is required and its daemon must be running.' >&2
  exit 1
fi
if ! command -v curl >/dev/null; then
  echo 'curl is required.' >&2
  exit 1
fi

suffix="ga4gh-sdk-trs-$$"
network=
mongodb_id=
trs_filer_id=

cleanup() {
  result=$?
  trap - EXIT INT TERM
  if [[ -n "$trs_filer_id" && "$result" -ne 0 ]]; then
    echo 'TRS-Filer container logs:' >&2
    docker logs --tail 120 "$trs_filer_id" >&2 || true
  fi
  # By name, so that containers whose `docker run` failed go, too.
  docker rm -f -v "$suffix-filer" "$suffix-mongodb" >/dev/null 2>&1 || true
  if [[ -n "$network" ]]; then
    docker network rm "$network" >/dev/null 2>&1 || true
  fi
  exit "$result"
}
trap cleanup EXIT INT TERM

network=$(docker network create "$suffix")
mongodb_id=$(docker run -d --name "$suffix-mongodb" --network "$network" \
  --network-alias mongodb "$mongodb_image")
# Port 0 on the host: Docker picks a free loopback port.
trs_filer_id=$(docker run -d --name "$suffix-filer" --network "$network" \
  --platform linux/amd64 -p 127.0.0.1::8080 "$trs_filer_image")

host_port=$(docker port "$trs_filer_id" 8080/tcp | head -n 1 | awk -F: '{print $NF}')
if [[ -z "$host_port" ]]; then
  echo 'Docker did not publish the TRS-Filer port.' >&2
  exit 1
fi
export TRS_FILER_BASE_URL="http://127.0.0.1:$host_port/ga4gh/trs/v2"

deadline=$(( $(date +%s) + startup_timeout ))
until curl --silent --show-error --fail --max-time 2 \
  "$TRS_FILER_BASE_URL/service-info" -o /dev/null 2>/dev/null; do
  if [[ $(docker inspect --format '{{.State.Running}}' "$trs_filer_id") != true ]]; then
    echo 'TRS-Filer exited before service-info became ready.' >&2
    exit 1
  fi
  if (( $(date +%s) >= deadline )); then
    echo "Timed out after ${startup_timeout}s waiting for $TRS_FILER_BASE_URL/service-info" >&2
    exit 1
  fi
  sleep 1
done

echo "TRS-Filer ready at $TRS_FILER_BASE_URL"

# `POST /tools` is an extension of TRS-Filer; it returns the new tool ID as
# a JSON string.
tool_id=$(curl --silent --show-error --fail-with-body --max-time 30 \
  -H 'Content-Type: application/json' \
  --data-binary "@$repo_root/tests/fixtures/trs-filer/tool.json" \
  "$TRS_FILER_BASE_URL/tools")
if ! [[ $tool_id =~ ^\"[A-Za-z0-9]+\"$ ]]; then
  echo "Unexpected response to the tool registration: $tool_id" >&2
  exit 1
fi
tool_id=${tool_id//\"/}
export TRS_FILER_TOOL_ID=$tool_id
echo "Registered tool $TRS_FILER_TOOL_ID"
cd "$repo_root"
cargo test --locked -p ga4gh-lib --features trs_filer_integration_tests \
  --test trs_filer -- --nocapture
cargo test --locked -p ga4gh-cli --features trs_filer_integration_tests \
  --test trs_filer -- --nocapture
