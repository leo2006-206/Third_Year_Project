#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# Original OBM Main Server Build & Interactive Run Script
# (Runs the original Dana Web Server directly on Port 7000)
# -----------------------------------------------------------------------------

# Find the repository root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== [1/4] Checking Dana Compiler and Pre-compiling Web Component ==="
cd "$PROJECT_ROOT/obm"

# Only recompile ws/Web.dn if it does not exist or has been modified after Web.o
if [ ! -f "ws/Web.o" ] || [ "ws/Web.dn" -nt "ws/Web.o" ]; then
    if command -v dnc &> /dev/null; then
        echo "Compiling ws/Web.dn with host dnc..."
        dnc ws/Web.dn
    elif [ -n "$DANA_HOME" ] && [ -f "$DANA_HOME/dnc" ]; then
        echo "Compiling ws/Web.dn using $DANA_HOME/dnc..."
        "$DANA_HOME/dnc" ws/Web.dn
    else
        echo "Note: dnc not found on PATH. Using existing pre-compiled Web.o if present."
    fi
else
    echo "ws/Web.o is up to date, skipping recompilation."
fi

echo "=== [2/4] Ensuring Dana Runtime is Staged ==="
RUNTIME_STAGE="$PROJECT_ROOT/servers_container/dana_runtime_copy"

if [ ! -f "$RUNTIME_STAGE/dana" ] || [ ! -d "$RUNTIME_STAGE/components" ]; then
    echo "Staging Dana runtime using $RUNTIME_STAGE/setup.sh..."
    "$RUNTIME_STAGE/setup.sh"
else
    echo "Dana runtime already staged in $RUNTIME_STAGE."
fi

echo "=== [3/4] Building Docker Image: original-obm-main ==="
cd "$PROJECT_ROOT"
docker build -f servers_container/original_obm_main/dockerfile -t original-obm-main .

echo "=== [4/4] Starting Original OBM Main Server Container Interactively ==="
echo "Dana Web Server: Port 7000 (http://localhost:7000/)"
echo "Press Ctrl+C to stop the server."
echo "-------------------------------------------------------------"

docker run -it --rm \
    --init \
    -p 7000:7000 \
    --name original-obm-main \
    original-obm-main

