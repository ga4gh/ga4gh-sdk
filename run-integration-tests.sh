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
while ! curl -s http://localhost:8000/healthz > /dev/null
do
    echo "Waiting for Funnel server..."
    sleep 1
done
echo "Funnel server is running."

# Run the tests
RUST_BACKTRACE=1 RUST_LOG=debug cargo test --locked --features integration_tests

