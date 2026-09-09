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

# Builds the server for an Ubuntu 24.04 host, at `target/x86_64-unknown-linux-gnu/release/`.
#
# The binary carries its own TLS but not the roots it checks against, so the server needs
# `ca-certificates` installed -- without it the process panics as it starts. See `src/fetch.rs`
# for the one platform where those roots are compiled in instead.
dreamweaver-release:
    RUSTFLAGS="-C target-cpu=x86-64" cross build --release --locked -p dreamweaver --target x86_64-unknown-linux-gnu
    strip target/x86_64-unknown-linux-gnu/release/dreamweaver
    @ls -lh target/x86_64-unknown-linux-gnu/release/dreamweaver

test: && test-wasm test-browser test-arm
    cargo nextest run --workspace

# Runs the force-graph behaviour tests compiled to wasm with SIMD on,
test-wasm:
    cargo test -p force_graph_3d --lib --target wasm32-wasip2

# Runs the app's own tests on the target that ships, in a real browser and with the same SIMD the
# page gets. Headless through chromedriver where there is one; without it the suite is served
# instead and the URL it prints opens in any browser. `NO_HEADLESS=1 just test-browser` forces the
# second even when chromedriver is installed.
test-browser:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${NO_HEADLESS:-}" ] && command -v chromedriver >/dev/null; then
        cargo test -p yumezu --lib --target wasm32-unknown-unknown
    else
        NO_HEADLESS=1 cargo test -p yumezu --lib --target wasm32-unknown-unknown
    fi

# Runs the force-graph behaviour tests on armv8, under qemu in cross's image.
test-arm:
    cross test -p force_graph_3d --lib --target aarch64-unknown-linux-gnu

# Times each of the kernel's load-bearing spellings against the alternative it was chosen over, on
# this machine and on a phone if one is attached. The only measurement of them that is not a
# scheduling model's guess, and the model has nothing at all for the core the clamp turned out to
# matter most on. Args are `<nodes> <theta> <steps>`; the phone half wants adb and the SDK
# `just apk` wants.
bench *args:
    tools/bench.sh {{args}}

# The thumbnail atlas and the placeholder, taken from the deployed page rather than packed again:
# `just thumbnails` downloads every world's image from the wiki, and these are the two files that
# run produced. Neither is committed -- they are the wiki's to distribute, see `.gitignore` -- so a
# fresh clone and a CI runner both start without them.
#
# A package built without them has no world pictures at all, and `cargo packager` refuses to build
# one, so this is not optional the way `android/build.sh`'s own copy is.
assets:
    #!/usr/bin/env bash
    set -euo pipefail
    for name in thumbnails.jpg unknown_location.png; do
        [ -f "static/$name" ] \
            || curl -fsSL --retry 3 -o "static/$name" "https://explorer.yumemiru.dev/static/$name"
    done

# Builds this host's desktop package into `target/packages`: an AppImage on Linux, an NSIS
# installer on Windows. `cargo packager` only packages, so the binary is built first.
#
# Unsigned unless CARGO_PACKAGER_SIGN_PRIVATE_KEY is set, and an unsigned package is one no
# installed copy will take as an update -- see `src/update.rs`. Signed or not, a package built
# without YUMEZU_UPDATE_PUBKEY has no update controls to press.
#
# NO_STRIP is linuxdeploy's: its own strip cannot read the `.relr.dyn` section that every library a
# current toolchain built has, and it treats that failure as fatal.
package: assets
    cargo build --release --locked --bin yumezu_main --features production
    NO_STRIP=1 APPIMAGE_EXTRACT_AND_RUN=1 cargo packager --release
