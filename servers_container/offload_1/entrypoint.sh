#!/bin/bash
set -e

LOCAL_URL="${1:-https://obm_offload_1.leowong.space/}"
PORT="${2:-7010}"

# Forward SIGINT (Ctrl+C) and SIGTERM to both child processes
cleanup() {
    echo ""
    echo "Shutting down offload servers..."
    if [ -n "$RUST_PID" ]; then
        kill -TERM "$RUST_PID" 2>/dev/null || true
    fi
    if [ -n "$DANA_PID" ]; then
        kill -TERM "$DANA_PID" 2>/dev/null || true
    fi
    wait 2>/dev/null || true
    exit 0
}

trap cleanup SIGINT SIGTERM

# 1. Start Dana Offload Site on internal port 9009
echo "=== Starting Dana Offload Site on internal port 9009 ==="
cd /app/obm
dana OffloadSite &
DANA_PID=$!

# 2. Start Rust Offload Server on port $PORT
echo "=== Starting Rust Offload Server on port $PORT (URL: $LOCAL_URL) ==="
/usr/local/bin/offload_server "$LOCAL_URL" "$PORT" &
RUST_PID=$!

# Wait for both background processes
wait "$RUST_PID" "$DANA_PID"
