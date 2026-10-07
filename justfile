# Serves the page; needs `just dreamweaver` running alongside.
serve: assets
    trunk serve --release

dist *args: assets
    trunk build --cargo-profile=min {{args}}

# Packs every world's wiki image into `static/thumbnails.jpg`, caching in `tools/atlas/cache`.
thumbnails:
    cargo run --release --manifest-path tools/atlas/Cargo.toml

# Builds a signed apk at `target/android/yumezu.apk`.
apk:
    android/build.sh

# Installs the apk over adb.
install: apk
    adb install -r target/android/yumezu.apk

# Serves `data.json`, building it from the wiki and keeping it current.
dreamweaver:
    cargo run --release -p dreamweaver -- --data data.json

# Builds the server for Ubuntu 24.04; the host needs `ca-certificates` or it panics at startup.
dreamweaver-release:
    RUSTFLAGS="-C target-cpu=x86-64" cross build --release --locked -p dreamweaver --target x86_64-unknown-linux-gnu
    strip target/x86_64-unknown-linux-gnu/release/dreamweaver
    @ls -lh target/x86_64-unknown-linux-gnu/release/dreamweaver

test: && test-wasm test-browser test-arm
    cargo nextest run --workspace

# Runs the force-graph behaviour tests on wasm with SIMD on.
test-wasm:
    cargo test -p force_graph_3d --lib --target wasm32-wasip2

# Runs the app's tests in a browser, headless if chromedriver exists; `NO_HEADLESS=1` serves them.
test-browser:
    #!/usr/bin/env bash
    set -euo pipefail
    if [ -z "${NO_HEADLESS:-}" ] && command -v chromedriver >/dev/null; then
        cargo test -p yumezu --lib --target wasm32-unknown-unknown
    else
        NO_HEADLESS=1 cargo test -p yumezu --lib --target wasm32-unknown-unknown
    fi

# Runs the force-graph behaviour tests on armv8 under qemu.
test-arm:
    cross test -p force_graph_3d --lib --target aarch64-unknown-linux-gnu

# Times the kernel's variants here and on an adb phone. Args: `<nodes> <theta> <steps>`.
bench *args:
    tools/bench.sh {{args}}

# Fetches the uncommitted thumbnail atlas and placeholder from the deployed page.
assets:
    #!/usr/bin/env bash
    set -euo pipefail
    for name in thumbnails.jpg unknown_location.png; do
        [ -f "static/$name" ] \
            || curl -fsSL --retry 3 -o "static/$name" "https://explorer.yumemiru.dev/static/$name"
    done

# NO_STRIP: linuxdeploy's strip cannot read `.relr.dyn` and fails fatally.
# Unsigned without CARGO_PACKAGER_SIGN_PRIVATE_KEY; no update controls without YUMEZU_UPDATE_PUBKEY.
# Builds this host's desktop package into `target/packages`.
package:
    #!/usr/bin/env bash
    set -euo pipefail
    case "$(uname -s)" in
        Linux) formats=appimage ;;
        Darwin) formats=app,dmg ;;
        *) formats=nsis ;;
    esac
    cargo build --release --locked --bin yumezu_main
    NO_STRIP=1 APPIMAGE_EXTRACT_AND_RUN=1 cargo packager --release --formats "$formats"
