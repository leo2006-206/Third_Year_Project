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

if command -v ruff &> /dev/null; then
    echo "-> ruff check..."
    ruff check --no-cache .
    echo "-> ruff format check..."
    ruff format --no-cache --check .
fi

echo "All CI checks passed locally!"