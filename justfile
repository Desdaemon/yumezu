serve:
    trunk serve --release

# Downloads every world's image from the wiki and packs them into `static/thumbnails.jpg`, which
# the app samples for the node thumbnails. Downloads are cached under `tools/atlas/cache`, so
# re-running this after the first time costs only the packing.
thumbnails:
    cargo run --release --manifest-path tools/atlas/Cargo.toml

# Builds a signed apk at `target/android/yumezu.apk`. See `android/build.sh` for what it needs.
apk:
    android/build.sh

# Installs that apk on the device adb is talking to.
install: apk
    adb install -r target/android/yumezu.apk
