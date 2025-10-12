#!/bin/bash

set -e

#echo "Building RosX with debug symbols and creating bootable image..."
#cargo bootimage
#
KERNEL_BINARY="../../target/rosx/debug/bootimage-rosx.bin"
#
#if [ ! -f "$KERNEL_BINARY" ]; then
#    echo "Error: Bootimage not found at $KERNEL_BINARY"
#    exit 1
#fi

echo "Launching QEMU with GDB server..."
echo "GDB server listening on localhost:1234"
echo "Waiting for debugger to attach..."
echo ""
echo "To debug:"
echo "1. In RustRover, use 'Remote Debug' configuration pointing to localhost:1234"
echo "2. Or use GDB: gdb -x gdb-init.gdb"
echo ""

qemu-system-x86_64 \
    -drive format=raw,file=$KERNEL_BINARY \
    -s \
    -S \
    -no-reboot \
    -no-shutdown