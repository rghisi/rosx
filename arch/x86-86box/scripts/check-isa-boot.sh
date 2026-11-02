#!/bin/sh
# Mnemonic audit of the GRUB Legacy stage binaries against the 486
# baseline, so the bootloader contract is guarded exactly like the kernel.
# The audit itself lives in xtask (one mnemonic table for the whole port);
# this script is the standalone entry point for local runs.
#
# usage: check-isa-boot.sh [stages-dir]
#        (default: <repo>/target/grub-legacy/stages)
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
stages=${1:-$repo/target/grub-legacy/stages}

cd "$repo"
exec cargo xtask check-isa x86_32 --stages "$stages"
