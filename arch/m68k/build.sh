#!/bin/bash
set -e

LIBGCC_PATH=$(m68k-linux-gnu-gcc -print-libgcc-file-name)
RUSTFLAGS="-C linker=m68k-linux-gnu-ld -C link-arg=-Tarch/m68k/linker.ld -C link-arg=--undefined=kernel_main -C link-arg=$LIBGCC_PATH" \
cargo build --package rosx-m68k --target m68k-unknown-none-elf -Z build-std=core,alloc,compiler_builtins --release
