#!/bin/bash
set -e

KERNEL_PATH="${1:-target/m68k-unknown-none-elf/release/rosx-m68k}"

exec qemu-system-m68k \
    -M virt \
    -m 128M \
    -kernel "$KERNEL_PATH" \
    -serial stdio \
    -display none \
    -no-reboot
