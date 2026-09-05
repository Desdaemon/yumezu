# Serves the page. `just dreamweaver` has to be running alongside it: the page asks its own host
# for the world dump, and `Trunk.toml` puts that request through to the server.
serve:
    trunk serve --release

dist *args:
    trunk build --cargo-profile=min --features=production {{args}}

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

# Serves `data.json`, building it from the wiki and keeping it current. See `crates/dreamweaver`.
dreamweaver:
    cargo run --release -p dreamweaver -- --data data.json
