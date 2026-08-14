#!/usr/bin/env bash
# macOS packaging: build the release binary for the host arch and produce
#   target/package/helix-<version>-macos-<arch>.dmg          (user download)
#   target/package/helix-<version>-macos-<arch>-app.tar.gz   (auto-updater)
# containing Helix.app (unsigned unless CODESIGN_IDENTITY is set).
#
# Usage: scripts/package-macos.sh
# Env:   CODESIGN_IDENTITY="Developer ID Application: …" to sign the bundle.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v cargo >/dev/null 2>&1 || PATH="$HOME/.cargo/bin:$PATH"
VERSION="$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')"
ARCH="$(uname -m)" # arm64 on Apple silicon runners
OUT_DIR="$ROOT/target/package"
APP="$OUT_DIR/Helix.app"
DMG="$OUT_DIR/helix-$VERSION-macos-$ARCH.dmg"
APP_TARBALL="$OUT_DIR/helix-$VERSION-macos-$ARCH-app.tar.gz"

cd "$ROOT"
cargo build --release -p helix

rm -rf "$APP" "$DMG" "$APP_TARBALL"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m 755 "$ROOT/target/release/helix" "$APP/Contents/MacOS/helix"
sed "s/__VERSION__/$VERSION/" "$ROOT/dist/macos/Info.plist" >"$APP/Contents/Info.plist"

# Icon: iconset from the shared 1024×1024 Helix artwork in dist/helix.png.
ICONSET="$OUT_DIR/helix.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$ROOT/dist/helix.png" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  retina=$((size * 2))
  sips -z "$retina" "$retina" "$ROOT/dist/helix.png" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/helix.icns"
rm -rf "$ICONSET"

if [[ -n "${CODESIGN_IDENTITY:-}" ]]; then
  codesign --deep --force --options runtime --sign "$CODESIGN_IDENTITY" "$APP"
else
  # Ad-hoc signature so the app launches on Apple silicon (Gatekeeper still
  # requires right-click → Open on first launch without notarization).
  codesign --deep --force --sign - "$APP"
fi

# The auto-updater artifact: keep the historical Helix.app path inside the
# tarball so already-installed Helix builds can consume the first Helix
# release. The DMG still presents Helix.app to new installs.
tar -czf "$APP_TARBALL" -s '/^Helix\.app/Helix.app/' -C "$OUT_DIR" Helix.app
echo "packaged: $APP_TARBALL"

hdiutil create -volname Helix -srcfolder "$APP" -ov -format UDZO "$DMG"
echo "packaged: $DMG"
