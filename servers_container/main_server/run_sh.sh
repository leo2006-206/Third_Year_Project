#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Main Server Build & Interactive Run Script
# (Runs the Rust Main Server on Port 7000)
# -----------------------------------------------------------------------------

# Find the repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== [1/3] Compiling Rust Server with Optimizations (--release) ==="
cd "$PROJECT_ROOT/servers_rust"
cargo build --release --bin main_server

echo "=== [2/3] Building Docker Image: obm-main-server ==="
cd "$PROJECT_ROOT"
docker build -f servers_container/main_server/dockerfile -t obm-main-server .

echo "=== [3/3] Starting OBM Main Server Container Interactively ==="
echo "Rust Main Server: Port 7000 (http://localhost:7000/)"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run -it --rm \
    --init \
    -p 7000:7000 \
    --name obm-main-server \
    obm-main-server