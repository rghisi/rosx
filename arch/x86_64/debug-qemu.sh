#!/bin/bash

set -e

BOOT_DISK="../../target/rosx-boot-disk.img"

if [ ! -f "$BOOT_DISK" ]; then
    echo "no unified boot disk at $BOOT_DISK; run 'cargo xtask build x86_64 --debug' first" >&2
    exit 1
fi

echo "Launching QEMU from the unified boot disk with GDB server..."
echo "GDB server listening on localhost:1234"
echo "Waiting for debugger to attach..."
echo ""
echo "To debug:"
echo "1. In RustRover, use 'Remote Debug' configuration pointing to localhost:1234"
echo "2. Or use GDB: gdb -x gdb-init.gdb"
echo ""
echo "The machine halts at reset: the RosX chain (stage1, stage2-64) runs first,"
echo "then the kernel lands in the +2 TiB window; 'b _start' hits that handoff."
echo ""

qemu-system-x86_64 \
    -drive "if=none,id=rosxboot,format=raw,file=$BOOT_DISK" \
    -device ide-hd,drive=rosxboot,bus=ide.0,unit=0,bootindex=1 \
    -debugcon stdio \
    -s \
    -S \
    -no-reboot \
    -no-shutdown
