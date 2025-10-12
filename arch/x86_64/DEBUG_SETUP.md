# RosX QEMU Debugging Setup

This directory contains scripts and configurations for debugging RosX with QEMU and GDB integration in RustRover.

## Quick Start

### Method 1: Using RustRover (Recommended - Fully Automated)

1. **In RustRover:**
   - Select run configuration: **"Debug RosX in QEMU"**
   - Click the Debug button (or press Shift+F9)
   - QEMU will automatically start in the background
   - Debugger will attach once QEMU is ready
   - Set breakpoints in your code
   - Use debug controls to step through code
   - Inspect variables, registers, and stack in the debugger panel

**Note:** The "Debug RosX in QEMU" configuration has a "Before Launch" task that automatically:
- Kills any existing QEMU instance
- Builds your kernel
- Starts QEMU with GDB server
- Waits for the server to be ready
- Then attaches the debugger

### Method 1b: Manual QEMU Launch (Alternative)

If you prefer to control QEMU manually:

1. **Start QEMU in debug mode:**
   ```bash
   cd arch/x86_64
   ./debug-qemu.sh
   ```

2. **In RustRover:**
   - Use the **"Debug RosX in QEMU"** configuration
   - Or create a copy without the "Before Launch" task

### Method 2: Using GDB directly

1. **Start QEMU in debug mode:**
   ```bash
   cd arch/x86_64
   ./debug-qemu.sh
   ```

2. **In another terminal, launch GDB:**
   ```bash
   cd arch/x86_64
   gdb -x gdb-init.gdb
   ```

3. **Common GDB commands:**
   - `c` or `continue` - Continue execution
   - `si` - Step one instruction
   - `b *0x<address>` - Set breakpoint at address
   - `b function_name` - Set breakpoint at function
   - `info registers` - Show all CPU registers
   - `x/10i $rip` - Show next 10 instructions
   - `bt` - Show backtrace/stack
   - `print variable` - Print variable value

## Files

- **debug-qemu.sh** - Script to build and launch QEMU with GDB server (manual use)
- **start-qemu-background.sh** - Wrapper script for RustRover "Before Launch" task
- **gdb-init.gdb** - GDB initialization script
- **DEBUG_SETUP.md** - This file

## RustRover Configurations

Three run configurations are available:

1. **Debug RosX in QEMU** - Main debug configuration (fully automated)
   - Automatically starts QEMU
   - Builds kernel with debug symbols
   - Attaches debugger when ready

2. **Start QEMU Debug Server** - Background task (used by "Debug RosX in QEMU")
   - Kills any existing QEMU instance
   - Builds and starts QEMU with GDB server
   - Waits for server to be ready

3. **Build RosX Debug** - Builds the kernel with debug symbols only

## Debugging Tips

### Setting Breakpoints

In RustRover, you can set breakpoints by clicking in the gutter next to line numbers.

In GDB:
```gdb
# Break at kernel entry
b _start

# Break at a function
b main_thread::run

# Break at specific address
b *0x100000
```

### Inspecting CPU State

In GDB:
```gdb
# Show all registers
info registers

# Show specific register
print/x $rip
print/x $rsp

# Show stack
x/20xg $rsp
```

### Viewing Assembly

In GDB:
```gdb
# Disassemble current location
disassemble

# Show next instructions
x/10i $rip

# Enable mixed source/assembly view
set disassemble-next-line on
```

### Memory Inspection

```gdb
# Examine memory at address
x/10xg 0x100000

# Formats:
# x = hex, d = decimal, s = string
# b = byte, h = halfword, w = word, g = giant (8 bytes)
```

## Troubleshooting

### QEMU won't start
- Check that bootimage built successfully: `ls -l target/rosx/debug/bootimage-rosx_arch_x86_64.bin`
- Try building manually: `cargo build && bootimage build`

### Debugger won't connect
- Ensure QEMU is running with `./debug-qemu.sh`
- Check that port 1234 is not in use: `netstat -an | grep 1234`
- Verify GDB is connecting to correct port: should be `localhost:1234`

### No symbols loaded
- Ensure debug build was created: `cargo build` (not `cargo build --release`)
- Check symbol file exists: `ls -l target/rosx/debug/rosx_arch_x86_64`
- In GDB, manually load symbols: `symbol-file target/rosx/debug/rosx_arch_x86_64`

### Breakpoints not hitting
- Verify code is actually executing (use `si` to step through)
- Check that symbols match binary: `info sources` in GDB
- For early kernel code, you may need to set breakpoints by address

## Advanced: Debugging from Bootloader

If you need to debug the bootloader itself, you'll need to:
1. Load bootloader symbols separately
2. Set architecture to i386 initially (16-bit real mode)
3. Switch to i386:x86-64 after entering 64-bit mode

This is more complex and usually not necessary for kernel development.