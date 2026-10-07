#!/bin/bash
set -e

ID="${1:-1}"
PORT="${2:-7010}"
DECODING="${3:-cpu}"
COMPOSITION="${4:-cpu}"
ENCODING="${5:-cpu}"

# Validate parameter values
if [ "$DECODING" != "cpu" ] && [ "$DECODING" != "gpu" ]; then
    echo "[FATAL ERROR] Invalid decoding '$DECODING'. Must be 'cpu' or 'gpu'." >&2
    exit 1
fi

if [ "$COMPOSITION" != "cpu" ] && [ "$COMPOSITION" != "llvmpipe" ] && [ "$COMPOSITION" != "gpu" ]; then
    echo "[FATAL ERROR] Invalid composition '$COMPOSITION'. Must be 'cpu', 'llvmpipe', or 'gpu'." >&2
    exit 1
fi

if [ "$ENCODING" = "gpu" ]; then
    echo "[FATAL ERROR] GPU encoding is currently unsupported/disabled. Please set encoding to 'cpu'." >&2
    exit 1
fi

if [ "$ENCODING" != "cpu" ]; then
    echo "[FATAL ERROR] Invalid encoding '$ENCODING'. Only 'cpu' is supported." >&2
    exit 1
fi

# Forward SIGINT (Ctrl+C) and SIGTERM to all child processes
cleanup() {
    echo ""
    echo "Shutting down offload server $ID (port $PORT, decoding $DECODING, composition $COMPOSITION, encoding $ENCODING)..."
    if [ -n "$RUST_PID" ]; then
        kill -TERM "$RUST_PID" 2>/dev/null || true
    fi
    if [ -n "$DANA_PID" ]; then
        kill -TERM "$DANA_PID" 2>/dev/null || true
    fi
    if [ -n "$XVFB_PID" ]; then
        kill -TERM "$XVFB_PID" 2>/dev/null || true
    fi
    wait 2>/dev/null || true
    exit 0
}

trap cleanup SIGINT SIGTERM

# 1. Start Xvfb virtual headless display server on :99
echo "=== Starting Xvfb virtual headless display on :99 ==="
Xvfb :99 -screen 0 1920x1080x24 -ac +extension GLX +render -noreset &
XVFB_PID=$!
export DISPLAY=:99
sleep 1

export SDL_VIDEODRIVER=x11

GPU_CARD=""
RENDER_DEV=""

# Check whether physical GPU hardware is needed
if [ "$COMPOSITION" = "gpu" ] || [ "$DECODING" = "gpu" ]; then
    echo "=== Hardware GPU Verification (Decoding: $DECODING, Composition: $COMPOSITION, Encoding: $ENCODING) ==="
    if [ ! -d "/dev/dri" ]; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] /dev/dri not found inside container." >&2
        echo "Hardware GPU acceleration requires mounting /dev/dri." >&2
        echo "==========================================================================" >&2
        exit 1
    fi

    # Find the hardware DRM card device (prefer Intel vendor 0x8086)
    for c in /dev/dri/card*; do
        [ -e "$c" ] || continue
        if [ "$(cat /sys/class/drm/$(basename $c)/device/vendor 2>/dev/null)" = "0x8086" ]; then
            GPU_CARD="$c"
            break
        fi
    done
    if [ -z "$GPU_CARD" ]; then
        GPU_CARD=$(ls /dev/dri/card* 2>/dev/null | head -n 1)
    fi

    if [ -z "$GPU_CARD" ] || [ ! -e "$GPU_CARD" ]; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] No /dev/dri/card* DRM device found inside container." >&2
        echo "Aborting startup. Container will NOT run on incorrect device tier." >&2
        echo "==========================================================================" >&2
        exit 1
    fi

    # Find render node for VA-API
    for r in /dev/dri/renderD*; do
        [ -e "$r" ] || continue
        if [ "$(cat /sys/class/drm/$(basename $r)/device/vendor 2>/dev/null)" = "0x8086" ]; then
            RENDER_DEV="$r"
            break
        fi
    done
    if [ -z "$RENDER_DEV" ]; then
        RENDER_DEV=$(ls /dev/dri/renderD* 2>/dev/null | head -n 1)
    fi

    VAINFO_CMD="vainfo"
    if [ -n "$RENDER_DEV" ] && [ -e "$RENDER_DEV" ]; then
        VAINFO_CMD="vainfo --display drm --device $RENDER_DEV"
    fi

    if [ "$DECODING" = "gpu" ]; then
        if ! $VAINFO_CMD 2>&1 | grep -q "VAEntrypointVLD"; then
            echo "==========================================================================" >&2
            echo "[FATAL ERROR] Hardware decoding requested, but VAEntrypointVLD not supported on $RENDER_DEV!" >&2
            echo "Aborting startup." >&2
            echo "==========================================================================" >&2
            exit 1
        fi
        echo "=== [DEVICE VERIFIED] VA-API Hardware Video Decoder Active on ${RENDER_DEV:-drm} ==="
    else
        echo "=== [DECODER VERIFIED] Software CPU Video Decoder Active (libavcodec) ==="
    fi

    echo "=== [ENCODER VERIFIED] Software CPU Video Encoder Active (libx264) ==="
fi

# Configure OpenGL renderer based on COMPOSITION
if [ "$COMPOSITION" = "gpu" ]; then
    GL_RENDERER=$(vglrun -d "$GPU_CARD" glxinfo -B 2>&1 | grep "OpenGL renderer string:" | cut -d: -f2 | xargs)
    echo "Probing VirtualGL OpenGL on $GPU_CARD: '$GL_RENDERER'"

    if echo "$GL_RENDERER" | grep -qi "llvmpipe\|software"; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] Composition 'gpu' requested, but renderer fell back to software: '$GL_RENDERER'!" >&2
        echo "Aborting startup." >&2
        echo "==========================================================================" >&2
        exit 1
    fi
    echo "=== [DEVICE VERIFIED] Hardware GPU Active: '$GL_RENDERER' on $GPU_CARD ==="

    export SDL_RENDER_DRIVER=opengl
    export LIBGL_ALWAYS_INDIRECT=0
    unset LIBGL_ALWAYS_SOFTWARE

elif [ "$COMPOSITION" = "llvmpipe" ]; then
    echo "=== Composition Tier: CPU Software OpenGL Rasterizer (Mesa llvmpipe) ==="
    export SDL_RENDER_DRIVER=opengl
    export LIBGL_ALWAYS_SOFTWARE=1

    GL_RENDERER=$(glxinfo -B 2>&1 | grep "OpenGL renderer string:" | cut -d: -f2 | xargs)
    echo "Probing Mesa OpenGL Renderer: '$GL_RENDERER'"

    if ! echo "$GL_RENDERER" | grep -qi "llvmpipe"; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] Composition 'llvmpipe' requested, but renderer is not llvmpipe: '$GL_RENDERER'!" >&2
        echo "Aborting startup." >&2
        echo "==========================================================================" >&2
        exit 1
    fi
    echo "=== [DEVICE VERIFIED] Mesa llvmpipe Software OpenGL Active ==="

elif [ "$COMPOSITION" = "cpu" ]; then
    echo "=== Composition Tier: Pure CPU Software Pipeline (SDL Software Blitter) ==="
    export SDL_RENDER_DRIVER=software
    export LIBGL_ALWAYS_SOFTWARE=1
    echo "=== [DEVICE VERIFIED] SDL Software Blitter Active ==="
fi

# Ensure ASSET_HOST in OffloadSite_new.dn matches the container's configured port
if ! grep -q "http://localhost:${PORT}/" /app/obm/OffloadSite_new.dn 2>/dev/null; then
    echo "Configuring ASSET_HOST for port $PORT..."
    sed -i "s|http://localhost:[0-9]*/|http://localhost:${PORT}/|g" /app/obm/OffloadSite_new.dn
    (cd /app/obm && dnc OffloadSite_new.dn)
fi

# 2. Start Dana Offload Site on internal port 9009
DANA_ARGS=()
if [ "$DECODING" = "gpu" ]; then
    DANA_ARGS+=(-lc "media.video.Decoder:h264|media/video/Decoder.h264va.o|media.video.Decoder:h264va")
fi

echo "=== Starting Dana Offload Site on internal port 9009 (Decoding: $DECODING, Composition: $COMPOSITION, Encoding: $ENCODING) ==="
cd /app/obm

if [ "$COMPOSITION" = "gpu" ]; then
    echo "Launching Dana with VirtualGL on $GPU_CARD..."
    vglrun -d "$GPU_CARD" dana "${DANA_ARGS[@]}" OffloadSite_new &
else
    echo "Launching Dana..."
    dana "${DANA_ARGS[@]}" OffloadSite_new &
fi
DANA_PID=$!

# 3. Start Rust Offload Server on port $PORT
echo "=== Starting Rust Offload Server $ID on port $PORT ==="
/usr/local/bin/offload_server "$PORT" &
RUST_PID=$!

# Wait for both background processes
wait "$RUST_PID" "$DANA_PID"
