#!/usr/bin/env bash
# Integration suite of the TES client against proTES, a TES gateway that
# requires access tokens, relaying tasks to Funnel (see tests/README.md).
# Starts the services of tests/protes/compose.yaml, gets an access token of
# the fake identity provider, runs `lib/tests/protes_tes.rs` and removes the
# services again. Needs Docker with Compose, curl and the `funnel`
# executable on PATH (or in FUNNEL_BIN), as for run-integration-tests.sh.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
compose_file="$repo_root/tests/protes/compose.yaml"
# proTES release 0.6.0, whose smoke test provides the fake identity
# provider; bump together with the images of the Compose file
protes_commit=55c4d1591cd3be883ad0b4fd2da01d49976414cb
fake_tes_sha256=ecb63ef21fd772b257c5d71c2f6b24d5c240e313e76ade4bfbb6bb06ea8803cd
# Image of the Funnel service, also used here to clean up its files
export DOCKER_CLI_IMAGE=docker.io/library/docker:28.5.2-cli@sha256:625d9431a9f54c5a2bc90f24f0e1c3d55b1349fd857dd85035f98c2c9acbdd4d
startup_timeout=${PROTES_STARTUP_TIMEOUT_SECONDS:-300}
port=${PROTES_PORT:-8090}
docker_socket=${DOCKER_SOCKET:-}

for value in "$startup_timeout" "$port"; do
  if ! [[ $value =~ ^[0-9]+$ ]]; then
    echo 'PROTES_STARTUP_TIMEOUT_SECONDS and PROTES_PORT must be integers' >&2
    exit 2
  fi
done
if ! command -v docker >/dev/null || ! docker info >/dev/null 2>&1; then
  echo 'Docker is required and its daemon must be running.' >&2
  exit 1
fi
funnel_bin=${FUNNEL_BIN:-$(command -v funnel || true)}
if [[ -z "$funnel_bin" || ! -x "$funnel_bin" ]]; then
  echo 'Install funnel (see tests/README.md) or set FUNNEL_BIN.' >&2
  exit 1
fi
if [[ -z "$docker_socket" ]]; then
  docker_host=$(docker context inspect --format '{{.Endpoints.docker.Host}}')
  docker_socket=${docker_host#unix://}
fi
if [[ ! -S "$docker_socket" ]]; then
  echo "Docker socket is unavailable: $docker_socket" >&2
  exit 1
fi

# Funnel's task directories must have the same path on the host as in its
# container (see the Compose file)
temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/ga4gh-sdk-protes.XXXXXXXX")
temp_dir=$(cd "$temp_dir" && pwd -P)
mkdir "$temp_dir/funnel"
export COMPOSE_FILE="$compose_file"
export COMPOSE_PROJECT_NAME="ga4gh-sdk-protes-$$"
export FAKE_TES_FILE="$temp_dir/fake_tes.py"
export FUNNEL_BIN
FUNNEL_BIN=$(cd "$(dirname "$funnel_bin")" && pwd -P)/$(basename "$funnel_bin")
export FUNNEL_WORK_DIR="$temp_dir/funnel"
export DOCKER_SOCKET="$docker_socket"
export PROTES_PORT="$port"
started=false

cleanup() {
  result=$?
  trap - EXIT INT TERM
  if [[ "$started" == true ]]; then
    if [[ "$result" -ne 0 ]]; then
      echo 'Logs of proTES, its worker and Funnel:' >&2
      docker compose logs --no-color --tail 80 protes protes-worker funnel >&2 || true
    fi
    docker compose down --volumes --remove-orphans >/dev/null 2>&1 || true
    # Funnel runs as root and leaves root-owned task directories
    if ! docker run --rm --entrypoint chown --volume "$temp_dir:$temp_dir" \
      "$DOCKER_CLI_IMAGE" -R "$(id -u):$(id -g)" "$temp_dir" >/dev/null; then
      echo "Could not restore ownership of $temp_dir for cleanup." >&2
      result=1
    fi
  fi
  if ! rm -rf "$temp_dir"; then
    echo "Could not remove temporary data at $temp_dir." >&2
    result=1
  fi
  exit "$result"
}
trap cleanup EXIT INT TERM

curl --fail --location --silent --show-error --retry 3 --max-time 60 \
  --output "$FAKE_TES_FILE" \
  "https://raw.githubusercontent.com/elixir-cloud-aai/proTES/$protes_commit/tests/smoke/fake_tes.py"
echo "$fake_tes_sha256  $FAKE_TES_FILE" | sha256sum --check --quiet

started=true
docker compose up --detach --wait --wait-timeout "$startup_timeout"

# Access token of a user of proTES (group `smoke-users`)
PROTES_ACCESS_TOKEN=$(docker compose exec -T fake-tes python -c \
  "import json, urllib.request; print(json.load(urllib.request.urlopen(
  'http://localhost:8080/oidc/token?sub=alice'))['access_token'])")
export PROTES_ACCESS_TOKEN
export PROTES_TES_URL="http://127.0.0.1:$port/ga4gh/tes/v1"
# The URL of the backend in tests/protes/config.yaml
export PROTES_BACKEND_URL=http://funnel:8000
echo "proTES ready at $PROTES_TES_URL"

cd "$repo_root"
RUST_BACKTRACE=1 cargo test --locked -p ga4gh-lib --features protes_integration_tests \
  --test protes_tes -- --nocapture
