#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Offload Server Build & Interactive Run Script
# Usage: ./run_sh.sh [ID] [PORT]
# Example: ./run_sh.sh 1 7010
# -----------------------------------------------------------------------------

ID="${1:-1}"
PORT="${2:-7010}"

CONTAINER_NAME="obm-offload-${ID}"
IMAGE_NAME="obm-offload-server"

# Find the repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Check if build should be skipped (e.g. pre-built by run_all.py)
if [ "$3" = "--skip-build" ] || [ "${SKIP_BUILD:-0}" = "1" ]; then
    echo "=== Skipping build step (already pre-built) ==="
else
    echo "=== [1/3] Compiling Rust Offload Server with Optimizations (--release) ==="
    cd "$PROJECT_ROOT/servers_rust"
    cargo build --release --bin offload_server

    echo "=== [2/3] Building Docker Image: $IMAGE_NAME ==="
    cd "$PROJECT_ROOT"
    docker build \
        --build-arg PORT="$PORT" \
        -f servers_container/offload_server/dockerfile \
        -t "$IMAGE_NAME" .
fi

# Ensure shared docker network exists
docker network create obm-net 2>/dev/null || true

# Remove any existing container with the same name
docker rm -f "$CONTAINER_NAME" 2>/dev/null || true

echo "=== [3/3] Starting Offload Server $ID ($CONTAINER_NAME) on Port $PORT ==="
echo "Container: $CONTAINER_NAME (Port $PORT)"
echo "Dana Offload Engine: Internal Port 9009"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run -it --rm \
    --init \
    --network obm-net \
    -p "${PORT}:${PORT}" \
    --name "$CONTAINER_NAME" \
    "$IMAGE_NAME" \
    "$ID" "$PORT"
