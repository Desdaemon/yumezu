#!/usr/bin/env bash
# Builds and signs an apk with only the SDK's command line tools: the app is native.
# Wants ANDROID_HOME (or ANDROID_SDK_ROOT) with platforms, build-tools and an ndk installed.
set -euo pipefail

readonly ABI=arm64-v8a
readonly TRIPLE=aarch64-linux-android
# Also the manifest's minSdkVersion.
readonly MIN_SDK=24

cd "$(dirname "$0")/.."
readonly ROOT=$PWD
readonly OUT=$ROOT/target/android

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}
[[ -d $sdk ]] || { echo "no Android SDK at $sdk; set ANDROID_HOME" >&2; exit 1; }

newest() { ls "$1" 2>/dev/null | sort -V | tail -1; }
readonly TOOLS=$sdk/build-tools/$(newest "$sdk/build-tools")
readonly NDK=$sdk/ndk/$(newest "$sdk/ndk")
readonly PLATFORM=$sdk/platforms/$(newest "$sdk/platforms")
for dir in "$TOOLS" "$NDK" "$PLATFORM"; do
    [[ -d $dir ]] || { echo "missing ${dir%/*} in $sdk; install it with sdkmanager" >&2; exit 1; }
done
readonly BIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin

# The api level goes in clang's name, not a flag.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$BIN/$TRIPLE$MIN_SDK-clang
export CC_aarch64_linux_android=$BIN/$TRIPLE$MIN_SDK-clang
export AR_aarch64_linux_android=$BIN/llvm-ar
cargo build --release --lib --target "$TRIPLE"

# The staging tree's layout is the apk's: `lib/<abi>` holds the library the manifest names.
rm -rf "$OUT/staging"
mkdir -p "$OUT/staging/lib/$ABI"
# `target/` keeps the unstripped copy for symbolising logcat crashes.
"$BIN/llvm-strip" -o "$OUT/staging/lib/$ABI/libyumezu.so" \
    "$ROOT/target/$TRIPLE/release/libyumezu.so"
# `link` takes compiled resources only.
"$TOOLS/aapt2" compile --dir "$ROOT/android/res" -o "$OUT/res.zip"

"$TOOLS/aapt2" link \
    -o "$OUT/unaligned.apk" \
    -I "$PLATFORM/android.jar" \
    --manifest "$ROOT/android/AndroidManifest.xml" \
    --min-sdk-version "$MIN_SDK" \
    --target-sdk-version 34 \
    "$OUT/res.zip"

# Stored: `extractNativeLibs="false"` maps the library straight out of the apk.
(cd "$OUT/staging" && zip -q -X -Z store "$OUT/unaligned.apk" "lib/$ABI/libyumezu.so")
"$TOOLS/zipalign" -f -p 4 "$OUT/unaligned.apk" "$OUT/yumezu.apk"

# The gitignored release key; mint it once and back it up, as only it can upgrade an installed apk:
#   keytool -genkeypair -v -keystore android/production.jks -alias yumezu \
#       -keyalg RSA -keysize 4096 -validity 10000
# Without it, the build signs with the SDK debug key.
readonly RELEASE_KEY=${YUMEZU_KEYSTORE:-$ROOT/android/production.jks}
if [[ -f $RELEASE_KEY ]]; then
    pass=()
    [[ -z ${YUMEZU_KEYSTORE_PASS:-} ]] || pass=(--ks-pass "pass:$YUMEZU_KEYSTORE_PASS")
    "$TOOLS/apksigner" sign --ks "$RELEASE_KEY" \
        --ks-key-alias "${YUMEZU_KEY_ALIAS:-yumezu}" "${pass[@]}" "$OUT/yumezu.apk"
else
    echo "WARNING: no RELEASE_KEY; signing with the debug key" >&2
    readonly KEYSTORE=$HOME/.android/debug.keystore
    if [[ ! -f $KEYSTORE ]]; then
        mkdir -p "$(dirname "$KEYSTORE")"
        keytool -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android \
            -alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" \
            -keyalg RSA -keysize 2048 -validity 10000
    fi
    "$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android \
        --ks-key-alias androiddebugkey "$OUT/yumezu.apk"
fi

echo "$OUT/yumezu.apk"
