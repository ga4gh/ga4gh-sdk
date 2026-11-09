# Check if a "funnel" process is already running
if ! ps aux | grep '[f]unnel server run'; then
    # If it's not running, start it
    echo "Funnel server is not running. Starting it now..."
    export PATH=$PATH:~/go/bin
    funnel server run --Server.HostName=localhost --Server.HTTPPort=8000 > funnel.log 2>&1 &
else
    echo "Funnel server is already running."
fi

# Wait for the Funnel server to start
echo "Waiting for Funnel server to start..."
funnel_startup_timeout_seconds=${FUNNEL_STARTUP_TIMEOUT_SECONDS:-60}
if ! [[ $funnel_startup_timeout_seconds =~ ^[0-9]+$ ]]; then
    echo "FUNNEL_STARTUP_TIMEOUT_SECONDS must be a non-negative integer" >&2
    exit 2
fi
funnel_startup_deadline=$((SECONDS + funnel_startup_timeout_seconds))
while ! curl --fail --silent --max-time 2 http://localhost:8000/healthz > /dev/null
do
    if (( SECONDS >= funnel_startup_deadline )); then
        echo "Funnel did not become healthy within ${funnel_startup_timeout_seconds}s" >&2
        exit 1
    fi
    echo "Waiting for Funnel server..."
    sleep 1
done
echo "Funnel server is running."

# Run the tests
RUST_BACKTRACE=1 RUST_LOG=debug cargo test -p ga4gh-lib --features integration_tests

