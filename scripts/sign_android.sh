#!/usr/bin/env bash
set -euo pipefail
umask 077

cd "$(dirname "$0")/.."
: "${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK directory}"
: "${ANDROID_KEYSTORE_BASE64:?Missing ANDROID_KEYSTORE_BASE64}"
: "${ANDROID_KEYSTORE_PASSWORD:?Missing ANDROID_KEYSTORE_PASSWORD}"
: "${ANDROID_KEY_ALIAS:?Missing ANDROID_KEY_ALIAS}"
: "${ANDROID_KEY_PASSWORD:?Missing ANDROID_KEY_PASSWORD}"
nekodash_build_tools="$ANDROID_HOME/build-tools/${ANDROID_BUILD_TOOLS:-35.0.1}"
nekodash_temporary=$(mktemp -d)
trap 'rm -rf "$nekodash_temporary"' EXIT
printf '%s' "$ANDROID_KEYSTORE_BASE64" | base64 --decode > "$nekodash_temporary/release.keystore"
unset ANDROID_KEYSTORE_BASE64

mkdir -p dist
nekodash_apk=dist/NekoDash-android-arm64-v8a.apk
"$nekodash_build_tools/apksigner" sign \
  --ks "$nekodash_temporary/release.keystore" \
  --ks-key-alias "$ANDROID_KEY_ALIAS" \
  --ks-pass env:ANDROID_KEYSTORE_PASSWORD \
  --key-pass env:ANDROID_KEY_PASSWORD \
  --v4-signing-enabled false \
  --out "$nekodash_temporary/signed.apk" target/release/apk/nekodash-aligned.apk
"$nekodash_build_tools/apksigner" verify --verbose --print-certs "$nekodash_temporary/signed.apk"
"$nekodash_build_tools/zipalign" -c -P 16 4 "$nekodash_temporary/signed.apk"
cp "$nekodash_temporary/signed.apk" "$nekodash_apk"
