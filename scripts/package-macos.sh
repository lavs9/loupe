#!/bin/bash
# Build Loupe.app and a drag-to-Applications .dmg.
#
#   ./scripts/package-macos.sh
#       unsigned Apple Silicon dmg
#   ./scripts/package-macos.sh --pkg
#       also write an unsigned .pkg (silent install into /Applications)
#   IDENTITY_APP="Developer ID Application: Name (TEAM)" \
#   APPLE_ID=... APPLE_TEAM_ID=... APPLE_APP_PASSWORD=... \
#   ./scripts/package-macos.sh --sign --notarize
set -euo pipefail
cd "$(dirname "$0")/.."
export COPYFILE_DISABLE=1

SIGN=0
NOTARIZE=0
WANT_PKG=0
for arg in "$@"; do
  case "$arg" in
    --sign) SIGN=1 ;;
    --notarize) NOTARIZE=1; SIGN=1 ;;
    --pkg) WANT_PKG=1 ;;
    *) echo "unknown flag $arg" >&2; exit 1 ;;
  esac
done

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP_NAME="Loupe"
BUNDLE_ID="app.loupe.gui"
DIST="$PWD/dist"
APP="$DIST/$APP_NAME.app"
DMG="$DIST/$APP_NAME-$VERSION.dmg"
STABLE_DMG="$DIST/$APP_NAME.dmg"
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
  codesign --force --sign - --entitlements scripts/Loupe.entitlements "$APP"
fi

STAGE=$(mktemp -d /tmp/loupe-dmg.XXXXXX)
ditto "$APP" "$STAGE/$APP_NAME.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create \
  -volname "Install Loupe" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDZO \
  -imagekey zlib-level=9 \
  "$DMG"
cp -f "$DMG" "$STABLE_DMG"
rm -rf "$STAGE"

if [[ "$WANT_PKG" -eq 1 ]]; then
  rm -f "$PKG"
  if [[ "$SIGN" -eq 1 ]]; then
    : "${IDENTITY_PKG:?set IDENTITY_PKG to a Developer ID Installer identity}"
    productbuild --component "$APP" /Applications --sign "$IDENTITY_PKG" "$PKG"
  else
    productbuild --component "$APP" /Applications "$PKG"
  fi
fi

if [[ "$NOTARIZE" -eq 1 ]]; then
  : "${APPLE_ID:?}" "${APPLE_TEAM_ID:?}" "${APPLE_APP_PASSWORD:?}"
  xcrun notarytool submit "$DMG" \
    --apple-id "$APPLE_ID" \
    --team-id "$APPLE_TEAM_ID" \
    --password "$APPLE_APP_PASSWORD" \
    --wait
  xcrun stapler staple "$DMG"
fi

echo "app $APP"
echo "dmg $DMG"
echo "dmg-latest $STABLE_DMG"
[[ -f "$PKG" ]] && echo "pkg $PKG"
if [[ "$SIGN" -eq 0 ]]; then
  echo "unsigned: open the dmg, drag Loupe to Applications, right-click Open."
fi
