#!/usr/bin/env bash
# Named local Toil 9.3.0 WES check of an actual prepared CWL attachment bundle.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
env_dir="${GA4GH_TOIL_ENV_DIR:-$root/target/toil-bundle-venv}"
if [[ ! -x "$env_dir/bin/python" ]]; then
    python3 -m venv "$env_dir"
fi
"$env_dir/bin/python" -m pip install --disable-pip-version-check 'toil[server,cwl,aws]==9.3.0'
work_dir="$(mktemp -d "${TMPDIR:-/tmp}/ga4gh-toil-bundle.XXXXXX")"
server_pid=''
cleanup() {
    status=$?
    if [[ -n "$server_pid" ]]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
    if [[ "$status" -ne 0 ]]; then
        tail -100 "$work_dir/server.log" 2>/dev/null || true
    fi
    rm -rf "$work_dir"
    exit "$status"
}
trap cleanup EXIT
port="$("$env_dir/bin/python" - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0))
    print(sock.getsockname()[1])
PY
)"
PATH="$env_dir/bin:$PATH" AWS_EC2_METADATA_DISABLED=true \
    "$env_dir/bin/toil" server --host 127.0.0.1 --port "$port" --workers 1 \
    --bypass_celery --work_dir "$work_dir/workflows" --opt=--no-container \
    > "$work_dir/server.log" 2>&1 &
server_pid=$!
url="http://127.0.0.1:$port/ga4gh/wes/v1"
ready=false
for _ in {1..240}; do
    if curl --fail --silent "$url/service-info" > /dev/null; then
        ready=true
        break
    fi
    kill -0 "$server_pid" 2>/dev/null || break
    sleep 0.25
done
if [[ "$ready" != true ]]; then
    echo 'Toil WES did not become ready within 60 seconds' >&2
    exit 1
fi
GA4GH_TOIL_WES_URL="$url" \
GA4GH_BUNDLE_EVIDENCE="$root/target/toil-bundle-evidence.json" \
    cargo test -p ga4gh-lib --test verified_bundle \
    real_toil_executes_verified_nested_cwl_bundle --locked -- --ignored --nocapture
