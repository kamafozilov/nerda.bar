#!/usr/bin/env bash
# Builds, bundles and signs dist/Nerda Bar.app.
#   scripts/bundle.sh             build only
#   scripts/bundle.sh --install   also install it as /Applications/Nerda Bar.app and start it
#
# SIGN_IDENTITY names the certificate to sign with. Without it the app is signed with the
# self-signed "Nerda Bar Local Signing" certificate from the login keychain, which is made the
# first time. Never ad hoc: macOS ties Launch at Login to the signature, and an ad hoc build is
# a new app after every rebuild.
set -euo pipefail

cd "$(dirname "$0")/.."
NAME="Nerda Bar"
BUNDLE_ID="nerda.bar"
EXE="nerda-bar"
LOCAL_IDENTITY="Nerda Bar Local Signing"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
BUILD="$(git rev-list --count HEAD 2>/dev/null || echo 1)"
APP="dist/$NAME.app"

# A code signing certificate that is only ever used on this Mac: untrusted, but the same for
# every build, and that is all a login item needs.
make_local_identity() {
  local work
  work="$(mktemp -d)"
  cat > "$work/request.cnf" <<CNF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $LOCAL_IDENTITY
[ext]
basicConstraints = critical, CA:FALSE
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
CNF
  /usr/bin/openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$work/request.cnf" \
    -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
  /usr/bin/openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -name "$LOCAL_IDENTITY" \
    -out "$work/identity.p12" -passout pass:nerda
  security import "$work/identity.p12" -k "$HOME/Library/Keychains/login.keychain-db" -P nerda -T /usr/bin/codesign >/dev/null
  rm -rf "$work"
  echo "Made the signing certificate \"$LOCAL_IDENTITY\" in the login keychain."
}

IDENTITY="${SIGN_IDENTITY:-}"
if [[ -z "$IDENTITY" ]]; then
  security find-identity -p codesigning | grep -q "\"$LOCAL_IDENTITY\"" || make_local_identity
  IDENTITY="$LOCAL_IDENTITY"
fi

cargo build --release

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/release/$EXE" "$APP/Contents/MacOS/$EXE"

# The icon, drawn in assets/AppIcon.svg: sips renders each size, iconutil packs them. The 16
# and 32 point icons (menus, lists) get a tighter viewBox, keeping the whole tile and its shadow.
ICONWORK="$(mktemp -d)"
ICONSET="$ICONWORK/AppIcon.iconset"
mkdir -p "$ICONSET"
sed 's/viewBox="0 0 1024 1024"/viewBox="80 80 864 864"/' assets/AppIcon.svg > "$ICONWORK/AppIconSmall.svg"
for size in 16 32 128 256 512; do
  source=assets/AppIcon.svg
  (( size <= 32 )) && source="$ICONWORK/AppIconSmall.svg"
  sips -s format png -z "$size" "$size" "$source" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
  sips -s format png -z $((size * 2)) $((size * 2)) "$source" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONWORK"

cat > "$APP/Contents/Info.plist" <<EOF2
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$EXE</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF2

codesign --force --sign "$IDENTITY" "$APP"
codesign --verify --strict "$APP"
echo "Built $APP $VERSION ($BUILD), signed with: $IDENTITY"

if [[ "${1:-}" == "--install" ]]; then
  # One fixed path: the login item points at this app.
  DEST="/Applications/$NAME.app"
  pkill -x "$EXE" 2>/dev/null || true
  # Opening it while the old one still quits fails with error -600.
  for _ in $(seq 50); do pgrep -x "$EXE" >/dev/null || break; sleep 0.1; done
  rm -rf "$DEST" "$HOME/Applications/$NAME.app"
  cp -R "$APP" "$DEST"
  # Right after the copy, Launch Services may not know the new bundle yet: try again.
  for _ in 1 2 3 4 5; do open "$DEST" 2>/dev/null && break; sleep 1; done
  sleep 1
  pgrep -x "$EXE" >/dev/null || { echo "$NAME didn't start" >&2; exit 1; }
  echo "Installed and started $DEST"
fi
