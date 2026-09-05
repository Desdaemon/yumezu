# yumezu

## Install

```bash
git clone https://github.com/Desdaemon/yumezu.git
just thumbnails
cargo install --path .
```

## Run

The app draws the world dump `dreamweaver` serves, so that has to be running alongside it —
see `crates/dreamweaver`. Until the dump arrives the app shows a loading frame, and it says so if
the server cannot be reached.

```bash
just dreamweaver   # builds data.json from the wiki and serves it on 127.0.0.1:5000
yumezu_main        # or `just serve` for the page, which proxies /data.json to the same port
```
