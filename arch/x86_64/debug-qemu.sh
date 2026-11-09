#!/bin/bash

set -e

KERNEL_BINARY="../../target/rosx/debug/rosx-x86_64.img"
BOOT_DISK="../../target/rosx-boot-disk.img"

echo "Launching QEMU with GDB server..."
echo "GDB server listening on localhost:1234"
echo "Waiting for debugger to attach..."
echo ""
echo "To debug:"
echo "1. In RustRover, use 'Remote Debug' configuration pointing to localhost:1234"
echo "2. Or use GDB: gdb -x gdb-init.gdb"
echo ""

BOOT_DISK_ARGS=()
if [ -f "$BOOT_DISK" ]; then
    BOOT_DISK_ARGS=(-drive "if=ide,index=0,media=disk,format=raw,file=$BOOT_DISK")
fi

qemu-system-x86_64 \
    "${BOOT_DISK_ARGS[@]}" \
    -drive if=none,id=rosxboot,format=raw,file=$KERNEL_BINARY \
    -device ide-hd,drive=rosxboot,bus=ide.1,unit=0,bootindex=1 \
    -s \
    -S \
    -no-reboot \
    -no-shutdown