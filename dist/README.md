# Packaging

## Linux (implemented)

```sh
scripts/package-linux.sh            # release build (thin LTO, stripped)
PROFILE=debug scripts/package-linux.sh   # fast smoke package
```

Produces `target/package/helix-<version>-linux-<arch>.tar.gz` containing:

- `helix` — the binary (headed by default; `helix headless` runs the engine alone)
- `helix.desktop` — XDG desktop entry
- `helix.png` — 1024×1024 Helix app icon
- `install.sh` — installs into `~/.local/{bin,share/applications,share/icons}`

The release profile in the root `Cargo.toml` sets `lto = "thin"` and
`strip = "symbols"` for distribution builds.

## macOS

```sh
scripts/package-macos.sh    # → target/package/helix-<version>-macos-<arch>.dmg
```

Builds the release binary, assembles `Helix.app` (Info.plist + icns), ad-hoc
signs it (set `CODESIGN_IDENTITY` for a real Developer ID), and wraps it in a
dmg. The auto-update tarball retains an internal `Helix.app` path so older
installed builds can update into Helix. CI runs this on tags
(`.github/workflows/release.yml`). The manual steps it automates, for reference
(run on a macOS host — gpui needs Metal; no cross-build from Linux):

1. Build the universal (or per-arch) binary:
   ```sh
   cargo build --release -p helix --target aarch64-apple-darwin
   cargo build --release -p helix --target x86_64-apple-darwin
   lipo -create -output helix \
     target/aarch64-apple-darwin/release/helix \
     target/x86_64-apple-darwin/release/helix
   ```
2. Assemble the bundle:
   ```sh
   mkdir -p Helix.app/Contents/{MacOS,Resources}
   cp helix Helix.app/Contents/MacOS/helix
   sed "s/__VERSION__/$(grep -m1 '^version' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')/" \
     dist/macos/Info.plist > Helix.app/Contents/Info.plist
   ```
3. Icon: generate `helix.icns` from `dist/helix.png` (`iconutil`) and place it at
   `Helix.app/Contents/Resources/helix.icns`:
   ```sh
   mkdir helix.iconset && sips -z 256 256 dist/helix.png --out helix.iconset/icon_256x256.png
   iconutil -c icns helix.iconset -o Helix.app/Contents/Resources/helix.icns
   ```
4. Sign + notarize (required for distribution):
   ```sh
   codesign --deep --force --options runtime --sign "Developer ID Application: …" Helix.app
   xcrun notarytool submit Helix.zip --keychain-profile … --wait
   xcrun stapler staple Helix.app
   ```
5. Ship as a `.dmg` (`hdiutil create -volname Helix -srcfolder Helix.app -ov -format UDZO Helix.dmg`).
