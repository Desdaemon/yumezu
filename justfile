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
# The compile happens in cross's own image rather than here, because this machine's glibc is
# the newer one: a native build asks for symbols the server has no version of, and refuses to
# start there. Nothing in this recipe names the server's release -- cross's image is Ubuntu
# 16.04, whose glibc is older than any live server's, so what it builds runs on all of them.
# Needs `cross` (`cargo install cross`) and a running docker.
#
# The binary carries its own TLS but not the roots it checks against, so the server needs
# `ca-certificates` installed -- without it the process panics as it starts. See `src/fetch.rs`
# for the one platform where those roots are compiled in instead.
#
# `target-cpu=x86-64` is the toolchain's own default for this target, and is written out only so
# that a `RUSTFLAGS` in the environment cannot quietly raise the floor: cross forwards that
# variable into the container. What rustc emits is then SSE2 and nothing later. It says nothing about
# `ring`'s assembly under rustls, which carries AVX and AES paths that it chooses between by
# `cpuid` as it starts, and so never runs on a processor that does not have them.
dreamweaver-release:
    RUSTFLAGS="-C target-cpu=x86-64" cross build --release --locked -p dreamweaver --target x86_64-unknown-linux-gnu
    strip target/x86_64-unknown-linux-gnu/release/dreamweaver
    @ls -lh target/x86_64-unknown-linux-gnu/release/dreamweaver
