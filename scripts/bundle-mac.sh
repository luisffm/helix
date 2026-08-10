#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

ICON="${ICON:-assets/icon.png}"
ICONSET_SRC="${ICONSET:-assets/icon.iconset}"
BUNDLE_ID="${BUNDLE_ID:-com.luisffm.helix}"
PROFILE="${PROFILE:-release}"
SIGN="${SIGN:-1}"
BUILD="${BUILD:-1}"
APP="target/Helix.app"
ICNS="$APP/Contents/Resources/helix.icns"
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"

if [ "$PROFILE" = "debug" ]; then
  BUILD_FLAGS=""
  BUILD_HINT="cargo build"
  PROFILE_DIR="debug"
else
  BUILD_FLAGS="--release"
  BUILD_HINT="cargo build --release"
  PROFILE_DIR="release"
fi

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

if [ "$BUILD" = "1" ]; then
  cargo build $BUILD_FLAGS
fi

BIN="target/$PROFILE_DIR/helix"
if [ ! -x "$BIN" ]; then
  echo "bundle-mac: no binary at $BIN — run '$BUILD_HINT' first" >&2
  exit 1
fi

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [ ! -f "$ICNS" ] || [ "$ICONSET_SRC" -nt "$ICNS" ] || [ "$ICON" -nt "$ICNS" ]; then
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
  iconutil -c icns "$ICONSET" -o "$ICNS"
  rm -rf "$STAGE"
fi

cp "$BIN" "$APP/Contents/MacOS/helix"

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
if [ "$SIGN" = "1" ]; then
  codesign --force --sign - "$APP" >/dev/null 2>&1 || echo "bundle-mac: ad-hoc signing failed, bundle stays unsigned" >&2
fi
touch "$APP"

echo "built $APP ($PROFILE, $VERSION, $BUNDLE_ID)"
