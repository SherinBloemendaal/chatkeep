#!/usr/bin/env bash
# Builds Chatkeep.app into ./build, ad-hoc signed so it runs without a developer certificate.
# The app runs the chatkeep command line tool for every action, so this also builds that tool
# from the repository root and puts it in Contents/Resources/chatkeep.
#
#   VERSION=1.2.0   sets the version shown in Finder and About (default: the one in ../Cargo.toml,
#                   so the app and the tool inside it never drift apart)
#   UNIVERSAL=1     builds for Apple Silicon and Intel in one app (used for releases)
set -euo pipefail

cd "$(dirname "$0")"
repo_root="$(cd .. && pwd)"

if [ -z "${VERSION:-}" ]; then
    VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$repo_root/Cargo.toml" | head -n 1)"
    [ -n "$VERSION" ] || { echo "build-app: no version found in $repo_root/Cargo.toml" >&2; exit 1; }
fi

arch_flags=()
rust_targets=()
if [ "${UNIVERSAL:-0}" = "1" ]; then
    arch_flags=(--arch arm64 --arch x86_64)
    rust_targets=(aarch64-apple-darwin x86_64-apple-darwin)
fi

swift build -c release --product Chatkeep ${arch_flags[@]+"${arch_flags[@]}"}
bin_path="$(swift build -c release --product Chatkeep ${arch_flags[@]+"${arch_flags[@]}"} --show-bin-path)"
binary="$bin_path/Chatkeep"

cli_dir="build/cli"
rm -rf "$cli_dir"
mkdir -p "$cli_dir"
if [ ${#rust_targets[@]} -eq 0 ]; then
    cargo build --release --locked --manifest-path "$repo_root/Cargo.toml"
    cp "$repo_root/target/release/chatkeep" "$cli_dir/chatkeep"
else
    slices=()
    for target in "${rust_targets[@]}"; do
        cargo build --release --locked --manifest-path "$repo_root/Cargo.toml" --target "$target"
        slices+=("$repo_root/target/$target/release/chatkeep")
    done
    lipo -create "${slices[@]}" -output "$cli_dir/chatkeep"
fi

app="build/Chatkeep.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
cp "$binary" "$app/Contents/MacOS/Chatkeep"
# Sparkle (the updater) ships as a framework; ditto keeps its symlinks intact.
ditto "$bin_path/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"
install_name_tool -add_rpath "@executable_path/../Frameworks" "$app/Contents/MacOS/Chatkeep"
cp Resources/Info.plist "$app/Contents/Info.plist"
cp Resources/AppIcon.icns "$app/Contents/Resources/AppIcon.icns"
cp "$cli_dir/chatkeep" "$app/Contents/Resources/chatkeep"
chmod 755 "$app/Contents/Resources/chatkeep"

plutil -replace CFBundleShortVersionString -string "$VERSION" "$app/Contents/Info.plist"
plutil -replace CFBundleVersion -string "$VERSION" "$app/Contents/Info.plist"

# Sign what is nested before the app: signing the app seals everything inside it. Sparkle's
# helpers go innermost first, as its documentation describes for signing it by hand.
sparkle="$app/Contents/Frameworks/Sparkle.framework/Versions/B"
codesign --force --sign - "$sparkle/XPCServices/Installer.xpc"
codesign --force --sign - --preserve-metadata=entitlements "$sparkle/XPCServices/Downloader.xpc"
codesign --force --sign - "$sparkle/Autoupdate"
codesign --force --sign - "$sparkle/Updater.app"
codesign --force --sign - "$app/Contents/Frameworks/Sparkle.framework"
codesign --force --sign - --identifier io.github.sherinbloemendaal.chatkeep.cli "$app/Contents/Resources/chatkeep"
codesign --force --sign - "$app"
echo "Built $app ($VERSION)"
