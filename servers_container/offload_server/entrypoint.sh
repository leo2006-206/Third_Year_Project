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
#   Tier 'gpu'      : Physical GPU Direct Rendering (Mesa DRI + VA-API decoder)
export SDL_VIDEODRIVER=x11

if [ "$DEVICE" = "gpu" ]; then
    echo "=== Tier 3: Physical GPU Direct Rendering (Mesa DRI + VA-API Decoder) ==="
    if [ ! -d "/dev/dri" ]; then
        echo "ERROR: /dev/dri not found inside container. Hardware GPU acceleration requires /dev/dri to be mounted." >&2
        echo "Aborting startup. Pass '--device /dev/dri:/dev/dri' or set device tier to 'llvmpipe' or 'cpu'." >&2
        exit 1
    fi
    export SDL_RENDER_DRIVER=opengl
    export LIBGL_ALWAYS_INDIRECT=0
    unset LIBGL_ALWAYS_SOFTWARE
elif [ "$DEVICE" = "llvmpipe" ]; then
    echo "=== Tier 2: CPU Software OpenGL Rasterizer (Mesa llvmpipe) ==="
    export SDL_RENDER_DRIVER=opengl
    export LIBGL_ALWAYS_SOFTWARE=1
else
    echo "=== Tier 1: Pure CPU Software Pipeline (SDL Software Blitter) ==="
    export SDL_RENDER_DRIVER=software
    export LIBGL_ALWAYS_SOFTWARE=1
fi

# Ensure Dana UIPlaneLib default window size matches 1280x720 for 1:1 OpenGL viewport mapping
if [ -f /opt/dana/components/resources-ext/UIPlaneLib\[deb.x64\].dnl ]; then
    perl -0777 -pi -e 's/\x48\xb9\x80\x02\x00\x00\xe0\x01\x00\x00/\x48\xb9\x00\x05\x00\x00\xd0\x02\x00\x00/g' \
        /opt/dana/components/resources-ext/UIPlaneLib\[deb.x64\].dnl 2>/dev/null || true
fi

# Ensure ASSET_HOST in OffloadSite.dn matches the container's configured port
if ! grep -q "http://localhost:${PORT}/" /app/obm/OffloadSite.dn 2>/dev/null; then
    echo "Configuring ASSET_HOST for port $PORT..."
    sed -i "s|http://localhost:[0-9]*/|http://localhost:${PORT}/|g" /app/obm/OffloadSite.dn
    (cd /app/obm && dnc OffloadSite.dn)
fi

# 2. Start Dana Offload Site on internal port 9009
echo "=== Starting Dana Offload Site on internal port 9009 (Tier: $DEVICE, Encoder: libx264) ==="
cd /app/obm

if [ "$DEVICE" = "gpu" ]; then
    echo "Using VA-API Hardware Decoder (Decoder.h264va) + Software Encoder (libx264)"
    dana -lc "media.video.Decoder:h264|media/video/Decoder.h264va.o|media.video.Decoder:h264va" \
         OffloadSite &
else
    echo "Using Software Video Codecs (libavcodec decoder + libx264 encoder)"
    dana OffloadSite &
fi
DANA_PID=$!

# 3. Start Rust Offload Server on port $PORT
echo "=== Starting Rust Offload Server $ID on port $PORT ==="
/usr/local/bin/offload_server "$PORT" &
RUST_PID=$!

# Wait for both background processes
wait "$RUST_PID" "$DANA_PID"
