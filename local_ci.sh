#!/usr/bin/env bash
set -e

echo "=== 1. Checking Rust (Workspace) ==="
echo "-> cargo fmt..."
cargo fmt --check --all
echo "-> cargo clippy..."
cargo clippy --workspace --all-targets --all-features -- -D warnings
echo "-> cargo test..."
cargo test --workspace

echo "=== 2. Checking Python ==="

if command -v ruff &> /dev/null; then
    echo "-> ruff check..."
    ruff check --no-cache .
    echo "-> ruff format check..."
    ruff format --no-cache --check .
fi

echo "All CI checks passed locally!"