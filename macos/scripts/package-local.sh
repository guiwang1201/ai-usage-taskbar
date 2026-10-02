#!/bin/sh
# Run build-prototype.sh first. This prepares a local app; it does not install or launch it.
set -eu
BUILD_ROOT=${IUSAGE_MAC_BUILD_DIR:-/private/tmp/iusage-mac-build}
MAC_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
SOURCE_APP="$BUILD_ROOT/IUsage Prototype.app"
APP_DIR="$BUILD_ROOT/release/IUsage.app"
test -x "$SOURCE_APP/Contents/MacOS/IUsagePrototype"
mkdir -p "$BUILD_ROOT/release"
if [ -e "$APP_DIR" ]; then
  mv "$APP_DIR" "$BUILD_ROOT/release/IUsage-$(date +%Y%m%d-%H%M%S).app"
fi
ditto "$SOURCE_APP" "$APP_DIR"
mkdir -p "$APP_DIR/Contents/Resources"
cp "$MAC_ROOT/Assets/IUsage.icns" "$APP_DIR/Contents/Resources/IUsage.icns"
/usr/libexec/PlistBuddy -c 'Add :CFBundleIconFile string IUsage' "$APP_DIR/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIdentifier com.iusage.mac' "$APP_DIR/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Set :CFBundleName IUsage' "$APP_DIR/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :IUsageDefaultLive bool true' "$APP_DIR/Contents/Info.plist"
/usr/libexec/PlistBuddy -c 'Add :LSMinimumSystemVersion string 13.0' "$APP_DIR/Contents/Info.plist"
codesign --force --sign - "$APP_DIR"
codesign --verify --strict "$APP_DIR"
shasum -a 256 "$APP_DIR/Contents/MacOS/IUsagePrototype" > "$BUILD_ROOT/release/SHA256.txt"
printf '%s\n' "$APP_DIR"
