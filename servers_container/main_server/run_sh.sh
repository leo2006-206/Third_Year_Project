#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Main Server Build & Interactive Run Script
# (Runs the Rust Main Server on Port 7000)
# -----------------------------------------------------------------------------

# Find the repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Check if build should be skipped (e.g. pre-built by run_all.py)
if [ "$1" = "--skip-build" ] || [ "${SKIP_BUILD:-0}" = "1" ]; then
    echo "=== Skipping build step (already pre-built) ==="
else
    echo "=== [1/3] Compiling Rust Server with Optimizations (--release) ==="
    cd "$PROJECT_ROOT/servers_rust"
    cargo build --release --bin main_server

    echo "=== [2/3] Building Docker Image: obm-main-server ==="
    cd "$PROJECT_ROOT"
    docker build -f servers_container/main_server/dockerfile -t obm-main-server .
fi

# Ensure shared docker network exists
docker network create obm-net 2>/dev/null || true

# Remove any existing container with the same name
docker rm -f obm-main-server 2>/dev/null || true

echo "=== [3/3] Starting OBM Main Server Container Interactively ==="
echo "Rust Main Server: Port 7000 (http://localhost:7000/)"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run -it --rm \
    --init \
    --network obm-net \
    -p 7000:7000 \
    -v "$PROJECT_ROOT/obm/assets:/app/obm/assets:ro" \
    --name obm-main-server \
    obm-main-server

