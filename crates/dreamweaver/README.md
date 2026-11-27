# dreamweaver

Builds and serves `data.json`, the world dump yumezu draws, and the thumbnail atlas of every world's
picture. The dump is queried from yume.wiki's Semantic MediaWiki store and kept in one JSON file,
served as is and read back on restart.

```
dreamweaver [--listen ADDR|PATH] [--data PATH] [--atlas DIR] [--sync-every HOURS]
```

There are no subcommands: the binary is the server.

| flag | default | |
|---|---|---|
| `--listen` | `127.0.0.1:5000` | a `host:port`, or the path of a Unix socket if the value has a `/` |
| `--data` | `data.json` | where every version of the dump is written, and read back at startup |
| `--atlas` | `atlas` | where the atlas is written and read back, and the wiki's pictures are kept |
| `--sync-every` | `2` | hours between syncs |

A server that comes up with the wiki unreachable still serves the last dump written.

## Listening

nginx reaches the server either way:

```nginx
proxy_pass http://127.0.0.1:5000;                # dreamweaver --listen 127.0.0.1:5000
proxy_pass http://unix:/run/dreamweaver.sock:;   # dreamweaver --listen /run/dreamweaver.sock
```

The socket is made mode `0666`, so the socket's directory decides who may connect: choose that
directory on purpose. A stale socket left by a process that was killed is cleared at
startup, and the socket is removed when the server stops.

## Routes

| route | |
|---|---|
| `GET /data` | the dump, byte for byte as the file holds it |
| `GET /data.json` | the same |
| `GET /static/thumbnails.jpg` | the thumbnail atlas |
| `GET /static/unknown_location.png` | the picture for a world the player has not been to |
| `GET /pollUpdate` | what the running sync is doing: `{"task": ..., "done": ...}` |
| `GET /getNextLocations?origin=&dest=` | the exits from `origin` toward `dest` |
| `GET /yno/info` | YNOproject, forwarded: who the session belongs to and every location they visited |
| `GET /yno/gamelocations` | YNOproject, forwarded |
| `POST /ynoauth/login` | YNOproject sign-in, forwarded |
| `POST /ynoauth/forget` | YNOproject sign-out, forwarded |

Every `GET /data` is answered from the file, including the ones that arrive while a sync is running.

A server that has not finished its first sync answers **`503 needs update`**. Poll `/pollUpdate`
until the answer says `done`, then try again. An empty dump is never served: a client cannot tell
an empty dump from a wiki with no worlds.

The four `yno` routes carry a player's own YNOproject session, because a browser may not reach
YNOproject directly. **A signed-in request therefore goes through this host, and this host sees the
session.** The native builds reach YNOproject directly instead.

### `GET /getNextLocations`

Both worlds are named by their English titles, and the answer is at most three exits, nearest
first:

```json
[{"title": "Blue Eyes World", "titleJP": "碧眼世界", "connType": 0, "typeParams": {}, "depth": 6}]
```

`connType` and `typeParams` are the wiki's own flags and words for that exit; `depth` counts the
connections left to the destination, that exit included. Routes are ordered by the harshest
condition anywhere along them and then by length, so an unconditional route wins at any length. A
route through a world the wiki keeps secret is held back unless there is no other, and a route into
an isolated section is never offered.

A world the dump does not hold is answered with `200` and:

```json
{"error": "Invalid request", "err_code": "INVALID_REQUEST"}
```

`Access-Control-Allow-Origin` names ynoproject.net alone.

## Syncing

Every `--sync-every` hours the server does a **soft** sync: the server queries the wiki for which
pages have been edited since the dump was built, and re-reads only what those edits made stale. Once
a week a **full** sync reads the whole wiki instead.

Syncs are timed from when the dump on disk was built, so restarting is not a way to make the server
re-read the wiki. Three other cases read the whole wiki: a server coming up with nothing on disk, a
dump more than thirty days old, and a wiki that cannot say what changed.

**Deleting `--data` and restarting is the only way to demand a full read.** No route triggers one.
A cold build has no previous dump to carry cells or secret marks forward from, so every world may
change cell.

## Packing the atlas

At startup and after every sync that publishes, the server packs each world's wiki picture into a
64x48 cell of `thumbnails.jpg`, skipping the pack when no world's cell or picture has changed. The
grid's last cell holds YNOproject's placeholder. Pictures are downloaded once into `--atlas`'s
`images/` and kept by file name, so a picture the wiki replaces under the same name is not fetched
again until that file is deleted.

Both pictures answer `503` until the first pack, and a failed pack keeps the last atlas standing.

## Fields a reader should not expect

`effectData`, `menuThemeData`, `wallpaperData` and `bgmTrackData` are published as empty lists.
Per-world `images` and `size` are not published at all.

Per-world `cell` is where in the thumbnail atlas that world's picture sits, and never changes once
handed out. A world past the end of a cached atlas draws the placeholder until the reader fetches
a newer one.

## Marking a world secret

`secret` on a world means a reader is not meant to be shown the world. Set the field by hand in
`data.json`; every later sync carries the mark forward by title, and the restart that reads the mark
packs the world's cell black. A world the wiki renames loses the mark, as does a cold build.

Secret worlds stay in the dump, keep their id, and stay an end of the connections that reach them.
yumezu leaves them out when reading the dump.
