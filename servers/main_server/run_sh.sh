#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Main Server Build & Interactive Run Script
# -----------------------------------------------------------------------------

# Find the repository root (parent of servers/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== [1/4] Checking Dana Compiler and Pre-compiling Web Component ==="
cd "$PROJECT_ROOT/obm"

# Compile ws/Web.dn if dnc is available
if command -v dnc &> /dev/null; then
    echo "Compiling ws/Web.dn with host dnc..."
    dnc ws/Web.dn
elif [ -n "$DANA_HOME" ] && [ -f "$DANA_HOME/dnc" ]; then
    echo "Compiling ws/Web.dn using $DANA_HOME/dnc..."
    "$DANA_HOME/dnc" ws/Web.dn
else
    echo "Note: dnc not found on PATH. Using existing pre-compiled Web.o if present."
fi

echo "=== [2/4] Ensuring Dana Runtime is Staged ==="
RUNTIME_STAGE="$PROJECT_ROOT/servers/dana_runtime_copy"

if [ ! -f "$RUNTIME_STAGE/dana" ] || [ ! -d "$RUNTIME_STAGE/components" ]; then
    echo "Staging Dana runtime using $RUNTIME_STAGE/setup.sh..."
    "$RUNTIME_STAGE/setup.sh"
else
    echo "Dana runtime already staged in $RUNTIME_STAGE."
fi

echo "=== [3/4] Building Docker Image: obm-main-server ==="
cd "$PROJECT_ROOT"
docker build -f servers/main_server/dockerfile -t obm-main-server .

echo "=== [4/4] Starting OBM Main Server Container Interactively ==="
echo "Port: 7000 (http://localhost:7000/)"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run -it --rm \
    -p 7000:7000 \
    --name obm-main-server \
    obm-main-server

