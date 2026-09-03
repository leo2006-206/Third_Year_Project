#!/bin/bash

# Trap SIGINT and SIGTERM to stop both services gracefully
cleanup() {
    echo ""
    echo "Stopping servers..."
    kill -TERM "$DANA_PID" 2>/dev/null || true
    kill -TERM "$RUST_PID" 2>/dev/null || true
    wait "$DANA_PID" 2>/dev/null || true
    wait "$RUST_PID" 2>/dev/null || true
    echo "Servers stopped."
    exit 0
}
trap cleanup SIGINT SIGTERM

echo "=== [1/2] Starting Dana Web Server on port 8000 ==="
cd /app/obm
dana ws.core -p 8000 &
DANA_PID=$!

echo "=== [2/2] Starting Rust Main Server on port 7000 ==="
/usr/local/bin/main_server &
RUST_PID=$!

# Wait for either process to exit
wait -n "$RUST_PID" "$DANA_PID"
cleanup

