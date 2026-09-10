# yumezu

<p align="center">
  <image src="icons/icon.png" alt="yumezu icon" width="256">
</p>

## Install

Check [Releases](https://github.com/Desdaemon/yumezu/releases) for the latest version.
Available for Linux, Windows, macOS and Android, or visit <https://explorer.yumemiru.dev>.

### MacOS

Newer versions of MacOS outright reject unsigned binaries, which includes yumezu.
Once downloaded and installed, remove the quarantine flag with:

```bash
xattr -dr com.apple.quarantine /Applications/yumezu.app
```

### From source

Needs [`just`](https://github.com/casey/just) and, on Linux, fontconfig's headers.

```bash
sudo apt install libfontconfig1-dev   # or fontconfig-devel, or fontconfig
cargo install --git https://github.com/Desdaemon/yumezu.git --locked yumezu
```

`just test-wasm` additionally needs [`wasmtime`](https://wasmtime.dev) on PATH and the
`wasm32-wasip2` target; `just test-arm` needs [`cross`](https://github.com/cross-rs/cross) and a
running docker; `just apk` and the phone half of `just bench` need `ANDROID_HOME` and a device on
adb; `just dist` needs [`trunk`](https://trunkrs.dev) and `wasm32-unknown-unknown`.

## Licence

The code is MIT OR Apache-2.0, except `crates/force_graph_3d` and `crates/dreamweaver`, which are
MIT. `data.json` and
`static/thumbnails.jpg` are the wiki's, under CC BY-NC-SA 4.0, and so is anything built carrying
them. See [LICENSE](LICENSE) and [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md).
