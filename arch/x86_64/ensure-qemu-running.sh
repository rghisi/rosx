#!/bin/bash
# This script ensures QEMU is running, or starts it if not

# Check if QEMU is already listening on port 1234
if lsof -i:1234 >/dev/null 2>&1; then
    echo "QEMU already running on port 1234"
    exit 0
fi

echo "QEMU not running, starting it now..."
./start-qemu-background.sh