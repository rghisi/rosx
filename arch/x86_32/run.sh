#!/bin/bash
set -e
KERNEL_PATH="$(realpath "$1")"

exec qemu-system-i386 \
    -kernel "$KERNEL_PATH" \
    -debugcon stdio \
    -no-reboot \
    -no-shutdown \
    -d cpu_reset
