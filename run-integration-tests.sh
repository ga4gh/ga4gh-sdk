#!/usr/bin/env bash
set -euo pipefail

health_url=http://localhost:8000/healthz
funnel_startup_timeout_seconds=${FUNNEL_STARTUP_TIMEOUT_SECONDS:-60}
if ! [[ $funnel_startup_timeout_seconds =~ ^[0-9]+$ ]]; then
    echo 'FUNNEL_STARTUP_TIMEOUT_SECONDS must be a non-negative integer' >&2
    exit 2
fi

if curl --fail --silent --max-time 2 "$health_url" > /dev/null; then
    echo 'Funnel is already healthy.'
else
    echo 'Funnel is not healthy. Starting it now...'
    export PATH="$PATH:$HOME/go/bin"
    funnel server run --Server.HostName=localhost --Server.HTTPPort=8000 > funnel.log 2>&1 &
fi

echo 'Waiting for Funnel server to become healthy...'
funnel_startup_deadline=$((SECONDS + funnel_startup_timeout_seconds))
until curl --fail --silent --max-time 2 "$health_url" > /dev/null; do
    if (( SECONDS >= funnel_startup_deadline )); then
        echo "Funnel did not become healthy within ${funnel_startup_timeout_seconds}s" >&2
        if [[ -f funnel.log ]]; then
            tail -n 80 funnel.log >&2
        fi
        exit 1
    fi
    sleep 1
done
echo 'Funnel is healthy.'

RUST_BACKTRACE=1 RUST_LOG=debug cargo test --locked -p ga4gh-lib --features integration_tests
