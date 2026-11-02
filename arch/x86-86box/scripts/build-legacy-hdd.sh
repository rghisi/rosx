#!/bin/sh
# Era boot media recipe 2 (plan §2.5): the AGENTS.md hardware recipe
# executed literally — GRUB Legacy 0.97-67 on a bootable FAT16 HDD with
# stage1 in the MBR and fat_stage1_5 in the gap, booting /boot/rosx. This
# is the medium that booted the emulated 486SX (machine 486sp3c) to rose>.
#
# The grub shell binary in the deb is an i386 ELF needing libtinfo5/
# libncurses5 (absent from trixie), so setup runs inside the pinned
# i386/debian:bullseye container — no loop devices, no --privileged:
# GRUB's `device (hd0) <file>` maps a BIOS disk onto a plain file.
#
# usage: build-legacy-hdd.sh <kernel-elf> <stages-dir> <output-img>
#        [--deb <grub-legacy deb>] [--setup-image <docker image>]
# Disk geometry is the verified 993/16/63 (488 MB, inside both BIOS
# translation limits of the era board); the partition and FAT are sized
# from the CHS count, not the file size (plan §7 geometry slack).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
manifest=$here/../assets/grub-legacy.toml

kernel=$1
stages=$2
img=$3
shift 3
deb=
setup_image=$(sed -n 's/^grub_shell *= *"\(.*\)".*/\1/p' "$manifest" | head -1)
while [ $# -gt 0 ]; do
    case $1 in
        --deb) deb=$2; shift 2 ;;
        --setup-image) setup_image=$2; shift 2 ;;
        *) echo "unknown flag: $1" >&2; exit 2 ;;
    esac
done

CYL=993 HEADS=16 SEC=63
TOTAL_SECTORS=$((CYL * HEADS * SEC))
PART_START=63
PART_SECTORS=1000266
PART_OFFSET=$((PART_START * 512))

for tool in sfdisk mformat mmd mcopy; do
    command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool (util-linux, mtools)" >&2; exit 1; }
done
[ -f "$kernel" ] || { echo "no kernel ELF at $kernel (run: cargo xtask build x86_32)" >&2; exit 1; }
for stage in stage1 stage2 fat_stage1_5; do
    [ -f "$stages/$stage" ] || { echo "no $stage in $stages (run fetch-grub-legacy.sh)" >&2; exit 1; }
done

[ -n "$deb" ] || {
    cache=$repo/target/grub-legacy
    version=$(sed -n 's/^version *= *"\([^\"]*\)\".*/\1/p' "$manifest" | head -1)
    deb=$cache/grub-legacy_${version}_i386.deb
    [ -f "$deb" ] || sh "$here/fetch-grub-legacy.sh" >/dev/null
}
[ -f "$deb" ] || { echo "no grub-legacy deb at $deb" >&2; exit 1; }

rm -f "$img"
truncate -s "$((TOTAL_SECTORS * 512))" "$img"

printf 'label: dos\n' \
    | sfdisk --no-reread --quiet "$img" >/dev/null
printf 'start=%d, size=%d, type=6, bootable\n' "$PART_START" "$PART_SECTORS" \
    | sfdisk --no-reread "$img" >/dev/null

mformat -i "$img@@$PART_OFFSET" -h "$HEADS" -s "$SEC" -T "$PART_SECTORS" -F -v ROSX
mmd -i "$img@@$PART_OFFSET" ::/boot ::/boot/grub
for stage in stage1 stage2 fat_stage1_5; do
    mcopy -m -o -i "$img@@$PART_OFFSET" "$stages/$stage" ::/boot/grub/
done
mcopy -m -o -i "$img@@$PART_OFFSET" "$kernel" ::/boot/rosx
printf 'timeout 5\ntitle RosX x86_32 on emulated 486SX-25 (GRUB Legacy %s on HDD)\nkernel /boot/rosx\n' \
    "$(sed -n 's/^version *= *"\([^\"]*\)\".*/\1/p' "$manifest" | head -1)" > "$here/.menu-lst.tmp"
mcopy -m -o -i "$img@@$PART_OFFSET" "$here/.menu-lst.tmp" ::/boot/grub/menu.lst
rm -f "$here/.menu-lst.tmp"

img_dir=$(cd "$(dirname "$img")" && pwd)
deb_dir=$(cd "$(dirname "$deb")" && pwd)
for path in "$img_dir" "$deb_dir" "$setup_image"; do
    case $path in
        *[!a-zA-Z0-9./_@:-]*)
            echo "path '$path' must be free of spaces/quotes (grub-shell + sg constraint)" >&2
            exit 1 ;;
    esac
done
mode=plain
if ! docker info >/dev/null 2>&1; then
    if sg docker -c 'docker info >/dev/null 2>&1'; then
        mode=sg
    else
        echo "docker not reachable (add the operator to group docker; agent shells need: sg docker -c '...')" >&2
        exit 1
    fi
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/rosx-era-hdd.XXXXXX")
trap 'rm -rf "$work"' EXIT
cat > "$work/grub-setup.sh" <<EOF
apt-get update -qq >/dev/null
apt-get install -y -qq libtinfo5 libncurses5 >/dev/null
dpkg -i '$deb' >/dev/null 2>&1
grub --batch --no-floppy --device-map=/dev/null <<GRUB
device (hd0) $img_dir/$(basename "$img")
root (hd0,0)
setup (hd0)
GRUB
EOF
if [ "$mode" = plain ]; then
    setup_log=$(docker run --rm \
        -v "$work:$work" -v "$img_dir:$img_dir" -v "$deb_dir:$deb_dir" \
        "$setup_image" bash "$work/grub-setup.sh" 2>&1)
else
    setup_log=$(sg docker -c "docker run --rm \
        -v $work:$work -v $img_dir:$img_dir -v $deb_dir:$deb_dir \
        $setup_image bash $work/grub-setup.sh" 2>&1)
fi
printf '%s\n' "$setup_log"
printf '%s\n' "$setup_log" | grep -q 'Done\.' || {
    echo "grub setup did not complete inside the container" >&2
    exit 1
}

dd if="$img" bs=32 count=1 of="$work/mbr.head" 2>/dev/null
dd if="$stages/stage1" bs=32 count=1 of="$work/stage1.head" 2>/dev/null
cmp -s "$work/mbr.head" "$work/stage1.head" || {
    echo "MBR does not carry the pinned stage1 code after setup" >&2
    exit 1
}
mdir -i "$img@@$PART_OFFSET" ::/boot/grub
echo "==> wrote era GRUB-Legacy HDD $img"
