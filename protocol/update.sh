#!/bin/sh
# Refresh the vendored river protocol XMLs from upstream.
# Usage: protocol/update.sh [ref]   (ref = river branch or commit, default main)
set -eu

ref="${1:-main}"
base="https://codeberg.org/river/river/raw/commit/$ref/protocol"
dir="$(cd "$(dirname "$0")" && pwd)"

for name in river-window-management-v1 river-xkb-bindings-v1 river-layer-shell-v1; do
    tmp="$dir/$name.xml.tmp"
    curl -fsS -o "$tmp" "$base/$name.xml"
    mv "$tmp" "$dir/$name.xml"
    printf '%s  %s\n' "$(sha256sum "$dir/$name.xml" | cut -d' ' -f1)" "$name.xml"
done

echo "updated from river $ref; run 'cargo test' and check src/ for protocol API changes"
