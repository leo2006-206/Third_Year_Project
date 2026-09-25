#!/usr/bin/env bash
set -e

echo "=== 1. Checking Rust (servers_rust) ==="
cd servers_rust
echo "-> cargo fmt..."
cargo fmt --check
echo "-> cargo clippy..."
cargo clippy --all-targets --all-features -- -D warnings
echo "-> cargo test..."
cargo test
cd ..

echo "=== 2. Checking Python ==="
PYTHON_FILES="servers_container/run_all_offload.py servers_rust/src/testing_webpage/generate_show_options.py"
if command -v ruff &> /dev/null; then
    echo "-> ruff check..."
    ruff check --no-cache $PYTHON_FILES
    echo "-> ruff format check..."
    ruff format --no-cache --check $PYTHON_FILES
else
    echo "-> ruff not installed locally, checking syntax with py_compile..."
    python3 -m py_compile $PYTHON_FILES
fi

echo "=== 3. Checking Markdown ==="
if command -v mdformat &> /dev/null; then
    echo "-> mdformat check..."
    mdformat --check README.md docs/
fi

echo "All CI checks passed locally!"