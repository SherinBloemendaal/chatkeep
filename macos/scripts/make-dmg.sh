#!/usr/bin/env bash
# Builds build/Chatkeep.dmg from build/Chatkeep.app: the app, an Applications link and the
# drag-to-install background. Run ./build-app.sh first.
#
#   VERSION=1.2.0   names the disk "Chatkeep 1.2.0" (default: the version build-app.sh put in the app)
#
# Uses dmgbuild, which writes the window layout itself, so it works without a Finder session
# (in CI too). It is installed into build/dmgbuild-venv on first use.
set -euo pipefail

cd "$(dirname "$0")/.."

dmgbuild_version="1.6.7"
app="build/Chatkeep.app"
dmg="build/Chatkeep.dmg"
[ -d "$app" ] || { echo "make-dmg: $app is missing; run ./build-app.sh first" >&2; exit 1; }

venv="build/dmgbuild-venv"
if [ ! -x "$venv/bin/dmgbuild" ]; then
    python3 -m venv "$venv"
    "$venv/bin/pip" install --quiet "dmgbuild==$dmgbuild_version"
fi

VERSION="${VERSION:-$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")}"
volume_name="Chatkeep $VERSION"
rm -f "$dmg"
"$venv/bin/dmgbuild" -s scripts/dmg-settings.py -D app="$app" "$volume_name" "$dmg"
echo "Built $dmg"
