#!/bin/bash
set -e

ID="${1:-1}"
PORT="${2:-7010}"
DEVICE="${3:-cpu}"

# Forward SIGINT (Ctrl+C) and SIGTERM to all child processes
cleanup() {
    echo ""
    echo "Shutting down offload server $ID (port $PORT, device $DEVICE)..."
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

# Configure rendering drivers and OpenGL settings based on acceleration tier:
#   Tier 'cpu'      : Pure CPU software rasterization (SDL software blitter)
#   Tier 'llvmpipe' : CPU Software OpenGL rasterizer (Mesa llvmpipe)
#   Tier 'gpu'      : Physical GPU Direct Rendering (VirtualGL + Mesa DRI + VA-API decoder)
export SDL_VIDEODRIVER=x11

GPU_CARD=""

if [ "$DEVICE" = "gpu" ]; then
    echo "=== Tier 3: Physical GPU Direct Rendering (VirtualGL + Mesa DRI + VA-API Decoder) ==="
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

    # Pre-flight check: Verify OpenGL hardware acceleration via VirtualGL
    GL_RENDERER=$(vglrun -d "$GPU_CARD" glxinfo -B 2>&1 | grep "OpenGL renderer string:" | cut -d: -f2 | xargs)
    echo "Probing VirtualGL OpenGL on $GPU_CARD: '$GL_RENDERER'"

    if echo "$GL_RENDERER" | grep -qi "llvmpipe\|software"; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] Tier 'gpu' requested, but renderer fell back to software!" >&2
        echo "  Observed Renderer : '$GL_RENDERER'" >&2
        echo "  Target DRM Card   : $GPU_CARD" >&2
        echo "Aborting startup. Container will NOT fall back to software rendering." >&2
        echo "==========================================================================" >&2
        exit 1
    fi

    if [ -z "$GL_RENDERER" ]; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] Tier 'gpu' requested, but failed to initialize OpenGL on $GPU_CARD!" >&2
        echo "Aborting startup." >&2
        echo "==========================================================================" >&2
        exit 1
    fi

    # Pre-flight check: Verify VA-API hardware video decoder
    RENDER_DEV=""
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

    if ! $VAINFO_CMD 2>&1 | grep -q "VAEntrypointVLD"; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] Tier 'gpu' requested, but VA-API Hardware Decoder not available!" >&2
        echo "Aborting startup." >&2
        echo "==========================================================================" >&2
        exit 1
    fi

    echo "=== [DEVICE VERIFIED] Hardware GPU Active: '$GL_RENDERER' on $GPU_CARD ==="
    echo "=== [DEVICE VERIFIED] VA-API Hardware Video Decoder Active on ${RENDER_DEV:-drm} ==="

    export SDL_RENDER_DRIVER=opengl
    export LIBGL_ALWAYS_INDIRECT=0
    unset LIBGL_ALWAYS_SOFTWARE

elif [ "$DEVICE" = "llvmpipe" ]; then
    echo "=== Tier 2: CPU Software OpenGL Rasterizer (Mesa llvmpipe) ==="
    export SDL_RENDER_DRIVER=opengl
    export LIBGL_ALWAYS_SOFTWARE=1

    GL_RENDERER=$(glxinfo -B 2>&1 | grep "OpenGL renderer string:" | cut -d: -f2 | xargs)
    echo "Probing Mesa OpenGL Renderer: '$GL_RENDERER'"

    if ! echo "$GL_RENDERER" | grep -qi "llvmpipe"; then
        echo "==========================================================================" >&2
        echo "[FATAL ERROR] Tier 'llvmpipe' requested, but renderer is not llvmpipe: '$GL_RENDERER'" >&2
        echo "Aborting startup." >&2
        echo "==========================================================================" >&2
        exit 1
    fi
    echo "=== [DEVICE VERIFIED] Mesa llvmpipe Software OpenGL Active ==="

elif [ "$DEVICE" = "cpu" ]; then
    echo "=== Tier 1: Pure CPU Software Pipeline (SDL Software Blitter) ==="
    export SDL_RENDER_DRIVER=software
    export LIBGL_ALWAYS_SOFTWARE=1
    echo "=== [DEVICE VERIFIED] SDL Software Blitter Active ==="

else
    echo "[FATAL ERROR] Unknown device tier '$DEVICE'. Expected 'gpu', 'llvmpipe', or 'cpu'." >&2
    exit 1
fi

# Ensure ASSET_HOST in OffloadSite_new.dn matches the container's configured port
if ! grep -q "http://localhost:${PORT}/" /app/obm/OffloadSite_new.dn 2>/dev/null; then
    echo "Configuring ASSET_HOST for port $PORT..."
    sed -i "s|http://localhost:[0-9]*/|http://localhost:${PORT}/|g" /app/obm/OffloadSite_new.dn
    (cd /app/obm && dnc OffloadSite_new.dn)
fi

# 2. Start Dana Offload Site on internal port 9009
echo "=== Starting Dana Offload Site on internal port 9009 (Tier: $DEVICE, Encoder: libx264) ==="
cd /app/obm

if [ "$DEVICE" = "gpu" ]; then
    echo "Launching Dana with VirtualGL on $GPU_CARD + Hardware Decoder (Decoder.h264va)..."
    vglrun -d "$GPU_CARD" dana -lc "media.video.Decoder:h264|media/video/Decoder.h264va.o|media.video.Decoder:h264va" \
         OffloadSite_new &
else
    echo "Launching Dana with Software Video Codecs (libavcodec decoder + libx264 encoder)..."
    dana OffloadSite_new &
fi
DANA_PID=$!

# 3. Start Rust Offload Server on port $PORT
echo "=== Starting Rust Offload Server $ID on port $PORT ==="
/usr/local/bin/offload_server "$PORT" &
RUST_PID=$!

# Wait for both background processes
wait "$RUST_PID" "$DANA_PID"
