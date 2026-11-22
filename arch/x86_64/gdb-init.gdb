# GDB initialization for RosX kernel debugging

# Set architecture
set architecture i386:x86-64

# Connect to QEMU GDB server
target remote localhost:1234

# Load kernel symbols from the debug ELF (linked at 0xFFFF800000100000; the
# RosX chain loads it there, so symbol addresses match the live addresses)
symbol-file ../../target/rosx/debug/rosx

# Disable pagination for cleaner output
set pagination off

# Enable pretty printing
set print pretty on

# Display useful info
echo \nConnected to QEMU GDB server on localhost:1234\n
echo Kernel symbols loaded\n
echo \n
echo Ready to debug! Use 'c' to continue, 'si' to step instruction, 'b' to set breakpoints\n
echo \n