#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Offload-1 Server Build & Interactive Run Script
# (Runs Rust Offload Server on Port 7010 alongside Dana Offload Site)
# -----------------------------------------------------------------------------

LOCAL_URL="https://obm_offload_1.leowong.space/"
PORT="7010"

# Find the repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== [1/3] Compiling Rust Offload Server with Optimizations (--release) ==="
cd "$PROJECT_ROOT/servers_rust"
cargo build --release --bin offload_server

echo "=== [2/3] Building Docker Image: obm-offload-1 ==="
cd "$PROJECT_ROOT"
docker build \
    --build-arg PORT="$PORT" \
    -f servers_container/offload_1/dockerfile \
    -t obm-offload-1 .

echo "=== [3/3] Starting OBM Offload-1 Container Interactively ==="
echo "Rust Offload Server: Port $PORT ($LOCAL_URL)"
echo "Dana Offload Engine: Internal Port 9009"
echo "Press Ctrl+C to stop both servers."
echo "-------------------------------------------------------------"

docker run -it --rm \
    --init \
    -p ${PORT}:${PORT} \
    --name obm-offload-1 \
    obm-offload-1 \
    "$LOCAL_URL" "$PORT"
