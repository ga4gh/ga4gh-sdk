#!/usr/bin/env bash
# Runs the CLI login suite against a temporary Keycloak.
#
# Starts Keycloak in development mode with the test realm in
# tests/keycloak/realm.json, publishes it on a free loopback port, waits for
# the realm's discovery document, runs cli/tests/keycloak_login.rs with
# `keycloak_integration_tests`, and removes the container again. The realm,
# its user and its password are for this test only.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
# Keycloak 26.8.2, pinned by tag and digest.
keycloak_image=quay.io/keycloak/keycloak:26.8.2@sha256:c68e286cacd87b942ac1939a0a1481461109d6acd3a57dc496969a319a76bbf0
realm=ga4gh-sdk-test
startup_timeout=${KEYCLOAK_STARTUP_TIMEOUT_SECONDS:-180}
if ! [[ $startup_timeout =~ ^[0-9]+$ ]]; then
  echo 'KEYCLOAK_STARTUP_TIMEOUT_SECONDS must be a non-negative integer' >&2
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

cd "$repo_root"
test_args=(--locked -p ga4gh-cli --features keycloak_integration_tests
  --test keycloak_login)
# Build before Keycloak starts, so that its startup limit doesn't include
# the build.
cargo test "${test_args[@]}" --no-run

# Random part: the name must not clash with containers of other runs.
name="ga4gh-sdk-keycloak-$$-$RANDOM"
started=false

cleanup() {
  result=$?
  trap - EXIT INT TERM
  if [[ $started == true && "$result" -ne 0 ]]; then
    echo 'Keycloak container logs:' >&2
    docker logs --tail 120 "$name" >&2 || true
  fi
  docker rm -f -v "$name" >/dev/null 2>&1 || true
  exit "$result"
}
trap cleanup EXIT INT TERM

# `start-dev` serves plain HTTP, which the CLI accepts on loopback only.
# The realm file is imported at startup; no admin user is created.
docker run -d --name "$name" -p 127.0.0.1::8080 \
  -v "$repo_root/tests/keycloak/realm.json:/opt/keycloak/data/import/realm.json:ro" \
  "$keycloak_image" start-dev --import-realm >/dev/null
started=true

host_port=$(docker port "$name" 8080/tcp | head -n 1 | awk -F: '{print $NF}')
if [[ -z "$host_port" ]]; then
  echo 'Docker did not publish the Keycloak port.' >&2
  exit 1
fi
# Keycloak derives the issuer from the request's host and port.
export KEYCLOAK_ISSUER="http://127.0.0.1:$host_port/realms/$realm"

deadline=$(( $(date +%s) + startup_timeout ))
until curl --silent --show-error --fail --max-time 2 \
  "$KEYCLOAK_ISSUER/.well-known/openid-configuration" -o /dev/null 2>/dev/null; do
  if [[ $(docker inspect --format '{{.State.Running}}' "$name") != true ]]; then
    echo 'Keycloak exited before the realm became ready.' >&2
    exit 1
  fi
  if (( $(date +%s) >= deadline )); then
    echo "Timed out after ${startup_timeout}s waiting for $KEYCLOAK_ISSUER" >&2
    exit 1
  fi
  sleep 1
done

echo "Keycloak ready at $KEYCLOAK_ISSUER"

cargo test "${test_args[@]}" -- --nocapture
