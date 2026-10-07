#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Offload Server Build & Interactive Run Script
# Usage: ./run_sh.sh [ID] [PORT] [DECODING] [COMPOSITION] [ENCODING] [--skip-build]
# Example: ./run_sh.sh 1 7010 gpu gpu cpu
# -----------------------------------------------------------------------------

ID="${1:-1}"
PORT="${2:-7010}"
DECODING="cpu"
COMPOSITION="cpu"
ENCODING="cpu"
SKIP_BUILD_FLAG=0

# Parse remaining arguments
POS_ARGS=()
for arg in "${@:3}"; do
    if [ "$arg" = "--skip-build" ]; then
        SKIP_BUILD_FLAG=1
    else
        POS_ARGS+=("$arg")
    fi
done

if [ ${#POS_ARGS[@]} -ge 3 ]; then
    DECODING="${POS_ARGS[0]}"
    COMPOSITION="${POS_ARGS[1]}"
    ENCODING="${POS_ARGS[2]}"
elif [ ${#POS_ARGS[@]} -eq 1 ]; then
    # Compatibility fallback if single device tier passed
    COMPOSITION="${POS_ARGS[0]}"
fi

if [ "$ENCODING" = "gpu" ]; then
    echo "[FATAL ERROR] GPU encoding is currently unsupported/disabled. Please set encoding to 'cpu'." >&2
    exit 1
fi

if [ "$ENCODING" != "cpu" ]; then
    echo "[FATAL ERROR] Invalid encoding '$ENCODING'. Only 'cpu' is supported." >&2
    exit 1
fi

CONTAINER_NAME="obm-offload-${ID}"
IMAGE_NAME="obm-offload-server"

# Find the repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Check if build should be skipped (e.g. pre-built by run_all_offload.py)
if [ "$SKIP_BUILD_FLAG" = "1" ] || [ "${SKIP_BUILD:-0}" = "1" ]; then
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

# Configure GPU flags if any stage requires GPU acceleration
DOCKER_GPU_ARGS=()
if [ "$DECODING" = "gpu" ] || [ "$COMPOSITION" = "gpu" ]; then
    if [ -d "/dev/dri" ]; then
        echo "Hardware GPU acceleration enabled: mounting /dev/dri into container."
        DOCKER_GPU_ARGS=(--device /dev/dri:/dev/dri)
    else
        echo "ERROR: /dev/dri not found on host. Hardware GPU acceleration requires a physical GPU (/dev/dri)." >&2
        echo "Aborting startup for $CONTAINER_NAME." >&2
        exit 1
    fi
fi

echo "=== [3/3] Starting Offload Server $ID ($CONTAINER_NAME) on Port $PORT (Decoding: $DECODING, Composition: $COMPOSITION, Encoding: $ENCODING) ==="
echo "Container: $CONTAINER_NAME (Port $PORT)"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run -it --rm \
    --init \
    --network obm-net \
    "${DOCKER_GPU_ARGS[@]}" \
    -p "${PORT}:${PORT}" \
    --name "$CONTAINER_NAME" \
    "$IMAGE_NAME" \
    "$ID" "$PORT" "$DECODING" "$COMPOSITION" "$ENCODING"
