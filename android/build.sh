#!/usr/bin/env bash
# Builds and signs an apk out of the crate, using only the Android SDK's own command line tools:
# no Gradle, no Android Studio, and no Java source. That is possible because the app is entirely
# native -- see `AndroidManifest.xml` -- so the apk is a zip of one shared library, the assets it
# reads, and a compiled manifest, which `aapt2`, `zipalign` and `apksigner` are enough to make.
#
# Wants ANDROID_HOME (or ANDROID_SDK_ROOT) pointing at an SDK that has platforms, build-tools and
# an ndk installed. Everything it writes goes under `target/android`.
set -euo pipefail

readonly ABI=arm64-v8a
readonly TRIPLE=aarch64-linux-android
# The oldest Android this runs on, and so the version of the platform library the code is linked
# against. Also what the manifest declares, so the two cannot drift apart.
readonly MIN_SDK=24

cd "$(dirname "$0")/.."
readonly ROOT=$PWD
readonly OUT=$ROOT/target/android

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}
[[ -d $sdk ]] || { echo "no Android SDK at $sdk; set ANDROID_HOME" >&2; exit 1; }

# Newest of whatever is installed, rather than a version pinned here that an SDK may not have.
newest() { ls "$1" 2>/dev/null | sort -V | tail -1; }
readonly TOOLS=$sdk/build-tools/$(newest "$sdk/build-tools")
readonly NDK=$sdk/ndk/$(newest "$sdk/ndk")
readonly PLATFORM=$sdk/platforms/$(newest "$sdk/platforms")
for dir in "$TOOLS" "$NDK" "$PLATFORM"; do
    [[ -d $dir ]] || { echo "missing ${dir%/*} in $sdk; install it with sdkmanager" >&2; exit 1; }
done
readonly BIN=$NDK/toolchains/llvm/prebuilt/linux-x86_64/bin

# The ndk's clang is both the compiler for the C that rustls' crypto is written in and the linker
# for the whole library, and it has to be told the api level in its own name rather than a flag.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$BIN/$TRIPLE$MIN_SDK-clang
export CC_aarch64_linux_android=$BIN/$TRIPLE$MIN_SDK-clang
export AR_aarch64_linux_android=$BIN/llvm-ar
cargo build --release --lib --target "$TRIPLE"

# The apk is assembled in a staging tree whose layout *is* the apk's: `lib/<abi>` is where the
# framework looks for the library named in the manifest, and `assets` is what the AssetManager in
# `src/thumbnails.rs` reads through.
rm -rf "$OUT/staging"
mkdir -p "$OUT/staging/lib/$ABI" "$OUT/staging/assets/static"
# Stripped on the way in: the debug symbols are two thirds of the library and nothing on a phone
# reads them. `target/` keeps the unstripped copy for anyone symbolising a crash out of logcat.
"$BIN/llvm-strip" -o "$OUT/staging/lib/$ABI/libyumezu.so" \
    "$ROOT/target/$TRIPLE/release/libyumezu.so"
# Absent until `just thumbnails` has been run, and the app draws the graph without pictures then.
cp "$ROOT/static/thumbnails.jpg" "$OUT/staging/assets/static/" 2>/dev/null \
    || echo "no static/thumbnails.jpg; the apk will have no world pictures" >&2

# There are no resources to compile, so linking is only the manifest plus the assets beside it.
"$TOOLS/aapt2" link \
    -o "$OUT/unaligned.apk" \
    -I "$PLATFORM/android.jar" \
    --manifest "$ROOT/android/AndroidManifest.xml" \
    -A "$OUT/staging/assets" \
    --min-sdk-version "$MIN_SDK" \
    --target-sdk-version 34

# Stored rather than deflated, because the manifest says `extractNativeLibs="false"`: the loader
# maps the library straight out of the apk, which it can only do if it is uncompressed and aligned.
(cd "$OUT/staging" && zip -q -X -Z store "$OUT/unaligned.apk" "lib/$ABI/libyumezu.so")
"$TOOLS/zipalign" -f -p 4 "$OUT/unaligned.apk" "$OUT/yumezu.apk"

# The same throwaway key the SDK's own tooling signs debug builds with. Enough to install; not
# enough to publish.
readonly KEYSTORE=$HOME/.android/debug.keystore
if [[ ! -f $KEYSTORE ]]; then
    mkdir -p "$(dirname "$KEYSTORE")"
    keytool -genkeypair -keystore "$KEYSTORE" -storepass android -keypass android \
        -alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" \
        -keyalg RSA -keysize 2048 -validity 10000
fi
"$TOOLS/apksigner" sign --ks "$KEYSTORE" --ks-pass pass:android --key-pass pass:android \
    --ks-key-alias androiddebugkey "$OUT/yumezu.apk"

echo "$OUT/yumezu.apk"
