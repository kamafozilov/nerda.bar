#!/usr/bin/env bash
# Packs the version in Cargo.toml as dist/NerdaBar.dmg, the app beside a link to Applications.
#   scripts/release.sh             build the disk image only
#   scripts/release.sh --publish   also tag the commit and publish it as a GitHub release
set -euo pipefail

cd "$(dirname "$0")/.."
NAME="Nerda Bar"
REPO="kamafozilov/nerda.bar"
# Every release is signed with this one certificate, for good: macOS keeps Launch at Login of
# an installed copy only for an update signed the same way.
RELEASE_IDENTITY="Nerda Bar Local Signing"
RELEASE_REQUIREMENT='identifier "nerda.bar" and certificate leaf = H"2b16970a149b19006ccb640640e5cb7f0d1866b0"'
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
TAG="v$VERSION"
DMG="dist/NerdaBar.dmg"
PUBLISH=false
[[ "${1:-}" == "--publish" ]] && PUBLISH=true

if $PUBLISH; then
  [[ -z "$(git status --porcelain)" ]] || { echo "Commit your changes first: a release is built from a commit." >&2; exit 1; }
  if gh release view "$TAG" --repo "$REPO" >/dev/null 2>&1; then
    echo "$TAG is already published. Raise the version in Cargo.toml and commit it." >&2; exit 1
  fi
  cargo test --release
fi

SIGN_IDENTITY="$RELEASE_IDENTITY" scripts/bundle.sh
APP="dist/$NAME.app"
codesign --verify --strict -R="$RELEASE_REQUIREMENT" "$APP" \
  || { echo "The build isn't signed with the release certificate ($RELEASE_IDENTITY)." >&2; exit 1; }

STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/$NAME.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -quiet -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"
echo "Packed $DMG ($(du -h "$DMG" | cut -f1 | tr -d ' '))"

$PUBLISH || exit 0

# With a message: a git set up to sign tags refuses one without it.
git tag -a "$TAG" -m "$NAME $VERSION"
git push origin "$TAG"
gh release create "$TAG" "$DMG" --repo "$REPO" --title "$NAME $VERSION" --generate-notes --latest
echo "Published https://github.com/$REPO/releases/tag/$TAG"
