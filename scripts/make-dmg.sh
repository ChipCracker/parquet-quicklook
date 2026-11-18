#!/bin/bash
# Builds ParquetPreview.app (universal, release) and packages it as a DMG in dist/.
#
# Optional, for distribution without a Gatekeeper warning (requires the Apple Developer Program):
#   DEVELOPER_ID="Developer ID Application: Name (TEAMID)"  → signs with it instead of ad-hoc signing
#   NOTARY_PROFILE="<profile>"  → notarizes + staples (create the profile first with
#                                 `xcrun notarytool store-credentials <profile>`)
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(grep -m1 'MARKETING_VERSION' project.yml | sed 's/.*"\(.*\)".*/\1/')
DD=build/dd-dist
APP=$DD/Build/Products/Release/ParquetPreview.app
OUT=dist/ParquetPreview-$VERSION.dmg

xcodegen generate >/dev/null
SIGN_ARGS=()
if [[ -n "${DEVELOPER_ID:-}" ]]; then
  SIGN_ARGS=(CODE_SIGN_IDENTITY="$DEVELOPER_ID" OTHER_CODE_SIGN_FLAGS="--timestamp")
fi
xcodebuild -project ParquetPreview.xcodeproj -scheme ParquetPreview -configuration Release \
  -derivedDataPath "$DD" -destination "generic/platform=macOS" ARCHS="arm64 x86_64" ONLY_ACTIVE_ARCH=NO ${SIGN_ARGS[@]+"${SIGN_ARGS[@]}"} clean build \
  | grep -E "error:|BUILD SUCCEEDED|BUILD FAILED" || true
[[ -d "$APP" ]] || { echo "Build failed"; exit 1; }
codesign --verify --deep --strict "$APP"
lipo -info "$APP/Contents/PlugIns/ParquetQuickLook.appex/Contents/MacOS/ParquetQuickLook"

STAGE=$(mktemp -d)
ditto "$APP" "$STAGE/ParquetPreview.app"
ln -s /Applications "$STAGE/Applications"
cp scripts/dmg-README.txt "$STAGE/README.txt"
mkdir -p dist && rm -f "$OUT"
hdiutil create -volname "Parquet Preview" -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$OUT" >/dev/null
rm -rf "$STAGE"

if [[ -n "${DEVELOPER_ID:-}" ]]; then
  codesign --sign "$DEVELOPER_ID" --timestamp "$OUT"
  if [[ -n "${NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$OUT" --keychain-profile "$NOTARY_PROFILE" --wait
    xcrun stapler staple "$OUT"
  fi
fi
echo "→ $OUT ($(du -h "$OUT" | cut -f1))"
