#!/bin/sh
# Fetch and hash-verify the pinned GRUB Legacy (assets/grub-legacy.toml),
# then extract its i386-pc stages under <cache>/stages. Downloads land in
# <cache>; a re-run with a valid cache fetches nothing, so the flow survives
# archive.debian.org rot. Never vendors binaries into git.
#
# usage: fetch-grub-legacy.sh [cache-dir]   (default: <repo>/target/grub-legacy)
# stdout: the stage directory (absolute) as the last line.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
manifest=$here/../assets/grub-legacy.toml
cache=${1:-$repo/target/grub-legacy}

for tool in curl sha256sum dpkg-deb; do
    command -v "$tool" >/dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 1; }
done

toml_get() {
    sed -n "s/^$1 *= *\"\([^\"]*\)\".*/\1/p" "$manifest" | head -1
}

url=$(toml_get url)
deb_sha=$(toml_get sha256)
version=$(toml_get version)
deb=$cache/grub-legacy_${version}_i386.deb
stages=$cache/stages

verify() {
    [ -f "$1" ] && printf '%s  %s\n' "$2" "$1" | sha256sum --check --quiet --status
}

mkdir -p "$cache" "$stages"
if [ ! -f "$deb" ] || ! verify "$deb" "$deb_sha"; then
    [ -f "$deb" ] && echo "cached deb failed its sha256, re-fetching" >&2
    echo "fetching $url" >&2
    curl -fsSL "$url" -o "$deb.tmp"
    verify "$deb.tmp" "$deb_sha" || {
        echo "sha256 mismatch for the pinned grub-legacy deb; refusing it" >&2
        rm -f "$deb.tmp"
        exit 1
    }
    mv "$deb.tmp" "$deb"
fi

extract=$cache/extract
for stage in stage1 stage2 fat_stage1_5 stage2_eltorito; do
    want=$(toml_get "$stage")
    if ! verify "$stages/$stage" "$want"; then
        rm -rf "$extract"
        mkdir -p "$extract"
        dpkg-deb -x "$deb" "$extract"
        src=$extract/usr/lib/grub/i386-pc/$stage
        [ -f "$src" ] || { echo "stage $stage absent from the deb" >&2; exit 1; }
        cp "$src" "$stages/$stage.tmp"
        verify "$stages/$stage.tmp" "$want" || {
            echo "sha256 mismatch for extracted stage $stage" >&2
            rm -f "$stages/$stage.tmp"
            exit 1
        }
        mv "$stages/$stage.tmp" "$stages/$stage"
    fi
done
rm -rf "$extract"

echo "$stages"
