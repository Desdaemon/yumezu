# dreamweaver

Builds and serves `data.json`, the world dump yumezu draws. It queries yume.wiki's Semantic
MediaWiki store for the Yume 2kki world data and keeps the result in one JSON file, which is both
what it serves and what it reads back when it restarts.

```
dreamweaver [--listen ADDR|PATH] [--data PATH] [--sync-every HOURS]
```

There is nothing to run but the server and nothing to tell it to do.

| flag | default | |
|---|---|---|
| `--listen` | `127.0.0.1:5000` | a `host:port`, or the path of a Unix socket if it has a `/` in it |
| `--data` | `data.json` | where every version of the dump is written, and read back at startup |
| `--sync-every` | `2` | hours between syncs |

A run that comes up with the wiki unreachable still serves the last dump it wrote.

## Listening

nginx reaches it either way:

```nginx
proxy_pass http://127.0.0.1:5000;                # dreamweaver --listen 127.0.0.1:5000
proxy_pass http://unix:/run/dreamweaver.sock:;   # dreamweaver --listen /run/dreamweaver.sock
```

The socket is made mode `0666`, so the directory you put it in is what decides who may connect:
choose it on purpose. A stale socket left by a run that was killed is cleared at startup, and the
socket is removed when the server stops.

## Routes

| route | |
|---|---|
| `GET /data` | the dump, byte for byte as the file holds it |
| `GET /data.json` | the same |
| `GET /pollUpdate` | what the running sync is doing: `{"task": ..., "done": ...}` |
| `GET /getNextLocations?origin=&dest=` | standing in `origin` and headed for `dest`, which ways on to take |
| `GET /yno/info` | YNOproject, forwarded: who the session belongs to and everywhere they have been |
| `GET /yno/gamelocations` | YNOproject, forwarded |
| `POST /ynoauth/login` | YNOproject sign-in, forwarded |
| `POST /ynoauth/forget` | YNOproject sign-out, forwarded |

Every `GET /data` is answered from the file, including the ones that arrive while a sync is running.

A server that has not finished its first sync answers **`503 needs update`**. Poll `/pollUpdate`
until it says `done`, then try again. An empty dump is never served: it is indistinguishable from a
wiki with no worlds in it, and the reader on the other end would draw the second.

The four `yno` routes carry a player's own YNOproject session, because a browser may not reach
YNOproject directly. **A signed-in request therefore goes through this host, and this host sees the
session.** The native builds reach YNOproject directly instead.

### `GET /getNextLocations`

Both worlds are named by their English titles, and the answer is at most three ways on, nearest
first:

```json
[{"title": "Blue Eyes World", "titleJP": "碧眼世界", "connType": 0, "typeParams": {}, "depth": 6}]
```

`connType` and `typeParams` are the wiki's own flags and words for that door; `depth` counts the
connections left to the destination, that door included. Routes are ordered by the harshest
condition anywhere along them and then by length, so an unconditional way round wins however long it
is. A route through a world the wiki keeps secret is held back unless there is no other, and one
that walks into an isolated section is never offered at all -- the way on from there is the way back.

A world the dump does not hold is answered with `200` and:

```json
{"error": "Invalid request", "err_code": "INVALID_REQUEST"}
```

`Access-Control-Allow-Origin` names ynoproject.net alone.

## Syncing

Every `--sync-every` hours the server does a **soft** sync: it queries the wiki for which pages have
been edited since the dump was built, and re-reads only what those edits made stale. Once a week it
does a **full** sync instead, reading the whole wiki.

Syncs are timed from when the dump on disk was built, so restarting is not a way to make the server
re-read the wiki. Three other cases read the whole wiki: a server coming up with nothing on disk, a
dump more than thirty days old, and a wiki that cannot say what it changed.

**Deleting `--data` and restarting is the only way to demand a full read.** There is no route for it.
Expect the atlas to need repacking afterwards: a cold build has no previous dump to carry cells
forward from, and none to carry the secret marks forward from either.

## Fields a reader should not expect

`effectData`, `menuThemeData`, `wallpaperData` and `bgmTrackData` are published as empty lists.
Per-world `images` and `size` are not published at all.

Per-world `cell` is where in the thumbnail atlas that world's picture sits, and never changes once
handed out. A world the atlas has no cell for draws the placeholder until yumezu's `just thumbnails`
is run again.

## Marking a world secret

`secret` on a world means a reader is not meant to be shown it. Set it by hand in `data.json`; every
later sync carries it forward by title. A world the wiki renames loses its mark, as does a cold
build. `tools/atlas` packs a secret world's cell black, so unmarking one costs a repack.

Secret worlds stay in the dump, keep their id, and stay an end of the connections that reach them.
yumezu is what leaves them out, as it reads the dump.
