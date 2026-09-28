#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
: "${ANDROID_HOME:?Set ANDROID_HOME to the Android SDK directory}"
export ANDROID_NDK="$ANDROID_HOME/ndk/${NDK_VERSION:-29.0.14206865}"
export ANDROID_NDK_ROOT="$ANDROID_NDK"
nekodash_build_tools="$ANDROID_HOME/build-tools/${ANDROID_BUILD_TOOLS:-35.0.1}"

# cargo-apk 0.10 requires a key even when only its unsigned output is needed.
# Keep this disposable build key separate from the release signing step.
nekodash_temporary=$(mktemp -d)
trap 'rm -rf "$nekodash_temporary"' EXIT
export CARGO_APK_RELEASE_KEYSTORE="$nekodash_temporary/build.p12"
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=temporary-build-key
keytool -genkeypair -keystore "$CARGO_APK_RELEASE_KEYSTORE" -storetype PKCS12 \
  -storepass:env CARGO_APK_RELEASE_KEYSTORE_PASSWORD \
  -keypass:env CARGO_APK_RELEASE_KEYSTORE_PASSWORD \
  -alias build -keyalg RSA -keysize 2048 -validity 2 -dname 'CN=NekoDash Build'

# cargo-apk 0.10 does not forward --locked. Validate and fetch the lockfile first,
# build offline, then verify that packaging left the dependency lock unchanged.
cp Cargo.lock "$nekodash_temporary/Cargo.lock"
cargo +"${RUST_TOOLCHAIN:-1.98.1}" fetch --locked
CARGO_NET_OFFLINE=true cargo +"${RUST_TOOLCHAIN:-1.98.1}" apk build --release -p nekodash --lib
cmp Cargo.lock "$nekodash_temporary/Cargo.lock"
"$nekodash_build_tools/zipalign" -f -P 16 4 \
  target/release/apk/nekodash-unaligned.apk target/release/apk/nekodash-aligned.apk
"$nekodash_build_tools/zipalign" -c -P 16 4 target/release/apk/nekodash-aligned.apk
python3 scripts/check_android_alignment.py target/release/apk/nekodash-aligned.apk
