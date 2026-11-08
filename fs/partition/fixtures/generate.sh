#!/bin/sh
# Regenerate the standard-tool master boot records parsed by the compatibility
# tests in tests/standard_tools.rs. Each capture is sector 0 of a 100 MiB
# image partitioned exclusively by a real tool, cross-checked against the
# tool's own report (`fdisk -l`, `sfdisk -d`, `parted print`, `gdisk -l`).
#
# Captured with util-linux fdisk/sfdisk 2.41.3, GNU parted, and gdisk. The
# disk identifier (bytes 440..443) is randomized per run, so a regeneration
# reproduces the committed fixtures everywhere except those four bytes; the
# table bytes and the tests do not depend on them.
#
# The dos-fdisk-*.mbr fixtures are NOT script-regenerable: they are sector 0
# of a blank 993/16/63 (1 000 944 sector) disk on a 486DX2-66 (86Box), booted
# from the FreeDOS 1.3 boot floppy (FD13BOOT.img of FD13-LiveCD.zip; the
# FD13LIVE.iso inside carries sha256 aaeeb690301b0403e012ed5a80d813a5472c095f
# e81a07410daa379f44a380e9) and partitioned by the running Free FDISK 1.3.4
# with large-disk support refused. dos-fdisk-max-primary is the maximum-size
# active primary of the Y-Y path; dos-fdisk-primary-extended is a 200 MB
# active primary plus the remaining space in an extended container holding
# three logical drives (its EBR chain lives at the container start, exactly
# what v1 recognizes and refuses to walk). Both captures verified against
# sfdisk --dump, sfdisk --verify and fdisk -l on the host.
#
# usage: generate.sh [output-dir]   (default: the script's own directory)
set -eu

out=${1:-$(cd "$(dirname "$0")" && pwd)}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

for tool in truncate sfdisk fdisk parted gdisk dd; do
    command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 1; }
done

new_image() { truncate -s 100M "$work/$1.img"; }

capture() { dd if="$work/$1.img" of="$out/$1.mbr" bs=512 count=1 status=none; }

new_image sfdisk-legacy-63-chs
sfdisk "$work/sfdisk-legacy-63-chs.img" >/dev/null <<'SCRIPT'
label: dos
start=63, size=50000, Id=0C, bootable
start=, size=40000, Id=82
SCRIPT

new_image sfdisk-aligned-pair
sfdisk "$work/sfdisk-aligned-pair.img" >/dev/null <<'SCRIPT'
label: dos
start=2048, size=81920, Id=83
start=, size=81920, Id=83
SCRIPT

new_image sfdisk-four-primaries
sfdisk "$work/sfdisk-four-primaries.img" >/dev/null <<'SCRIPT'
label: dos
start=2048, size=50000, Id=0B, bootable
start=, size=50000, Id=83
start=, size=50000, Id=82
start=, size=30000, Id=16
SCRIPT

new_image sfdisk-deleted-slot1
sfdisk "$work/sfdisk-deleted-slot1.img" >/dev/null <<'SCRIPT'
label: dos
start=2048, size=50000, Id=0B, bootable
start=, size=50000, Id=83
start=, size=50000, Id=82
start=, size=30000, Id=16
SCRIPT
printf 'd\n2\nw\n' | fdisk "$work/sfdisk-deleted-slot1.img" >/dev/null

new_image sfdisk-container-05
sfdisk "$work/sfdisk-container-05.img" >/dev/null <<'SCRIPT'
label: dos
start=2048, size=150000, Id=05
SCRIPT

new_image parted-extended-0f-logical
parted -s "$work/parted-extended-0f-logical.img" \
    mklabel msdos \
    mkpart extended 1049s 100% \
    mkpart logical 1050s 60000s >/dev/null 2>&1

new_image sfdisk-data-and-container
sfdisk "$work/sfdisk-data-and-container.img" >/dev/null <<'SCRIPT'
label: dos
start=2048, size=90000, Id=83, bootable
start=, size=100000, Id=0F
SCRIPT

new_image gdisk-protective-ee
printf 'n\n\n\n\n8300\nw\nY\n' | gdisk "$work/gdisk-protective-ee.img" >/dev/null

for name in sfdisk-legacy-63-chs sfdisk-aligned-pair sfdisk-four-primaries \
    sfdisk-deleted-slot1 sfdisk-container-05 parted-extended-0f-logical \
    sfdisk-data-and-container gdisk-protective-ee; do
    capture "$name"
done

ls -l "$out"/*.mbr
