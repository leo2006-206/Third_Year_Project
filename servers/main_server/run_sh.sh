#!/usr/bin/env bash
set -e

# -----------------------------------------------------------------------------
# OBM Main Server Build & Interactive Run Script
# (Stage 2: Rust Server on Port 7000 + Dana Web Server on Port 8000)
# -----------------------------------------------------------------------------

# Find the repository root (parent of servers/)
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

echo "=== [1/5] Checking Dana Compiler and Pre-compiling Web Component ==="
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

echo "=== [2/5] Ensuring Dana Runtime is Staged ==="
RUNTIME_STAGE="$PROJECT_ROOT/servers/dana_runtime_copy"

if [ ! -f "$RUNTIME_STAGE/dana" ] || [ ! -d "$RUNTIME_STAGE/components" ]; then
    echo "Staging Dana runtime using $RUNTIME_STAGE/setup.sh..."
    "$RUNTIME_STAGE/setup.sh"
else
    echo "Dana runtime already staged in $RUNTIME_STAGE."
fi

echo "=== [3/5] Compiling Rust Server with Optimizations (--release) ==="
cd "$PROJECT_ROOT/obm_servers"
cargo build --release --bin main_server

echo "=== [4/5] Building Docker Image: obm-main-server ==="
cd "$PROJECT_ROOT"
docker build -f servers/main_server/dockerfile -t obm-main-server .

echo "=== [5/5] Starting OBM Main Server Container Interactively ==="
echo "Rust Main Server: Port 7000 (http://localhost:7000/)"
echo "Dana Web Server:  Port 8000 (http://localhost:8000/)"
echo "Press Ctrl+C to stop the servers."
echo "-------------------------------------------------------------"

docker run -it --rm \
    -p 7000:7000 \
    -p 8000:8000 \
    --name obm-main-server \
    obm-main-server