#!/bin/bash
# Build Loupe.app and an installer .pkg.
#
#   ./scripts/package-macos.sh
#       unsigned Apple Silicon pkg (right-click Open)
#   IDENTITY_APP="Developer ID Application: Name (TEAM)" \
#   IDENTITY_PKG="Developer ID Installer: Name (TEAM)" \
#   APPLE_ID=... APPLE_TEAM_ID=... APPLE_APP_PASSWORD=... \
#   ./scripts/package-macos.sh --sign --notarize
set -euo pipefail
cd "$(dirname "$0")/.."

SIGN=0
NOTARIZE=0
for arg in "$@"; do
  case "$arg" in
    --sign) SIGN=1 ;;
    --notarize) NOTARIZE=1; SIGN=1 ;;
    *) echo "unknown flag $arg" >&2; exit 1 ;;
  esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP_NAME="Loupe"
BUNDLE_ID="app.loupe.gui"
DIST="$PWD/dist"
APP="$DIST/$APP_NAME.app"
PKG="$DIST/$APP_NAME-$VERSION.pkg"

echo "building $APP_NAME $VERSION"
cargo build --release --features gui

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/release/loupe" "$APP/Contents/MacOS/loupe"
chmod +x "$APP/Contents/MacOS/loupe"
if [[ -f scripts/AppIcon.icns ]]; then
  cp scripts/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

cat > "$APP/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Loupe</string>
    <key>CFBundleDisplayName</key><string>Loupe</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleExecutable</key><string>loupe</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>NSHumanReadableCopyright</key><string>Copyright © 2026 Mayank Lavania</string>
</dict>
</plist>
EOF

if [[ "$SIGN" -eq 1 ]]; then
  : "${IDENTITY_APP:?set IDENTITY_APP to a Developer ID Application identity}"
  codesign --force --options runtime --timestamp \
    --entitlements scripts/Loupe.entitlements \
    --sign "$IDENTITY_APP" \
    "$APP"
  codesign --verify --deep --strict "$APP"
else
  # Ad-hoc signature so the bundle is a valid Mach-O. Gatekeeper still
  # wants a right-click Open until Developer ID + notarization ship.
  codesign --force --sign - --entitlements scripts/Loupe.entitlements "$APP"
fi

rm -f "$PKG"
if [[ "$SIGN" -eq 1 ]]; then
  : "${IDENTITY_PKG:?set IDENTITY_PKG to a Developer ID Installer identity}"
  productbuild --component "$APP" /Applications \
    --sign "$IDENTITY_PKG" \
    "$PKG"
else
  productbuild --component "$APP" /Applications "$PKG"
fi

if [[ "$NOTARIZE" -eq 1 ]]; then
  : "${APPLE_ID:?}" "${APPLE_TEAM_ID:?}" "${APPLE_APP_PASSWORD:?}"
  xcrun notarytool submit "$PKG" \
    --apple-id "$APPLE_ID" \
    --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" \
    --wait
  xcrun stapler staple "$PKG"
  spctl --assess --type install "$PKG"
fi

echo "app $APP"
[[ -f "$PKG" ]] && echo "pkg $PKG"
if [[ "$SIGN" -eq 0 ]]; then
  echo "unsigned: right-click the pkg, choose Open. A signed notarized pkg is later."
fi
