#!/usr/bin/env bash
# Writes build/appcast.xml, the feed the app's updater (Sparkle) reads, for build/Chatkeep.dmg.
# Run ./build-app.sh and scripts/make-dmg.sh first.
#
#   SPARKLE_ED_PRIVATE_KEY   the private key that signs the DMG (required); the app only installs
#                            a DMG signed with it (its public half is SUPublicEDKey in Info.plist)
#   SIGN_UPDATE              path to Sparkle's sign_update tool (required)
#   DOWNLOAD_URL             where the DMG is published (default: the GitHub release of the version)
#
# Each release attaches its appcast.xml; the app reads the one of the latest release.
set -euo pipefail

cd "$(dirname "$0")/.."

app="build/Chatkeep.app"
dmg="build/Chatkeep.dmg"
feed="build/appcast.xml"
[ -f "$dmg" ] || { echo "make-appcast: $dmg is missing; run scripts/make-dmg.sh first" >&2; exit 1; }
[ -n "${SPARKLE_ED_PRIVATE_KEY:-}" ] || { echo "make-appcast: SPARKLE_ED_PRIVATE_KEY is not set" >&2; exit 1; }
[ -x "${SIGN_UPDATE:-}" ] || { echo "make-appcast: SIGN_UPDATE does not point to sign_update" >&2; exit 1; }

plist="$app/Contents/Info.plist"
short_version="$(plutil -extract CFBundleShortVersionString raw "$plist")"
build_version="$(plutil -extract CFBundleVersion raw "$plist")"
minimum_system="$(plutil -extract LSMinimumSystemVersion raw "$plist")"
release_page="https://github.com/SherinBloemendaal/chatkeep/releases/tag/v$short_version"
download_url="${DOWNLOAD_URL:-https://github.com/SherinBloemendaal/chatkeep/releases/download/v$short_version/Chatkeep.dmg}"

# Prints: sparkle:edSignature="…" length="…"
signature="$(printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | "$SIGN_UPDATE" --ed-key-file - "$dmg")"
[[ "$signature" == sparkle:edSignature=* ]] || { echo "make-appcast: sign_update gave no signature" >&2; exit 1; }

cat > "$feed" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
    <channel>
        <title>Chatkeep</title>
        <link>https://github.com/SherinBloemendaal/chatkeep</link>
        <item>
            <title>Chatkeep $short_version</title>
            <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
            <sparkle:version>$build_version</sparkle:version>
            <sparkle:shortVersionString>$short_version</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>$minimum_system</sparkle:minimumSystemVersion>
            <sparkle:releaseNotesLink>$release_page</sparkle:releaseNotesLink>
            <enclosure url="$download_url" type="application/x-apple-diskimage" $signature/>
        </item>
    </channel>
</rss>
EOF
echo "Built $feed ($short_version)"
