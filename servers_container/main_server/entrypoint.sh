#!/bin/bash
set -e

# Forward SIGINT (Ctrl+C) and SIGTERM to the Rust process
cleanup() {
    if [ -n "$RUST_PID" ]; then
        kill -TERM "$RUST_PID" 2>/dev/null || true
        wait "$RUST_PID" 2>/dev/null || true
    fi
    exit 0
}

trap cleanup SIGINT SIGTERM

/usr/local/bin/main_server &
RUST_PID=$!

wait "$RUST_PID"
