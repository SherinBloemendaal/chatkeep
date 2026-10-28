#!/usr/bin/env bash
# Writes build/appcast.xml, the feed the app's updater (Sparkle) reads, for build/Chatkeep.dmg, and
# signs both the DMG and the feed, so the app only installs updates made by this script.
# Run ./build-app.sh and scripts/make-dmg.sh first.
#
#   SPARKLE_ED_PRIVATE_KEY   the private EdDSA key; without it the key is read from 1Password (needs `op`)
#   DOWNLOAD_URL             where the DMG is published (default: the GitHub release of the version)
#
# Each release attaches its appcast.xml; the app reads the one of the latest release.
set -euo pipefail

cd "$(dirname "$0")/.."

app="build/Chatkeep.app"
dmg="build/Chatkeep.dmg"
feed="build/appcast.xml"
repo="SherinBloemendaal/chatkeep"
[ -f "$dmg" ] || { echo "make-appcast: $dmg is missing; run scripts/make-dmg.sh first" >&2; exit 1; }

# The copy of Sparkle's tools that swift build downloaded, checked against the package's checksum.
sign_update="$(find .build/artifacts -path '*/Sparkle/bin/sign_update' -type f | head -n 1)"
[ -x "$sign_update" ] || { echo "make-appcast: sign_update not found; run ./build-app.sh first" >&2; exit 1; }

# The key lives only in 1Password; CI gets a copy as the SPARKLE_ED_PRIVATE_KEY secret.
key_item="op://Employee/bvi6qqs4u2dorgcygt6khpg5oa/password"
if [ -z "${SPARKLE_ED_PRIVATE_KEY:-}" ]; then
    SPARKLE_ED_PRIVATE_KEY="$(op read "$key_item")"
fi

sign() {
    printf '%s' "$SPARKLE_ED_PRIVATE_KEY" | "$sign_update" --ed-key-file - "$@"
}

escape() {
    sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

plist="$app/Contents/Info.plist"
short_version="$(plutil -extract CFBundleShortVersionString raw "$plist")"
build_version="$(plutil -extract CFBundleVersion raw "$plist")"
minimum_system="$(plutil -extract LSMinimumSystemVersion raw "$plist")"
download_url="${DOWNLOAD_URL:-https://github.com/$repo/releases/download/v$short_version/Chatkeep.dmg}"

# What's new: the feat and fix commits since the previous release, without their prefix.
previous="$(git describe --tags --abbrev=0 --match 'v*' "v$short_version^" 2>/dev/null || true)"
range="${previous:+$previous..}v$short_version"
notes="$(git log --format=%s "$range" 2>/dev/null \
    | grep -E '^(feat|fix)(\(.+\))?: ' \
    | sed -E 's/^[a-z]+(\([^)]*\))?: //' \
    | escape \
    | perl -pe 's|^(.)(.*)$|            <li>\u$1$2</li>|' || true)"
[ -n "$notes" ] || notes="            <li>Small improvements.</li>"

signature="$(sign -p "$dmg")"
length="$(stat -f %z "$dmg")"

cat > "$feed" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
    <channel>
        <title>Chatkeep</title>
        <link>https://github.com/$repo</link>
        <item>
            <title>Chatkeep $short_version</title>
            <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
            <sparkle:version>$build_version</sparkle:version>
            <sparkle:shortVersionString>$short_version</sparkle:shortVersionString>
            <sparkle:minimumSystemVersion>$minimum_system</sparkle:minimumSystemVersion>
            <sparkle:fullReleaseNotesLink>https://github.com/$repo/releases/tag/v$short_version</sparkle:fullReleaseNotesLink>
            <description><![CDATA[
        <h2>What's new in Chatkeep $short_version</h2>
        <ul>
$notes
        </ul>
            ]]></description>
            <enclosure url="$download_url" length="$length" type="application/x-apple-diskimage"
                       sparkle:edSignature="$signature"/>
        </item>
    </channel>
</rss>
EOF

sign "$feed"
echo "Built $feed ($short_version)"
