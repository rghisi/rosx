#!/bin/bash
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
KERNEL_PATH="$(realpath "$1")"
RUNNER_DIR="$(realpath "$SCRIPT_DIR/../../runner")"

cd "$RUNNER_DIR"
exec cargo run -- "$KERNEL_PATH"
