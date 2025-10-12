#!/bin/bash

# Kill any existing QEMU instance on port 1234
lsof -ti:1234 | xargs kill -9 2>/dev/null || true

# Start QEMU in background
./debug-qemu.sh &
QEMU_PID=$!

echo "QEMU starting (PID: $QEMU_PID)..."
echo "Waiting for GDB server to be ready..."

# Wait for QEMU to start listening on port 1234
for i in {1..30}; do
    if lsof -i:1234 >/dev/null 2>&1; then
        echo "QEMU GDB server is ready on port 1234"
        exit 0
    fi
    sleep 0.5
done

echo "ERROR: QEMU failed to start within 15 seconds"
kill $QEMU_PID 2>/dev/null
exit 1