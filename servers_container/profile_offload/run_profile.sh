#!/bin/bash
set -e

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$DIR/../.." && pwd)"

cd "$PROJECT_ROOT"

echo "================================================================================"
echo " Building Profiled Offload Server Docker Image..."
echo "================================================================================"
docker build -f servers_container/profile_offload/dockerfile -t obm-offload-profile .

# Clean up previous profiling containers if any
docker rm -f obm-offload-prof-gpu 2>/dev/null || true
docker rm -f obm-offload-prof-cpu 2>/dev/null || true

echo ""
echo "================================================================================"
echo " 1. Running GPU Profile Container on Port 7030..."
echo "================================================================================"
docker run -d \
    --name obm-offload-prof-gpu \
    --network obm-net \
    --device /dev/dri:/dev/dri \
    -p 7030:7030 \
    obm-offload-profile 3 7030 gpu

echo "Waiting for GPU container to initialize (5s)..."
sleep 5

TEST_URL="http://localhost:7030/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam"

echo "Sending curl request to GPU offload container..."
time curl -s "$TEST_URL" -o /tmp/seg_profile_gpu.h264

echo ""
echo "--- GPU CONTAINER LOGS ---"
docker logs obm-offload-prof-gpu | grep -A 25 "DANA OFFLOAD PROFILING BREAKDOWN" || docker logs --tail 40 obm-offload-prof-gpu

docker rm -f obm-offload-prof-gpu 2>/dev/null || true

echo ""
echo "================================================================================"
echo " 2. Running CPU Profile Container on Port 7040..."
echo "================================================================================"
docker run -d \
    --name obm-offload-prof-cpu \
    --network obm-net \
    -p 7040:7040 \
    obm-offload-profile 4 7040 cpu

echo "Waiting for CPU container to initialize (5s)..."
sleep 5

TEST_URL_CPU="http://localhost:7040/offload/show/f1_full.json/0/10/1280/720/race/landscape/4/race|race/driver|Sam/track|track/drivers|Sam"

echo "Sending curl request to CPU offload container..."
time curl -s "$TEST_URL_CPU" -o /tmp/seg_profile_cpu.h264

echo ""
echo "--- CPU CONTAINER LOGS ---"
docker logs obm-offload-prof-cpu | grep -A 25 "DANA OFFLOAD PROFILING BREAKDOWN" || docker logs --tail 40 obm-offload-prof-cpu

docker rm -f obm-offload-prof-cpu 2>/dev/null || true

echo ""
echo "================================================================================"
echo " Profiling Complete!"
echo "================================================================================"
