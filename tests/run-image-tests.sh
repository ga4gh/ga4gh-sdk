#!/usr/bin/env bash
# Builds the ga4gh-cli image and checks it (see tests/README.md):
#
# 1. The build context leaves out secret files and build output
#    (.dockerignore).
# 2. The image runs `ga4gh-cli` as its entrypoint, as a non-root user, prints
#    its help and the workspace version, and contains no sources.
# 3. Against a local Funnel TES, the Kubernetes site check script
#    (examples/kubernetes/tes-site-check.sh) runs in the image and exits 0 for
#    a task that completes, and 1 for a task that fails or runs too long (and
#    is canceled).
#
# Environment:
#   IMAGE         image tag to build and check (default: ga4gh-cli:test)
#   PLATFORM      build platform, such as linux/arm64 (default: the host's)
#   FUNNEL_CHECK  true, false or auto (default: auto, which runs step 3 if
#                 the funnel executable is on PATH)
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=${IMAGE:-ga4gh-cli:test}
context_image=ga4gh-cli-context:test
platform=${PLATFORM:-}
funnel_check=${FUNNEL_CHECK:-auto}
funnel_url=http://localhost:8000

if ! command -v docker >/dev/null || ! docker info >/dev/null 2>&1; then
    echo 'Docker is required and its daemon must be running.' >&2
    exit 1
fi
case $funnel_check in
    auto)
        if command -v funnel >/dev/null; then funnel_check=true; else funnel_check=false; fi
        ;;
    true | false) ;;
    *)
        echo "FUNNEL_CHECK must be true, false or auto, not '$funnel_check'" >&2
        exit 2
        ;;
esac

temp_dir=$(mktemp -d)
funnel_pid=
created=()
cleanup() {
    result=$?
    trap - EXIT INT TERM
    if [[ -n "$funnel_pid" ]]; then
        if [[ "$result" -ne 0 ]]; then
            tail -n 80 "$temp_dir/funnel.log" >&2 || true
        fi
        kill "$funnel_pid" 2>/dev/null || true
    fi
    for path in "${created[@]}"; do
        rm -rf "${repo_root:?}/$path"
    done
    docker image rm "$context_image" >/dev/null 2>&1 || true
    rm -rf "$temp_dir"
    exit "$result"
}
trap cleanup EXIT INT TERM

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

platform_args=()
if [[ -n "$platform" ]]; then
    platform_args=(--platform "$platform")
fi

echo '--- Build context'
# Probe files that .dockerignore must leave out, and one it must keep
excluded=(.env.image-test lib/.env.image-test image-test.pem lib/image-test.key
    id_ed25519_image_test target/image-test lib/target/image-test
    .ga4gh-cli/config.json)
included=(.env.example)
for path in "${excluded[@]}" "${included[@]}"; do
    if [[ -e "$repo_root/$path" ]]; then
        fail "$path exists already; move it away before running this test"
    fi
done
for path in "${excluded[@]}" "${included[@]}"; do
    # Remove the first new directory of the path again, or else the file
    top=$path
    while [[ "$top" == */* && ! -e "$repo_root/$(dirname "$top")" ]]; do
        top=$(dirname "$top")
    done
    created+=("$top")
    mkdir -p "$(dirname "$repo_root/$path")"
    echo 'image test probe, not a secret' > "$repo_root/$path"
done
runtime_image=$(sed -n 's/^ARG RUNTIME_IMAGE=//p' "$repo_root/Dockerfile")
[[ -n "$runtime_image" ]] || fail 'RUNTIME_IMAGE not found in the Dockerfile'
# A Dockerfile from stdin uses the .dockerignore of the context
docker buildx build "${platform_args[@]}" --load --quiet --tag "$context_image" \
    --build-arg "BASE=$runtime_image" --file - "$repo_root" > /dev/null <<'EOF'
ARG BASE
FROM ${BASE}
COPY . /context
EOF
listing=$(docker run --rm "${platform_args[@]}" --entrypoint find "$context_image" \
    /context -mindepth 1 -printf '%P\n')
for path in "${excluded[@]}"; do
    if grep -qxF -- "$path" <<< "$listing"; then
        fail "$path is in the build context"
    fi
done
for path in "${included[@]}"; do
    grep -qxF -- "$path" <<< "$listing" || fail "$path is missing from the build context"
done
for path in Cargo.lock Cargo.toml rust-toolchain.toml cli/src/main.rs lib/Cargo.toml; do
    grep -qxF -- "$path" <<< "$listing" || fail "$path is missing from the build context"
done
echo 'ok: secret files and build output are left out'

echo '--- Image'
docker buildx build "${platform_args[@]}" --load --tag "$image" "$repo_root"

help=$(docker run --rm "${platform_args[@]}" "$image" --help)
grep -q '^Usage: ga4gh-cli' <<< "$help" || fail "unexpected --help output: $help"
# The default command is --help, too
docker run --rm "${platform_args[@]}" "$image" | grep -q '^Usage: ga4gh-cli' \
    || fail 'the default command does not print the help'

expected_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' \
    "$repo_root/Cargo.toml")
version=$(docker run --rm "${platform_args[@]}" "$image" --version)
[[ "$version" == "ga4gh-cli $expected_version" ]] \
    || fail "--version prints '$version', expected 'ga4gh-cli $expected_version'"

entrypoint=$(docker image inspect --format '{{json .Config.Entrypoint}}' "$image")
[[ "$entrypoint" == '["ga4gh-cli"]' ]] || fail "entrypoint is $entrypoint"
user=$(docker image inspect --format '{{.Config.User}}' "$image")
[[ "$user" == 10001:10001 ]] || fail "image user is '$user'"
uid=$(docker run --rm "${platform_args[@]}" --entrypoint id "$image" -u)
[[ "$uid" == 10001 ]] || fail "runs as UID $uid"
docker run --rm "${platform_args[@]}" --entrypoint sh "$image" -c \
    'test ! -e /src && test "$HOME" = /home/ga4gh && test "$(pwd)" = /work' \
    || fail 'the image contains /src, or HOME or the working directory differ'
echo "ok: $version, entrypoint ga4gh-cli, UID 10001, no sources"

if [[ "$funnel_check" != true ]]; then
    echo '--- Funnel site check skipped (FUNNEL_CHECK=false or no funnel on PATH)'
    exit 0
fi

echo '--- Funnel site check'
if curl --fail --silent --max-time 2 "$funnel_url/healthz" > /dev/null; then
    echo 'Funnel is already running; using it.'
else
    (cd "$temp_dir" && exec funnel server run --Server.HostName=localhost \
        --Server.HTTPPort=8000 > "$temp_dir/funnel.log" 2>&1) &
    funnel_pid=$!
    deadline=$((SECONDS + ${FUNNEL_STARTUP_TIMEOUT_SECONDS:-60}))
    until curl --fail --silent --max-time 2 "$funnel_url/healthz" > /dev/null; do
        ((SECONDS < deadline)) || fail 'Funnel did not become healthy'
        sleep 1
    done
fi

# The CLI in the container reads the config as UID 10001
mkdir -p "$temp_dir/config" "$temp_dir/check"
printf '{"TES": {"base_path": "%s/ga4gh/tes/v1"}}\n' "$funnel_url" \
    > "$temp_dir/config/config.json"
cp "$repo_root/examples/kubernetes/tes-site-check.sh" \
    "$repo_root/examples/kubernetes/task.json" "$temp_dir/check/"
sed 's/echo ga4gh-cli site check/exit 3/' "$repo_root/examples/kubernetes/task.json" \
    > "$temp_dir/check/failing-task.json"
sed 's/echo ga4gh-cli site check/sleep 120/' "$repo_root/examples/kubernetes/task.json" \
    > "$temp_dir/check/slow-task.json"
chmod 755 "$temp_dir/config" "$temp_dir/check"
chmod 644 "$temp_dir/config/config.json" "$temp_dir/check/"*

site_check() {
    docker run --rm "${platform_args[@]}" --network host --read-only \
        --cap-drop ALL --security-opt no-new-privileges \
        --volume "$temp_dir/config:/home/ga4gh/.ga4gh-cli:ro" \
        --volume "$temp_dir/check:/etc/ga4gh-site-check:ro" \
        --env TASK_FILE="/etc/ga4gh-site-check/$1" --env POLL_SECONDS=1 \
        --env TIMEOUT_SECONDS="${2:-300}" \
        --entrypoint sh "$image" /etc/ga4gh-site-check/tes-site-check.sh
}

if ! output=$(site_check task.json); then
    printf '%s\n' "$output" >&2
    fail 'the site check failed for a task that should complete'
fi
printf '%s\n' "$output"
grep -q '^task_id=.' <<< "$output" || fail 'no task ID on stdout'
grep -qx 'result=COMPLETE' <<< "$output" || fail 'no result=COMPLETE on stdout'
grep -q '"state": *"COMPLETE"' <<< "$output" || fail 'no FULL view on stdout'
echo 'ok: completed task, exit 0'

set +e
output=$(site_check failing-task.json)
status=$?
set -e
printf '%s\n' "$output"
[[ "$status" -eq 1 ]] || fail "the site check exited with $status for a failing task, expected 1"
grep -qx 'result=EXECUTOR_ERROR' <<< "$output" || fail 'no result=EXECUTOR_ERROR on stdout'
echo 'ok: failing task, exit 1'

set +e
output=$(site_check slow-task.json 2)
status=$?
set -e
printf '%s\n' "$output"
[[ "$status" -eq 1 ]] || fail "the site check exited with $status for a slow task, expected 1"
grep -qx 'result=TIMEOUT' <<< "$output" || fail 'no result=TIMEOUT on stdout'
grep -q '"state": *"CANCEL' <<< "$output" || fail 'the slow task was not canceled'
echo 'ok: slow task canceled, exit 1'
