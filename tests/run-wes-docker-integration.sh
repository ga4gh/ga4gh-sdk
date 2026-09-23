#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=ga4gh/ga4gh-starter-kit-wes:0.2.0-nextflow
docker_socket=${DOCKER_SOCKET:-}
startup_timeout=${WES_STARTUP_TIMEOUT_SECONDS:-180}

if ! command -v docker >/dev/null || ! docker info >/dev/null 2>&1; then
  echo 'Docker is required and its daemon must be running.' >&2
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

# The shared directory must have the same absolute path on the host and in
# the container because Nextflow can pass that path to the host Docker daemon.
temp_dir=$(mktemp -d /tmp/ga4gh-sdk-wes.XXXXXXXX)
temp_dir=$(cd "$temp_dir" && pwd -P)
work_dir="$temp_dir/work"
fixture_repo="$temp_dir/fixture-repo"
container_name="ga4gh-sdk-wes-$$"
container_id=

cleanup() {
  result=$?
  trap - EXIT INT TERM
  if [[ -n "$container_id" ]]; then
    if [[ "$result" -ne 0 ]]; then
      echo 'Starter Kit WES container logs:' >&2
      docker logs --tail 120 "$container_id" >&2 || true
    fi
    docker rm -f "$container_id" >/dev/null 2>&1 || true
    # WES and Nextflow run as root in this image. On Linux they leave
    # root-owned files in the bind mount, so restore the host user's ownership
    # from a short-lived container before deleting the temporary directory.
    if ! docker run --rm --platform linux/amd64 --user 0:0 \
      --entrypoint /bin/chown -v "$temp_dir:$temp_dir" \
      "$image" -R "$(id -u):$(id -g)" "$temp_dir" >/dev/null; then
      echo "Could not restore ownership of $temp_dir for cleanup." >&2
      result=1
    fi
  fi
  if ! rm -rf "$temp_dir"; then
    echo "Could not remove temporary WES data at $temp_dir." >&2
    result=1
  fi
  exit "$result"
}
trap cleanup EXIT INT TERM

mkdir -p "$temp_dir/config" "$temp_dir/db" "$work_dir" "$fixture_repo"
chmod 755 "$temp_dir"
chmod 777 "$temp_dir/db" "$work_dir"
cat > "$temp_dir/config/config.yml" <<'YAML'
wes:
  serverProps:
    publicApiPort: 7500
    adminApiPort: 7501
  serviceInfo:
    id: org.ga4gh.sdk.wes.integration
    name: GA4GH SDK WES integration test
  databaseProps:
    url: jdbc:sqlite:/db/wes.sqlite
YAML

# WES 0.2.0 derives a Nextflow project name and revision from a GitHub-shaped
# URL. It does not process workflow_attachment. A cached local Git project with
# matching origin metadata lets the old Nextflow engine run our fixture offline.
cp "$repo_root/tests/fixtures/wes-nextflow/main.nf" "$fixture_repo/main.nf"
git -C "$fixture_repo" init -q
git -C "$fixture_repo" add main.nf
GIT_AUTHOR_DATE='2020-01-01T00:00:00Z' GIT_COMMITTER_DATE='2020-01-01T00:00:00Z' \
  git -C "$fixture_repo" -c user.name='GA4GH SDK integration' \
  -c user.email='sdk-integration@example.invalid' -c commit.gpgsign=false \
  commit -q -m 'WES Nextflow integration fixture'
git -C "$fixture_repo" remote add origin https://github.com/ga4gh-sdk/wes-fixture.git
export WES_DOCKER_FIXTURE_REVISION
WES_DOCKER_FIXTURE_REVISION=$(git -C "$fixture_repo" rev-parse HEAD)

# Starter Kit 0.2.0 does not create its SQLite tables automatically. This is
# its release's database/sqlite/create-tables.sql schema.
python3 - "$temp_dir/db/wes.sqlite" <<'PY'
import sqlite3
import sys

with sqlite3.connect(sys.argv[1]) as db:
    db.execute("""CREATE TABLE wes_run (
        id TEXT PRIMARY KEY,
        workflow_type TEXT NOT NULL,
        workflow_type_version TEXT,
        workflow_url TEXT NOT NULL,
        workflow_params TEXT NOT NULL,
        workflow_engine TEXT NOT NULL,
        workflow_engine_version TEXT
    )""")
PY
chmod 666 "$temp_dir/db/wes.sqlite"

container_id=$(docker run -d --name "$container_name" --platform linux/amd64 \
  -p 127.0.0.1::7500 -p 127.0.0.1::7501 \
  -v "$temp_dir/config:/config:ro" \
  -v "$fixture_repo:/root/.nextflow/assets/ga4gh-sdk/wes-fixture" \
  -v "$temp_dir/db:/db" \
  -v "$docker_socket:/var/run/docker.sock" \
  -v "$work_dir:$work_dir" \
  --workdir "$work_dir" \
  "$image" -c /config/config.yml)

host_port=$(docker port "$container_id" 7500/tcp | head -n 1 | awk -F: '{print $NF}')
if [[ -z "$host_port" ]]; then
  echo 'Docker did not publish the WES public API port.' >&2
  exit 1
fi
export WES_DOCKER_BASE_URL="http://127.0.0.1:$host_port/ga4gh/wes/v1/"
export WES_DOCKER_WORK_DIR="$work_dir"

deadline=$(( $(date +%s) + startup_timeout ))
while ! curl --silent --show-error --fail --max-time 2 \
  "${WES_DOCKER_BASE_URL}service-info" -o /dev/null 2>/dev/null; do
  if [[ $(docker inspect --format '{{.State.Running}}' "$container_id") != true ]]; then
    echo 'Starter Kit WES exited before service-info became ready.' >&2
    exit 1
  fi
  if (( $(date +%s) >= deadline )); then
    echo "Timed out after ${startup_timeout}s waiting for ${WES_DOCKER_BASE_URL}service-info" >&2
    exit 1
  fi
  sleep 1
done

echo "Starter Kit WES ready at $WES_DOCKER_BASE_URL"
cd "$repo_root"
cargo test -p ga4gh-lib --features wes_docker_integration_tests \
  --test wes_starter_kit_docker -- --nocapture
