#!/bin/sh
set -eu
MAC_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUILD_ROOT=${IUSAGE_MAC_BUILD_DIR:-/private/tmp/iusage-mac-build}
export CARGO_HOME="${CARGO_HOME:-$BUILD_ROOT/cargo-home}"
mkdir -p "$BUILD_ROOT/module-cache"
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  "$MAC_ROOT/Credentials/Credentials.swift" "$MAC_ROOT/Credentials/CodexStorage.swift" "$MAC_ROOT/Credentials/LocalSources.swift" "$MAC_ROOT/Credentials/LocalIssue.swift" "$MAC_ROOT/Tests/main.swift" -o "$BUILD_ROOT/credential-tests"
"$BUILD_ROOT/credential-tests"
cargo test --offline --manifest-path "$MAC_ROOT/Core/Cargo.toml" --target-dir "$BUILD_ROOT/rust"
cargo build --release --offline --manifest-path "$MAC_ROOT/Core/Cargo.toml" --target-dir "$BUILD_ROOT/rust"
CORE_LIB="$BUILD_ROOT/rust/release/libiusage_mac_core.a"
CORE_HEADER="$MAC_ROOT/Core/include/IUsageCore.h"
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  -import-objc-header "$CORE_HEADER" "$MAC_ROOT/CoreTests/main.swift" "$CORE_LIB" \
  -o "$BUILD_ROOT/core-integration-tests"
"$BUILD_ROOT/core-integration-tests"
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  -import-objc-header "$CORE_HEADER" "$MAC_ROOT/Runtime/AlertPolicy.swift" "$MAC_ROOT/Runtime/DurableFile.swift" "$MAC_ROOT/Runtime/OfflineRuntime.swift" \
  "$MAC_ROOT/Prototype/DisplayFormat.swift" "$MAC_ROOT/RuntimeTests/main.swift" "$CORE_LIB" -o "$BUILD_ROOT/runtime-tests"
"$BUILD_ROOT/runtime-tests"
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  "$MAC_ROOT/Network/UsageHTTP.swift" "$MAC_ROOT/NetworkTests/main.swift" -o "$BUILD_ROOT/network-tests"
"$BUILD_ROOT/network-tests"
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  -import-objc-header "$CORE_HEADER" "$MAC_ROOT/Runtime/AlertPolicy.swift" "$MAC_ROOT/Runtime/DurableFile.swift" "$MAC_ROOT/Runtime/OfflineRuntime.swift" \
  "$MAC_ROOT/Network/UsageHTTP.swift" "$MAC_ROOT/Network/FetchCoordinator.swift" "$MAC_ROOT/Credentials/ClaudeSession.swift" \
  "$MAC_ROOT/Credentials/Credentials.swift" "$MAC_ROOT/CoordinatorTests/main.swift" "$CORE_LIB" -o "$BUILD_ROOT/coordinator-tests"
"$BUILD_ROOT/coordinator-tests"
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  -import-objc-header "$CORE_HEADER" "$MAC_ROOT/Runtime/AlertPolicy.swift" "$MAC_ROOT/Runtime/DurableFile.swift" "$MAC_ROOT/Runtime/OfflineRuntime.swift" \
  "$MAC_ROOT/Credentials/Credentials.swift" "$MAC_ROOT/Credentials/LocalIssue.swift" "$MAC_ROOT/Network/UsageHTTP.swift" \
  "$MAC_ROOT/Runtime/ClaudeStore.swift" "$MAC_ROOT/Runtime/ClaudeRenewal.swift" "$MAC_ROOT/RenewalTests/main.swift" "$CORE_LIB" -o "$BUILD_ROOT/renewal-tests"
"$BUILD_ROOT/renewal-tests"
# Compile the diagnostic entry point too, without executing it or reading credentials.
swiftc -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
  -import-objc-header "$CORE_HEADER" "$MAC_ROOT/Runtime/AlertPolicy.swift" "$MAC_ROOT/Runtime/DurableFile.swift" "$MAC_ROOT/Runtime/OfflineRuntime.swift" \
  "$MAC_ROOT/Credentials/Credentials.swift" "$MAC_ROOT/Credentials/CodexStorage.swift" "$MAC_ROOT/Credentials/LocalSources.swift" \
  "$MAC_ROOT/Credentials/ClaudeSession.swift" "$MAC_ROOT/Network/UsageHTTP.swift" "$MAC_ROOT/Network/FetchCoordinator.swift" \
  "$MAC_ROOT/OnlineCheck/main.swift" "$CORE_LIB" -o "$BUILD_ROOT/online-check"
APP_DIR="$BUILD_ROOT/IUsage Prototype.app"
mkdir -p "$APP_DIR/Contents/MacOS"
swiftc -Osize -Xlinker -dead_strip -warnings-as-errors -module-cache-path "$BUILD_ROOT/module-cache" \
   -import-objc-header "$CORE_HEADER" "$MAC_ROOT/Prototype/CoreUsage.swift" \
  "$MAC_ROOT/Prototype/DisplayFormat.swift" "$MAC_ROOT/Prototype/UsageBars.swift" "$MAC_ROOT/Runtime/AlertPolicy.swift" "$MAC_ROOT/Runtime/DurableFile.swift" "$MAC_ROOT/Runtime/OfflineRuntime.swift" \
  "$MAC_ROOT/Credentials/Credentials.swift" "$MAC_ROOT/Credentials/CodexStorage.swift" "$MAC_ROOT/Credentials/LocalSources.swift" "$MAC_ROOT/Credentials/LocalIssue.swift" \
  "$MAC_ROOT/Network/UsageHTTP.swift" "$MAC_ROOT/Network/FetchCoordinator.swift" "$MAC_ROOT/Credentials/ClaudeSession.swift" \
  "$MAC_ROOT/Prototype/SystemFeatures.swift" "$MAC_ROOT/Prototype/main.swift" "$CORE_LIB" -o "$APP_DIR/Contents/MacOS/IUsagePrototype"
cat > "$APP_DIR/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>IUsagePrototype</string>
<key>CFBundleIdentifier</key><string>com.iusage.offline-prototype</string>
<key>CFBundleName</key><string>IUsage Prototype</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
strip -x "$APP_DIR/Contents/MacOS/IUsagePrototype"
codesign --force --sign - "$APP_DIR"
printf '%s\n' "$APP_DIR"
