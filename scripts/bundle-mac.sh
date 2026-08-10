#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

ICON="${ICON:-assets/icon.png}"
ICONSET_SRC="${ICONSET:-assets/icon.iconset}"
BUNDLE_ID="${BUNDLE_ID:-com.luisffm.helix}"
APP="target/Helix.app"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"

if [ ! -d "$ICONSET_SRC" ] && [ ! -f "$ICON" ]; then
  echo "bundle-mac: no icon at $ICONSET_SRC or $ICON" >&2
  echo "            drop a square 1024x1024 png there, or set ICON=path/to/icon.png" >&2
  exit 1
fi

if [ ! -d "$ICONSET_SRC" ]; then
  read -r WIDTH HEIGHT <<<"$(sips -g pixelWidth -g pixelHeight "$ICON" | sed -n 's/.*: \([0-9]*\)$/\1/p' | paste -sd' ' -)"
  if [ "$WIDTH" != "$HEIGHT" ]; then
    echo "bundle-mac: icon is ${WIDTH}x${HEIGHT}, macOS wants a square" >&2
    exit 1
  fi
  if [ "$WIDTH" -lt 1024 ]; then
    echo "bundle-mac: icon is ${WIDTH}px, upscaling to 1024 will look soft" >&2
  fi
fi

cargo build --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

STAGE="$(mktemp -d)"
ICONSET="$STAGE/helix.iconset"
mkdir -p "$ICONSET"
if [ -d "$ICONSET_SRC" ]; then
  cp "$ICONSET_SRC"/*.png "$ICONSET/"
else
  for SIZE in 16 32 128 256 512; do
    sips -z "$SIZE" "$SIZE" "$ICON" --out "$ICONSET/icon_${SIZE}x${SIZE}.png" >/dev/null
    RETINA=$((SIZE * 2))
    sips -z "$RETINA" "$RETINA" "$ICON" --out "$ICONSET/icon_${SIZE}x${SIZE}@2x.png" >/dev/null
  done
fi
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/helix.icns"
rm -rf "$STAGE"

cp target/release/helix "$APP/Contents/MacOS/helix"

cat >"$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Helix</string>
  <key>CFBundleDisplayName</key>
  <string>Helix</string>
  <key>CFBundleExecutable</key>
  <string>helix</string>
  <key>CFBundleIdentifier</key>
  <string>${BUNDLE_ID}</string>
  <key>CFBundleIconFile</key>
  <string>helix</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${VERSION}</string>
  <key>CFBundleVersion</key>
  <string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

plutil -lint "$APP/Contents/Info.plist" >/dev/null
codesign --force --sign - "$APP" >/dev/null 2>&1 || echo "bundle-mac: ad-hoc signing failed, bundle stays unsigned" >&2
touch "$APP"

echo "built $APP ($VERSION, $BUNDLE_ID)"
echo "run: open $APP --args \$PWD"
