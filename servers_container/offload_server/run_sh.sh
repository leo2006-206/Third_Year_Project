#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Offload Server Build & Interactive Run Script
# Usage: ./run_sh.sh [ID] [PORT] [DECODING] [COMPOSITION] [ENCODING] [CPU] [MEMORY] [DELAY_MS] [JITTER_MS] [--skip-build]
#   CPU:       CPU cores the container may use (docker --cpus), -1 = no cap (default)
#   MEMORY:    memory limit in GB (docker --memory, swap disabled), -1 = no cap (default 3.5)
#   DELAY_MS:  netem delay added to all traffic the site sends (incl. asset downloads), 0 = none (default)
#   JITTER_MS: netem jitter (normal distribution) around DELAY_MS, 0 = none (default)
#   DETACH=1 (env): run detached (-d) instead of interactive (-it), e.g. for benchmarks
#   MAIN_SERVER_ADDR (env): main server the site fetches /shows and /assets from
#     (default obm-main-server:7000, same docker network; e.g. 10.10.0.1:7000 from another machine)
# Example: ./run_sh.sh 1 7010 gpu gpu cpu 4 2 50 5
# -----------------------------------------------------------------------------

ID="${1:-1}"
PORT="${2:-7010}"
DECODING="cpu"
COMPOSITION="cpu"
ENCODING="cpu"
CPU_LIMIT="-1"
MEMORY_LIMIT="3.5"
DELAY_MS="0"
JITTER_MS="0"
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
    CPU_LIMIT="${POS_ARGS[3]:-$CPU_LIMIT}"
    MEMORY_LIMIT="${POS_ARGS[4]:-$MEMORY_LIMIT}"
    DELAY_MS="${POS_ARGS[5]:-$DELAY_MS}"
    JITTER_MS="${POS_ARGS[6]:-$JITTER_MS}"
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

# CPU / MEMORY: -1 (no cap) or a positive number
is_limit() { [ "$1" = "-1" ] || [[ "$1" =~ ^[0-9]*\.?[0-9]+$ && "$1" != "0" && ! "$1" =~ ^0*\.?0*$ ]]; }
if ! is_limit "$CPU_LIMIT"; then
    echo "[FATAL ERROR] Invalid CPU '$CPU_LIMIT'. Must be -1 (no cap) or a positive number." >&2
    exit 1
fi
if ! is_limit "$MEMORY_LIMIT"; then
    echo "[FATAL ERROR] Invalid MEMORY '$MEMORY_LIMIT'. Must be -1 (no cap) or a positive number of GB." >&2
    exit 1
fi

# DELAY_MS / JITTER_MS: non-negative integers (ms)
if ! [[ "$DELAY_MS" =~ ^[0-9]+$ ]] || ! [[ "$JITTER_MS" =~ ^[0-9]+$ ]]; then
    echo "[FATAL ERROR] Invalid DELAY_MS '$DELAY_MS' / JITTER_MS '$JITTER_MS'. Must be non-negative integers (ms)." >&2
    exit 1
fi
if [ "$DELAY_MS" = "0" ] && [ "$JITTER_MS" != "0" ]; then
    echo "[FATAL ERROR] JITTER_MS requires DELAY_MS > 0." >&2
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
    cd "$PROJECT_ROOT"
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
        # Dana's VA-API decoder opens the default (first) render node, so on multi-GPU hosts
        # (e.g. Intel iGPU + NVIDIA) expose only the Intel GPU, as card0/renderD128 in the container.
        INTEL_CARD=""
        INTEL_RENDER=""
        for d in /sys/class/drm/card[0-9]* /sys/class/drm/renderD[0-9]*; do
            n=$(basename "$d")
            [[ "$n" =~ ^(card[0-9]+|renderD[0-9]+)$ ]] || continue
            [ "$(cat "$d/device/vendor" 2>/dev/null)" = "0x8086" ] || continue
            if [[ "$n" == card* ]] && [ -z "$INTEL_CARD" ]; then INTEL_CARD="/dev/dri/$n"; fi
            if [[ "$n" == renderD* ]] && [ -z "$INTEL_RENDER" ]; then INTEL_RENDER="/dev/dri/$n"; fi
        done
        if [ -n "$INTEL_CARD" ] && [ -n "$INTEL_RENDER" ]; then
            echo "Hardware GPU acceleration enabled: mounting Intel GPU $INTEL_CARD + $INTEL_RENDER as card0 + renderD128."
            DOCKER_GPU_ARGS=(--device "$INTEL_CARD:/dev/dri/card0" --device "$INTEL_RENDER:/dev/dri/renderD128")
        else
            echo "Hardware GPU acceleration enabled: no Intel GPU found, mounting all of /dev/dri into container."
            DOCKER_GPU_ARGS=(--device /dev/dri:/dev/dri)
        fi
    else
        echo "ERROR: /dev/dri not found on host. Hardware GPU acceleration requires a physical GPU (/dev/dri)." >&2
        echo "Aborting startup for $CONTAINER_NAME." >&2
        exit 1
    fi
fi

# Resource caps
DOCKER_RES_ARGS=()
if [ "$CPU_LIMIT" != "-1" ]; then
    DOCKER_RES_ARGS+=(--cpus="$CPU_LIMIT")
    # llvmpipe spawns one raster thread per host core by default; match it to the CPU cap
    if [ "$COMPOSITION" = "llvmpipe" ]; then
        DOCKER_RES_ARGS+=(-e LP_NUM_THREADS="$(python3 -c "import math; print(max(1, math.ceil($CPU_LIMIT)))")")
    fi
fi
if [ "$MEMORY_LIMIT" != "-1" ]; then
    DOCKER_RES_ARGS+=(--memory="${MEMORY_LIMIT}g" --memory-swap="${MEMORY_LIMIT}g")
fi

# Network emulation is applied by entrypoint.sh (tc netem), which needs NET_ADMIN in the container
DOCKER_NET_ARGS=(-e MAIN_SERVER_ADDR="${MAIN_SERVER_ADDR:-obm-main-server:7000}")
if [ "$DELAY_MS" != "0" ]; then
    DOCKER_NET_ARGS+=(--cap-add NET_ADMIN -e NETEM_DELAY_MS="$DELAY_MS" -e NETEM_JITTER_MS="$JITTER_MS")
fi

DOCKER_MODE_ARGS=(-it)
if [ "${DETACH:-0}" = "1" ]; then
    DOCKER_MODE_ARGS=(-d)
fi

echo "=== [3/3] Starting Offload Server $ID ($CONTAINER_NAME) on Port $PORT (Decoding: $DECODING, Composition: $COMPOSITION, Encoding: $ENCODING, CPU: $CPU_LIMIT, Memory: ${MEMORY_LIMIT}GB, Delay: ${DELAY_MS}ms ±${JITTER_MS}ms) ==="
echo "Container: $CONTAINER_NAME (Port $PORT)"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run "${DOCKER_MODE_ARGS[@]}" --rm \
    --init \
    --network obm-net \
    "${DOCKER_RES_ARGS[@]}" \
    "${DOCKER_NET_ARGS[@]}" \
    "${DOCKER_GPU_ARGS[@]}" \
    -p "${PORT}:${PORT}" \
    --name "$CONTAINER_NAME" \
    "$IMAGE_NAME" \
    "$ID" "$PORT" "$DECODING" "$COMPOSITION" "$ENCODING"
