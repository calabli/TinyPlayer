#!/usr/bin/env bash
# Builds dist/TinyPlayer.app and dist/TinyPlayer-<version>-macos-<arch>.dmg.
#
# Requirements:
#   brew install mpv dylibbundler
# Optional environment:
#   SIGN_IDENTITY   "Developer ID Application: Name (TEAMID)"; default is ad-hoc signing
#   NOTARY_PROFILE  keychain profile from `xcrun notarytool store-credentials`
#                   (only used together with SIGN_IDENTITY)
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
APP=dist/TinyPlayer.app
DMG="dist/TinyPlayer-$VERSION-macos-$ARCH.dmg"

echo "Building release..."
cargo build --release

echo "Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Frameworks" "$APP/Contents/Resources"
cp target/release/tinyplayer "$APP/Contents/MacOS/TinyPlayer"

# libmpv: prefer a copy in third_party/, fall back to Homebrew's.
LIBMPV=third_party/mpv/macos/libmpv.2.dylib
if [ ! -f "$LIBMPV" ]; then
    LIBMPV="$(brew --prefix mpv)/lib/libmpv.2.dylib"
fi
[ -f "$LIBMPV" ] || { echo "libmpv not found (brew install mpv)"; exit 1; }
cp "$LIBMPV" "$APP/Contents/Frameworks/libmpv.2.dylib"
chmod u+w "$APP/Contents/Frameworks/libmpv.2.dylib"
# The app looks for libmpv in Contents/Frameworks (see src/mpv.rs).
install_name_tool -id @rpath/libmpv.2.dylib "$APP/Contents/Frameworks/libmpv.2.dylib"

# Copy libmpv's own dependencies (ffmpeg, libass, ...) next to it and
# rewrite their load paths so the bundle is self-contained.
command -v dylibbundler >/dev/null || { echo "dylibbundler missing (brew install dylibbundler)"; exit 1; }
dylibbundler --overwrite-files --bundle-deps \
    --fix-file "$APP/Contents/Frameworks/libmpv.2.dylib" \
    --dest-dir "$APP/Contents/Frameworks/" \
    --install-path @loader_path/

# dylibbundler can leave the same rpath twice, and dyld refuses to load a
# library like that. Keep one of each.
for lib in "$APP"/Contents/Frameworks/*.dylib; do
    otool -l "$lib" | awk '/cmd LC_RPATH/ { getline; getline; print $2 }' | sort | uniq -d |
        while read -r rpath; do
            install_name_tool -delete_rpath "$rpath" "$lib" 2>/dev/null
        done
done

# Homebrew builds for the system it runs on, so the bundle can't run on
# anything older than its newest library asks for. Declare that, or older
# systems open the app and then fail to load libmpv.
MIN_MACOS=$(for lib in "$APP"/Contents/Frameworks/*.dylib; do
    vtool -show-build "$lib" | awk '$1 == "minos" { print $2 }'
done | sort -V | tail -1)
echo "Minimum macOS: $MIN_MACOS"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>TinyPlayer</string>
    <key>CFBundleDisplayName</key><string>TinyPlayer</string>
    <key>CFBundleIdentifier</key><string>app.tinyplayer.TinyPlayer</string>
    <key>CFBundleExecutable</key><string>TinyPlayer</string>
    <key>CFBundleIconFile</key><string>TinyPlayer</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_MACOS</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.video</string>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Media</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>LSItemContentTypes</key>
            <array>
                <string>public.movie</string>
                <string>public.audio</string>
            </array>
        </dict>
        <!-- macOS has no type for these, so public.movie doesn't cover them. -->
        <dict>
            <key>CFBundleTypeName</key><string>Video</string>
            <key>CFBundleTypeRole</key><string>Viewer</string>
            <key>LSHandlerRank</key><string>Alternate</string>
            <key>CFBundleTypeExtensions</key>
            <array>
                <string>mkv</string><string>webm</string><string>flv</string>
                <string>rmvb</string><string>wmv</string><string>ts</string>
                <string>m2ts</string><string>ogv</string><string>opus</string>
            </array>
        </dict>
    </array>
</dict>
</plist>
PLIST

cp README.md README.en.md THIRD_PARTY.md THIRD_PARTY.en.md LICENSE "$APP/Contents/Resources/"

# App icon: build an .icns from assets/icon-512.png (scripts/make-icon.py).
ICONSET=dist/TinyPlayer.iconset
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
for s in 16 32 128 256; do
    sips -z $s $s assets/icon-512.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z $((s * 2)) $((s * 2)) assets/icon-512.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
cp assets/icon-512.png "$ICONSET/icon_512x512.png"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/TinyPlayer.icns"
rm -rf "$ICONSET"

echo "Signing"
if [ -n "${SIGN_IDENTITY:-}" ]; then
    # Sign libraries first, then the app, with the hardened runtime for notarization.
    find "$APP/Contents/Frameworks" -name '*.dylib' -exec \
        codesign --force --timestamp --options runtime --sign "$SIGN_IDENTITY" {} \;
    codesign --force --timestamp --options runtime --sign "$SIGN_IDENTITY" "$APP"
else
    # Ad-hoc: runs on this Mac; other Macs must allow it once under System
    # Settings → Privacy & Security.
    find "$APP/Contents/Frameworks" -name '*.dylib' -exec codesign --force --sign - {} \;
    codesign --force --sign - "$APP"
fi
codesign --verify --deep --strict "$APP"

# A library that won't load only shows up at runtime, as a player that can't
# play, so check every one here. Homebrew must not be able to fill in for a
# file the bundle is missing, hence no search paths and no Homebrew in them.
echo "Checking bundled libraries"
if otool -L "$APP"/Contents/Frameworks/*.dylib | grep -E '^\s+(/opt/homebrew|/usr/local)'; then
    echo "bundle still depends on the libraries above"; exit 1
fi
CHECK=$(mktemp -d)
cat > "$CHECK/check.c" <<'C'
#include <dlfcn.h>
#include <stdio.h>
int main(int argc, char **argv) {
    int failed = 0;
    for (int i = 1; i < argc; i++)
        if (!dlopen(argv[i], RTLD_NOW)) { printf("%s\n", dlerror()); failed = 1; }
    return failed;
}
C
cc "$CHECK/check.c" -o "$CHECK/check"
"$CHECK/check" "$APP"/Contents/Frameworks/*.dylib
rm -rf "$CHECK"

echo "Creating $DMG"
rm -f "$DMG"
# The window that opens with the disk image: drag the app onto Applications.
STAGE=dist/dmg-stage
rm -rf "$STAGE"; mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
# hdiutil now and then fails with "Resource busy", most often on CI machines.
for attempt in 1 2 3 4 5; do
    hdiutil create -volname TinyPlayer -srcfolder "$STAGE" -ov -format UDZO "$DMG" && break
    [ "$attempt" -lt 5 ] || exit 1
    echo "hdiutil failed, retrying"
    sleep 5
done
rm -rf "$STAGE"

if [ -n "${SIGN_IDENTITY:-}" ] && [ -n "${NOTARY_PROFILE:-}" ]; then
    echo "Notarizing (this can take a few minutes)"
    xcrun notarytool submit "$DMG" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
fi

du -sh "$APP" "$DMG"
