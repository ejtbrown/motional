#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"

cargo build --release --locked --bin motional-gui
version="$(cargo pkgid | sed 's/.*@//')"

app_dir="${1:-target/release/bundle/macos/Motional.app}"
contents_dir="$app_dir/Contents"
macos_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"

rm -rf "$app_dir"
mkdir -p "$macos_dir" "$resources_dir"

cp target/release/motional-gui "$macos_dir/Motional"
cp assets/motional-icon.icns "$resources_dir/Motional.icns"

cat > "$contents_dir/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Motional</string>
  <key>CFBundleExecutable</key>
  <string>Motional</string>
  <key>CFBundleIconFile</key>
  <string>Motional</string>
  <key>CFBundleIdentifier</key>
  <string>com.ejtbrown.motional</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Motional</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>${version}</string>
  <key>CFBundleVersion</key>
  <string>${version}</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.utilities</string>
  <key>LSMinimumSystemVersion</key>
  <string>11.0</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
PLIST

printf 'APPL????' > "$contents_dir/PkgInfo"

echo "$app_dir"
