#!/bin/bash
set -euo pipefail

desktop=$(cd -- "$(dirname -- "$0")" && pwd)
project=$(dirname -- "$desktop")
cargo_bin=${OMNIRECOVERY_CARGO:-cargo}
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$desktop/Cargo.toml")
export MACOSX_DEPLOYMENT_TARGET=11.0

"$cargo_bin" build --manifest-path "$desktop/Cargo.toml" --locked --release \
  --target x86_64-apple-darwin --target aarch64-apple-darwin
"$cargo_bin" build --manifest-path "$project/Cargo.toml" --locked --release \
  --target x86_64-apple-darwin --target aarch64-apple-darwin

scratch=$(mktemp -d "${TMPDIR:-/tmp}/omnirecovery-desktop.XXXXXX")
app="$scratch/OmniRecovery.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create \
  "$desktop/target/x86_64-apple-darwin/release/omnirecovery-desktop" \
  "$desktop/target/aarch64-apple-darwin/release/omnirecovery-desktop" \
  -output "$app/Contents/MacOS/OmniRecovery"
lipo "$app/Contents/MacOS/OmniRecovery" -verify_arch x86_64 arm64
lipo -create \
  "$project/target/x86_64-apple-darwin/release/omnirecovery" \
  "$project/target/aarch64-apple-darwin/release/omnirecovery" \
  -output "$app/Contents/MacOS/omnirecovery-cli"
lipo "$app/Contents/MacOS/omnirecovery-cli" -verify_arch x86_64 arm64
cp "$desktop/icons/icon.icns" "$app/Contents/Resources/OmniRecovery.icns"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDevelopmentRegion</key><string>en</string>
<key>CFBundleDisplayName</key><string>OmniRecovery</string>
<key>CFBundleName</key><string>OmniRecovery</string>
<key>CFBundleExecutable</key><string>OmniRecovery</string>
<key>CFBundleIdentifier</key><string>local.omnirecovery.desktop</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleIconFile</key><string>OmniRecovery.icns</string>
<key>LSMinimumSystemVersion</key><string>11.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$app/Contents/MacOS/omnirecovery-cli"
codesign --force --sign - --timestamp=none "$app"
codesign --verify --strict --all-architectures "$app"

volume="$scratch/volume"
mkdir "$volume"
ditto "$app" "$volume/OmniRecovery.app"
ln -s /Applications "$volume/Applications"
dmg="OmniRecovery-$version-macos-universal.dmg"
hdiutil create -volname "OmniRecovery $version" -fs HFS+ -format UDZO -srcfolder "$volume" "$scratch/$dmg"
hdiutil verify "$scratch/$dmg"
mkdir -p "$project/dist"
mv "$scratch/$dmg" "$project/dist/$dmg"
printf 'App bundle for local testing: %s\n' "$app"
printf 'Disk image: %s\n' "$project/dist/$dmg"
