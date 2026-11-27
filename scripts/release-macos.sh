#!/usr/bin/env bash
# Build a universal (Apple Silicon + Intel) Terminal.app, sign it with a Developer ID
# certificate and the hardened runtime, notarize and staple it, then package it as
# target/universal-apple-darwin/release/Terminal.dmg.
set -euo pipefail
cd "$(dirname "$0")/.."

identity=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -1)}
[ -n "$identity" ] || { echo "no Developer ID Application certificate in the keychain; set SIGN_IDENTITY" >&2; exit 1; }
notarize=${NOTARIZE:-1}
notary=(--keychain-profile "${NOTARY_PROFILE:-terminal-rs-notary}")
[ -z "${NOTARY_KEYCHAIN:-}" ] || notary+=(--keychain "$NOTARY_KEYCHAIN")
if [ "$notarize" = 1 ] && ! xcrun notarytool history "${notary[@]}" >/dev/null; then
  echo "no notarytool credentials; create the profile, or set NOTARIZE=0" >&2
  exit 1
fi

version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
repo=https://github.com/ProperProperProper/MOOMOO-OPEND-TradeStation

tmp=$(mktemp -d)
cleanup() {
  if [ -d "$tmp/mnt" ] && [ "$(stat -f %d "$tmp/mnt")" != "$(stat -f %d "$tmp")" ]; then
    hdiutil detach -force "$tmp/mnt" >/dev/null || { echo "left $tmp/rw.dmg mounted at $tmp/mnt" >&2; return; }
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

# Submits $1, waits for Apple's verdict, then staples the ticket to $2.
notarize() {
  local out id
  out=$(xcrun notarytool submit "$1" "${notary[@]}" --wait --output-format json) || true
  if [ "$(plutil -extract status raw -o - - <<<"$out" 2>/dev/null)" != Accepted ]; then
    echo "notarizing $1 failed: $out" >&2
    if id=$(plutil -extract id raw -o - - <<<"$out" 2>/dev/null); then
      xcrun notarytool log "$id" "${notary[@]}" >&2
    fi
    exit 1
  fi
  xcrun stapler staple "$2"
}

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --release --locked --target "$target" -p terminal
done
out=target/universal-apple-darwin/release
mkdir -p "$out"
rm -f "$out"/Terminal-*.zip
lipo -create -output "$out/terminal" \
  target/aarch64-apple-darwin/release/terminal target/x86_64-apple-darwin/release/terminal
scripts/bundle-macos.sh "$out/terminal"
app=$out/Terminal.app

echo "signing as $identity"
codesign --force --options runtime --timestamp --sign "$identity" "$app"
codesign --verify --deep --strict --verbose=2 "$app"
if [ "$notarize" = 1 ]; then
  ditto -c -k --keepParent "$app" "$tmp/Terminal.zip"
  notarize "$tmp/Terminal.zip" "$app"
fi

dmg=$out/Terminal.dmg
stage=$tmp/dmg
mkdir "$stage" "$tmp/mnt"
ditto "$app" "$stage/Terminal.app"
ln -s /Applications "$stage/Applications"
cp app/assets/AppIcon.icns "$stage/.VolumeIcon.icns"
hdiutil create -volname Terminal -srcfolder "$stage" -format UDRW "$tmp/rw.dmg"
hdiutil attach -nobrowse -noautoopen -mountpoint "$tmp/mnt" "$tmp/rw.dmg" >/dev/null
SetFile -a C "$tmp/mnt"
hdiutil detach "$tmp/mnt" >/dev/null
hdiutil convert "$tmp/rw.dmg" -format UDZO -ov -o "$dmg"
codesign --force --timestamp --sign "$identity" "$dmg"
if [ "$notarize" = 1 ]; then
  notarize "$dmg" "$dmg"
  spctl --assess --type execute --verbose=2 "$app"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
fi
echo "built $dmg"

zip=$out/Terminal-$version.zip
ditto -c -k --keepParent "$app" "$zip"
echo "built $zip"
echo "publish with: gh release create v$version $dmg $zip"
