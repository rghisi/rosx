#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL_PATH="$(realpath "$1")"
REPO_ROOT="$(realpath "$SCRIPT_DIR/../..")"

cd "$REPO_ROOT"
exec env -u CARGO_BUILD_TARGET -u CARGO_ENCODED_RUSTFLAGS -u CARGO_TARGET_DIR \
    cargo run --manifest-path "$REPO_ROOT/arch/x86-runner/Cargo.toml" -- "$KERNEL_PATH" "x86_64"
