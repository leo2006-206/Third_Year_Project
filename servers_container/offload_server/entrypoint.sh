#!/bin/bash
set -e

ID="${1:-1}"
PORT="${2:-7010}"
DEVICE="${3:-cpu}"

# Forward SIGINT (Ctrl+C) and SIGTERM to both child processes
cleanup() {
    echo ""
    echo "Shutting down offload server $ID (port $PORT, device $DEVICE)..."
    if [ -n "$RUST_PID" ]; then
        kill -TERM "$RUST_PID" 2>/dev/null || true
    fi
    if [ -n "$DANA_PID" ]; then
        kill -TERM "$DANA_PID" 2>/dev/null || true
    fi
    wait 2>/dev/null || true
    exit 0
}

trap cleanup SIGINT SIGTERM

# Ensure ASSET_HOST in OffloadSite.dn matches the container's configured port
if ! grep -q "http://localhost:${PORT}/" /app/obm/OffloadSite.dn 2>/dev/null; then
    echo "Configuring ASSET_HOST for port $PORT..."
    sed -i "s|http://localhost:[0-9]*/|http://localhost:${PORT}/|g" /app/obm/OffloadSite.dn
    (cd /app/obm && dnc OffloadSite.dn)
fi

# 1. Start Dana Offload Site on internal port 9009
echo "=== Starting Dana Offload Site on internal port 9009 (Device: $DEVICE) ==="
cd /app/obm

if [ "$DEVICE" = "gpu" ]; then
    echo "Using VA-API Hardware Video Acceleration (Encoder.h264va / Decoder.h264va)"
    dana -lc "media.video.Encoder:h264|media/video/Encoder.h264va.o|media.video.Encoder:h264va" \
         -lc "media.video.Decoder:h264|media/video/Decoder.h264va.o|media.video.Decoder:h264va" \
         OffloadSite &
else
    echo "Using Software Video Codecs (libx264)"
    dana OffloadSite &
fi
DANA_PID=$!

# 2. Start Rust Offload Server on port $PORT
echo "=== Starting Rust Offload Server $ID on port $PORT ==="
/usr/local/bin/offload_server "$PORT" &
RUST_PID=$!

# Wait for both background processes
wait "$RUST_PID" "$DANA_PID"
