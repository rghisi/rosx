#!/bin/sh
# Era boot media recipe 1 (plan §2.5): a GRUB Legacy 0.97 El Torito CD
# carrying the RosX x86_32 kernel. stage2_eltorito passes a sizes-only
# Multiboot 1.0 info block (FLAGS bit 0), so a boot through it exercises
# the mem_upper fallback path. The [SiS 496] ASUS PVI-486SP3 BIOS has no
# El Torito, so on 486sp3c this medium is for the QEMU -cpu 486 control:
#   qemu-system-i386 -cpu 486 -m 8 -cdrom <iso> -boot order=d \
#       -debugcon stdio -no-reboot -display none
#
# usage: build-legacy-iso.sh <kernel-elf> <rom-image> <stages-dir> <output-iso>
set -eu

kernel=$1
rom=$2
stages=$3
out=$4

for tool in xorriso mktemp; do
    command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 1; }
done
[ -f "$kernel" ] || { echo "no kernel ELF at $kernel (run: cargo xtask build x86_32)" >&2; exit 1; }
[ -f "$rom" ] || { echo "no ROM image at $rom (run: cargo xtask rom x86_32)" >&2; exit 1; }
[ -f "$stages/stage2_eltorito" ] || { echo "no stage2_eltorito in $stages (run fetch-grub-legacy.sh)" >&2; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/boot/grub"
cp "$stages/stage2_eltorito" "$work/boot/grub/"
cp "$kernel" "$work/boot/rosx"
cp "$rom" "$work/boot/rom"
printf 'timeout 0\ntitle RosX x86_32 on 486SX via GRUB Legacy\nkernel /boot/rosx\nmodule /boot/rom\n' \
    > "$work/boot/grub/menu.lst"

xorriso -as mkisofs -r -V ROSX_LEGACY \
    -b boot/grub/stage2_eltorito -no-emul-boot -boot-load-size 4 -boot-info-table \
    -o "$out" "$work"
echo "==> wrote era El Torito ISO $out"
