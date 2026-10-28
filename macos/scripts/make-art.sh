#!/usr/bin/env bash
# Turns the drawings in art/ into the files the app and the DMG use:
#   Resources/AppIcon.icns          app and disk icon
#   Resources/dmg-background.tiff   DMG window background, normal and Retina in one file
#
# Run it after changing an SVG and commit the results; builds do not need rsvg-convert.
# Needs rsvg-convert (brew install librsvg).
set -euo pipefail

cd "$(dirname "$0")/.."

command -v rsvg-convert >/dev/null || { echo "make-art: install rsvg-convert with: brew install librsvg" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

iconset="$work/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
    rsvg-convert -w "$size" -h "$size" art/AppIcon.svg -o "$iconset/icon_${size}x${size}.png"
    rsvg-convert -w $((size * 2)) -h $((size * 2)) art/AppIcon.svg -o "$iconset/icon_${size}x${size}@2x.png"
done
iconutil -c icns "$iconset" -o Resources/AppIcon.icns

rsvg-convert -w 660 -h 400 art/dmg-background.svg -o "$work/background.png"
rsvg-convert -w 1320 -h 800 art/dmg-background.svg -o "$work/background@2x.png"
tiffutil -cathidpicheck "$work/background.png" "$work/background@2x.png" -out Resources/dmg-background.tiff 2>/dev/null

echo "Wrote Resources/AppIcon.icns and Resources/dmg-background.tiff"
