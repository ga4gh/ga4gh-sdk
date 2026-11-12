#!/usr/bin/env bash
# Builds the ga4gh-cli image and checks it (see tests/README.md):
#
# 1. The build context holds the tracked sources of the workspace and leaves
#    out probe files that look like secrets or build output (.dockerignore),
#    and the builder's Rust version matches rust-toolchain.toml.
# 2. The image runs `ga4gh-cli` as its entrypoint, as a non-root user, prints
#    its help and the workspace version, and contains no sources.
# 3. Against a local Funnel TES, the Kubernetes site check script
#    (examples/kubernetes/tes-site-check.sh) runs in the image and exits 0 for
#    a task that completes, and 1 for a task that fails, a task that runs too
#    long (and is canceled), an unreachable TES and an invalid setting.
#
# Needs bash 4.4 or later, Docker with Buildx and, for step 3, curl and the
# funnel executable.
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

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

if ((BASH_VERSINFO[0] < 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] < 4))); then
    echo "This script needs bash 4.4 or later, not $BASH_VERSION" >&2
    exit 2
fi
if ! command -v docker >/dev/null || ! docker info >/dev/null 2>&1; then
    echo 'Docker is required and its daemon must be running.' >&2
    exit 1
fi
case $funnel_check in
    auto)
        if command -v funnel >/dev/null; then funnel_check=true; else funnel_check=false; fi
        ;;
    true)
        command -v funnel >/dev/null || fail 'FUNNEL_CHECK=true, but funnel is not on PATH'
        command -v curl >/dev/null || fail 'FUNNEL_CHECK=true, but curl is not on PATH'
        ;;
    false) ;;
    *)
        echo "FUNNEL_CHECK must be true, false or auto, not '$funnel_check'" >&2
        exit 2
        ;;
esac

# Removes only the probe files, then the directories created for them if they
# are empty again, deepest first
remove_probes() {
    for path in "${probe_files[@]}"; do
        rm -f "${repo_root:?}/$path"
    done
    for ((i = ${#probe_dirs[@]} - 1; i >= 0; i--)); do
        rmdir "${repo_root:?}/${probe_dirs[i]}" 2>/dev/null || true
    done
    probe_files=()
    probe_dirs=()
}

temp_dir=$(mktemp -d)
funnel_pid=
probe_files=()
probe_dirs=()
cleanup() {
    result=$?
    trap - EXIT INT TERM
    if [[ -n "$funnel_pid" ]]; then
        if [[ "$result" -ne 0 ]]; then
            tail -n 80 "$temp_dir/funnel.log" >&2 || true
        fi
        kill "$funnel_pid" 2>/dev/null || true
    fi
    remove_probes
    docker image rm "$context_image" >/dev/null 2>&1 || true
    rm -rf "$temp_dir"
    exit "$result"
}
trap cleanup EXIT INT TERM

platform_args=()
if [[ -n "$platform" ]]; then
    platform_args=(--platform "$platform")
fi

echo '--- Build context'
channel=$(sed -n 's/^channel = "\(.*\)"$/\1/p' "$repo_root/rust-toolchain.toml")
grep -q "^FROM docker.io/library/rust:$channel-" "$repo_root/Dockerfile" \
    || fail "the Dockerfile's Rust image doesn't match rust-toolchain.toml ($channel)"

# Probe files that .dockerignore must leave out, and one it must keep
excluded=(.env.image-test lib/.env.image-test image-test.pem lib/image-test.key
    id_ed25519_image_test cli/id_rsa_image_test .ssh/image-test .kube/config
    lib/kubeconfig-image-test .aws/credentials .cargo/credentials.toml
    .git-credentials .netrc lib/credentials-image-test.json .ga4gh-cli/config.json
    target/image-test lib/target/image-test cli/image-test.log
    docs/image-test.md examples/image-test.json)
included=(lib/.env.example)
for path in "${excluded[@]}" "${included[@]}"; do
    if [[ -e "$repo_root/$path" ]]; then
        fail "$path exists already; move it away before running this test"
    fi
done
for path in "${excluded[@]}" "${included[@]}"; do
    dirs=()
    dir=$(dirname "$path")
    while [[ "$dir" != . && ! -e "$repo_root/$dir" ]]; do
        dirs=("$dir" "${dirs[@]}")
        dir=$(dirname "$dir")
    done
    probe_dirs+=("${dirs[@]}")
    mkdir -p "$(dirname "$repo_root/$path")"
    probe_files+=("$path")
    echo 'image test probe, not a secret' > "$repo_root/$path"
done
runtime_image=$(sed -n 's/^FROM \(docker.io\/library\/debian:[^ ]*\)$/\1/p' "$repo_root/Dockerfile")
[[ -n "$runtime_image" ]] || fail 'the Debian runtime image is not found in the Dockerfile'
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
for path in .git tests docs examples; do
    if grep -qxF -- "$path" <<< "$listing"; then
        fail "$path is in the build context"
    fi
done
# Every tracked file that the build may need
while IFS= read -r path; do
    grep -qxF -- "$path" <<< "$listing" || fail "$path is missing from the build context"
done < <(git -C "$repo_root" ls-files -- Cargo.toml Cargo.lock rust-toolchain.toml \
    README.md LICENSE lib cli python; printf '%s\n' "${included[@]}")
# Before the image build, so that its context (and layer cache) is the
# same as that of a later build from the checkout
remove_probes
echo 'ok: the context holds the workspace sources, without probe files'

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
    echo "A service already answers at $funnel_url/healthz; using it as Funnel."
else
    (cd "$temp_dir" && exec funnel server run --Server.HostName=localhost \
        --Server.HTTPPort=8000 > "$temp_dir/funnel.log" 2>&1) &
    funnel_pid=$!
    deadline=$((SECONDS + ${FUNNEL_STARTUP_TIMEOUT_SECONDS:-60}))
    until curl --fail --silent --max-time 2 "$funnel_url/healthz" > /dev/null; do
        kill -0 "$funnel_pid" 2>/dev/null || fail 'Funnel stopped'
        ((SECONDS < deadline)) || fail 'Funnel did not become healthy'
        sleep 1
    done
fi

# The CLI in the container reads these files as UID 10001
mkdir -p "$temp_dir/config" "$temp_dir/unreachable" "$temp_dir/check"
printf '{"TES": {"base_path": "%s/ga4gh/tes/v1"}}\n' "$funnel_url" \
    > "$temp_dir/config/config.json"
# Nothing listens on port 9 (discard) of the loopback interface
printf '{"TES": {"base_path": "http://127.0.0.1:9/ga4gh/tes/v1"}}\n' \
    > "$temp_dir/unreachable/config.json"
task="$repo_root/examples/kubernetes/task.json"
cp "$repo_root/examples/kubernetes/tes-site-check.sh" "$task" "$temp_dir/check/"
variant() {
    sed "s/echo ga4gh-cli site check/$1/" "$task" > "$temp_dir/check/$2"
    ! cmp -s "$task" "$temp_dir/check/$2" || fail "could not derive $2 from task.json"
}
variant 'exit 3' failing-task.json
variant 'sleep 120' slow-task.json
chmod 755 "$temp_dir/config" "$temp_dir/unreachable" "$temp_dir/check"
chmod 644 "$temp_dir/config/config.json" "$temp_dir/unreachable/config.json" \
    "$temp_dir/check/"*

# Runs the site check with the given task, time limit and configuration
# directory; sets output and status
site_check() {
    set +e
    output=$(docker run --rm "${platform_args[@]}" --network host --read-only \
        --cap-drop ALL --security-opt no-new-privileges \
        --volume "$temp_dir/${3:-config}:/home/ga4gh/.ga4gh-cli:ro" \
        --volume "$temp_dir/check:/etc/ga4gh-site-check:ro" \
        --env TASK_FILE="/etc/ga4gh-site-check/$1" --env POLL_SECONDS=1 \
        --env TIMEOUT_SECONDS="${2:-300}" \
        --entrypoint sh "$image" /etc/ga4gh-site-check/tes-site-check.sh)
    status=$?
    set -e
    printf '%s\n' "$output"
}

site_check task.json
[[ "$status" -eq 0 ]] || fail "the site check exited with $status for a task that should complete"
grep -q '^task_id=.' <<< "$output" || fail 'no task ID on stdout'
grep -qx 'result=COMPLETE' <<< "$output" || fail 'no result=COMPLETE on stdout'
grep -q '"state": *"COMPLETE"' <<< "$output" || fail 'no FULL view on stdout'
echo 'ok: completed task, exit 0'

site_check failing-task.json
[[ "$status" -eq 1 ]] || fail "the site check exited with $status for a failing task, expected 1"
grep -qx 'result=EXECUTOR_ERROR' <<< "$output" || fail 'no result=EXECUTOR_ERROR on stdout'
echo 'ok: failing task, exit 1'

site_check slow-task.json 2
[[ "$status" -eq 1 ]] || fail "the site check exited with $status for a slow task, expected 1"
grep -qx 'result=TIMEOUT' <<< "$output" || fail 'no result=TIMEOUT on stdout'
grep -q '"state": *"CANCEL' <<< "$output" || fail 'the slow task was not canceled'
echo 'ok: slow task canceled, exit 1'

site_check task.json 300 unreachable
[[ "$status" -eq 1 ]] || fail "the site check exited with $status for an unreachable TES, expected 1"
if grep -q '^task_id=' <<< "$output"; then
    fail 'a task ID was printed for an unreachable TES'
fi
echo 'ok: unreachable TES, exit 1'

site_check task.json soon
[[ "$status" -eq 1 ]] || fail "the site check exited with $status for an invalid time limit, expected 1"
if grep -q '^task_id=' <<< "$output"; then
    fail 'a task was submitted despite an invalid time limit'
fi
echo 'ok: invalid time limit, exit 1 before submitting'
