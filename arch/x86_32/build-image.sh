#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(realpath "$SCRIPT_DIR/../..")"

PROFILE="${PROFILE:-release}"
CARGO_FLAGS="$([ "$PROFILE" = "release" ] && echo "--release" || echo "")"
KERNEL="$REPO_ROOT/target/rosx-i686/$PROFILE/rosx-x86"
OUTPUT="${1:-$REPO_ROOT/target/rosx-i686/$PROFILE/rosx-x86_32.img}"

# ── Tool checks ───────────────────────────────────────────────────────────────

GRUB_MKRESCUE=""
for cmd in grub-mkrescue grub2-mkrescue; do
    if command -v "$cmd" &>/dev/null; then
        GRUB_MKRESCUE="$cmd"
        break
    fi
done

MISSING=()
[ -z "$GRUB_MKRESCUE" ] && MISSING+=("grub-mkrescue")
command -v xorriso &>/dev/null || MISSING+=("xorriso")
command -v mcopy   &>/dev/null || MISSING+=("mtools")

if [ ${#MISSING[@]} -gt 0 ]; then
    echo "error: missing required tools: ${MISSING[*]}"
    echo ""
    if command -v apt-get &>/dev/null; then
        echo "  sudo apt install grub-pc-bin grub-common xorriso mtools"
    elif command -v dnf &>/dev/null; then
        echo "  sudo dnf install grub2-tools xorriso mtools"
    elif command -v pacman &>/dev/null; then
        echo "  sudo pacman -S grub xorriso mtools"
    else
        echo "  Install: grub (grub-pc-bin), xorriso, mtools"
    fi
    exit 1
fi

# ── Build ─────────────────────────────────────────────────────────────────────

echo "Building kernel (profile: $PROFILE)..."
cd "$SCRIPT_DIR"
cargo build $CARGO_FLAGS

# ── Assemble image ────────────────────────────────────────────────────────────

WORK=$(mktemp -d)
trap "rm -rf '$WORK'" EXIT

mkdir -p "$WORK/boot/grub"
cp "$KERNEL" "$WORK/boot/kernel"

cat > "$WORK/boot/grub/grub.cfg" << 'EOF'
set timeout=0
set default=0

menuentry "rosx" {
    multiboot /boot/kernel
    boot
}
EOF

echo "Creating image: $OUTPUT"
"$GRUB_MKRESCUE" -o "$OUTPUT" "$WORK"

echo ""
echo "Image: $OUTPUT"
echo "USB:   sudo dd if=\"$OUTPUT\" of=/dev/sdX bs=4M status=progress && sync"
