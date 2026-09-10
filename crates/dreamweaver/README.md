# dreamweaver

Builds and serves `data.json`, the world dump yumezu draws.

It reimplements the data half of [Yume-2kki-Explorer]'s `app.js`: the same document, field for
field, for the same reader. What it does not reimplement is how that program gets there. `app.js`
keeps a MySQL database, scrapes a dozen wiki pages for the parts of the dump that live in prose, and
runs a worker thread to reconcile the two. dreamweaver asks yume.wiki's Semantic MediaWiki store for
the structured data it holds -- which is nearly all of it -- and keeps the result in one JSON file,
which is both what it serves and what it reads back when it restarts.

```
dreamweaver [--listen 127.0.0.1:5000] [--data data.json] [--sync-every 1]
```

`--listen` takes a `host:port` or, if it has a `/` in it, the path of a Unix socket -- so that
nginx can reach it either way its `proxy_pass` knows:

```nginx
proxy_pass http://127.0.0.1:5000;                # dreamweaver --listen 127.0.0.1:5000
proxy_pass http://unix:/run/dreamweaver.sock:;   # dreamweaver --listen /run/dreamweaver.sock
```

The socket is made mode `0666` and removed again when the server stops -- and a stale one left by
a run that was killed is cleared out of the way at startup. Which is to say the directory it sits
in is what decides who may connect, which is worth choosing on purpose.

There is nothing to run but the server and nothing to tell it to do. It writes every version of the
dump to `--data`, so a run that comes up with the wiki unreachable still serves the last dump it
wrote.

| route          | |
|----------------|--|
| `GET /data`    | the dump, byte for byte as the file holds it |
| `GET /data.json` | the same |
| `GET /pollUpdate` | what the running sync is doing: `{"task": ..., "done": ...}` |

The server keeps the dump current on its own clock -- a sync every `--sync-every` hours, timed
from when the dump on disk was built so a restart is not a way to make it re-read the wiki. Every
`GET /data` is answered from the file, including the ones that arrive while a sync is running: the
dump standing is what a pass that publishes nothing would hand back anyway.

The one thing never served is an empty dump, which is indistinguishable from a wiki with no worlds
in it and would have the reader on the other end draw the second. A server that has not finished
its first sync answers `503 needs update`, and the client polls `/pollUpdate` until it says `done`
and then asks again.

`/pollUpdate` is how the reader says what the wait is for -- it is the reference implementation's
own route, answered in the reference's own JSON, with the stage named by one of its task names. This
program fetches the authors, the releases and the connections as one concurrent question, so it
four stages where the reference names two dozen. See `src/progress.rs`.

## Where the data comes from

All of it is the wiki's own store, asked directly through `api.php` -- see `src/smw.rs`. A world's
infobox, the connections out of it, the people credited for it and the releases it lived through are
all properties and subobjects, so these are queries for structured data and nothing here reads wiki
prose. Nothing is asked of [ynoproject/wikiwrapper] any more.

The one thing the wrapper answered that the store cannot is the **galleries** -- the pictures on a
world's page are page content rather than properties -- and they are no longer published. They were
the whole of what a second host and a second response format were for, and nothing reads them.

Two of the queries exist because the wrapper could not answer them at all, and they are worth
knowing about:

- The **version history** has no endpoint on it, so `versionInfoData` used to go out empty and a
  reader could name the release a world arrived in but not say when that was. The store keeps a
  subobject per release, patches included, and one query dates all twelve hundred of them.
- The **connections** have an endpoint, and it cannot reach the end of them. The store refuses to
  look more than about five and a half thousand rows into a result set, and instead of saying so it
  answers with the first page again while the offset carries on counting -- which is what the
  wrapper's `continueKey` passes on when it appears to wrap. Yume 2kki has more connections than
  that,
  so every one past the cap was invisible: alphabetically the last sixty-odd worlds' exits, missing
  from every dump built that way. Asking the store directly does not lift the cap; it allows the
  question to be split into one query per first letter of a world's title, each a few hundred rows.
  That is why the connections are fetched after the worlds -- the worlds are what say which letters
  there are.

The worlds and the authors the wrapper answered correctly, and they were moved anyway: a fetch that
goes to the same place as the rest can be steered by the same account of what has changed, which is
what the next section is about. Both were checked against the wrapper's answers field by field
before the switch -- identical worlds, pictures, maps, music and versions, and an author list
identical down to its order.

## Keeping up with the wiki

Every `--sync-every` hours the server does a **soft** sync: it asks the wiki which pages in the
Yume 2kki namespace have been edited since the dump was built. On most passes the answer is none
and the pass costs one small request, which is why the interval is set by how soon an edit should
show rather than by what the asking costs.

When the answer is not none, the list of pages is also a list of which answers are now stale, and
only those are asked for again. The author list is one page. The version history is a handful. A
connection belongs to the page of the world it leaves, so an edited world can only have changed the
letter its own title falls under. The worlds themselves are re-read every time -- that is one query
for all sixteen hundred, and a cache of them would be something to reconcile rather than something
to skip.

What that saves is requests rather than minutes: the store answers quickly, with the worlds taking
about twenty seconds and all twenty-seven connection groups together about thirty.

Two corrections are made to "everything since the dump was built", both because taking the wiki
literally would lose edits:

- The question starts an **hour earlier** than the dump's own stamp. The store is not written by
  the edit that changes it; a job queue re-reads the page afterwards, and until it has, a query
  answers with what the page used to say. A sync that asked only about what changed since it last
  ran would read the stale answer, move its stamp past the edit, and never ask again.
- A dump older than **thirty days** is not asked about at all, and is rebuilt whole. MediaWiki
  keeps its record of recent changes for a fixed span and then forgets, so a question from further
  back than it reaches is answered with what it still has -- which reads exactly like "nothing has
  changed".

There is one hole left in it, and it is the reference implementation's too: only Yume 2kki's
namespace is watched, so a template or a file the worlds are built out of can change what the store
answers without any page here being touched. Worse, a soft sync that misses an edit misses it for
good -- nothing later asks about that week again.

So **once a week the sync is a full one**: it skips the question of what has changed and reads the
whole wiki. `lastFullUpdate` is when that last happened, and what the week is counted from, so the
schedule survives a restart -- a soft sync carries the stamp over rather than moving it.

Three other cases read the whole wiki, and they are the same pass: a server coming up with nothing
on disk, a dump older than the wiki's memory, and a wiki that cannot be asked what it changed.
Deleting `--data` and restarting is the only way to demand one -- there is no route for it.

A full run is also the only pass that gives up an [atlas cell](#the-atlas-cells), and only for a
world the dump has stopped publishing. Nothing still published moves, so it never costs a repack:
what it reclaims is the tail of the cell range, a gap inside staying a gap. Waiting for the weekly
pass to do it gives a world the wiki marks removed and then restores a week to keep its picture.

The wiki's edge answers a plain request with a challenge page, so every request carries
`Origin: https://explorer.yume.wiki` -- the explorer this program stands in for.

## What is not in the dump

`effectData`, `menuThemeData`, `wallpaperData` and `bgmTrackData` are published as empty lists.
None of them says anything about how the worlds join up, which is the whole of what this dump is
read for, and none of them is in the store: effects and menu themes live in prose and a table on
their pages, and reading those would make this the second program scraping them. The fields stay as
empty lists so a reader written against the reference dump keeps working.

Per-world `images` is left out as well: it is the gallery on a world's page, which the store does
not hold at all.

Per-world `size` is left out too, and something else is published in its place. The reference works
it out from the dimensions of the RPG Maker maps a world is built out of, scraped off the wiki's
`Map IDs` pages -- a table of `#id`, width and height -- and shares each map's area between the
worlds that use it. The store holds no width and no height for a map anywhere: `Has map ID`,
`Has map type`, `Map ID annotation` and `Is map part of game` are the only map properties the wiki
has, so the area cannot be had without scraping the same tables, which is the thing this program
exists not to do.

The store does hold which maps a world is, as subobjects on the world's own page, and this used to
publish them as a `mapIds` field of its own. Nothing read it. It ordered the dump, and it marked
the debug room a secret, and the [atlas cells](#the-atlas-cells) took the first job away while the
second turned out to be doing very little -- so the property is no longer asked for at all, and the
dump is 31 KiB lighter for it.

## The order the worlds go out in

A world's published id is its index in `worldData`, and a connection names its far end with one, so
the two have to agree within a dump. Nothing else reads a world's position: this program and
`yumezu` both find the origin by title, and `yumezu` persists nothing keyed by an id.

So the order is **the origin first, then every other world by title**. The origin is named rather
than sorted to the front, where `Urotsuki's Room` does not belong: a dump opening on
`3D Structures Path` reads as a mistake, and it is the same title `depth` measures every distance
from.

The point is that nothing in it reads the last dump. The reference's ids are its database's insert
order, which cannot be reproduced by anything but that database; this program used to imitate it by
carrying the previous dump's order forward, which meant a run that came up with nothing published a
different dump from one that did not. Title is a property of the wiki, so a cold build and a warm
build of the same wiki publish byte-identical ids.

The order was the game's map numbering until the [atlas cells](#the-atlas-cells) arrived. That put
a world roughly where it was added, which mattered only because the thumbnail atlas was packed in
this order and could not survive an insertion -- and it did not really survive one anyway, a world
moving whenever the wiki corrected which maps it was built out of. A cell does that job properly,
and title is the same key the cells and the secret marks are already carried by.

## The atlas cells

The thumbnail atlas is one image holding a small picture of every world, and **`cell`** is where in
it a world's picture sits. It is a separate field from the id because the two want opposite things:
an id is a place in an order the wiki can move a world within, and a cell must never move at all. A
world documented late, or one whose maps the wiki corrects, shifts every id above it -- and would,
if the atlas were packed by id, hand every world after it somebody else's picture until the atlas
was packed again.

So a cell is handed out once. A world keeps whatever the last dump gave it; one first seen now takes
a cell above every cell in use; a world dropped from the dump leaves a gap rather than freeing it
for the next world along, and only the [weekly full run](#keeping-up-with-the-wiki) reclaims the
cells above the last world still published. An atlas is then still right about every world it was packed
with however far the dump has moved on, and a world it has no cell for draws the placeholder until
`yumezu`'s `just thumbnails` is run again.

This is the one part of the dump that does read the last one, for the same reason the secret marks
do -- there is nowhere else to remember it -- and it carries the same two costs. A world the wiki
renames is a world this has never seen, and takes a fresh cell. And a cold build has nothing to
carry forward, so it hands out cells in publish order and the atlas has to be packed again after
one.

Secrets are given a cell like any other world, and `tools/atlas` packs it black rather than with
their picture: a mark meaning "do not show this" is worth little if the picture ships anyway. The
cell stays theirs, so unmarking one costs a repack and moves nothing.

## What a secret is

`secret` on a world means a reader is not meant to be shown it. An operator sets it by hand in
`data.json` and every later sync carries it forward by title.

The debug room used to be marked automatically, by being built out of map 1. That guarded one of
the thirteen marks and only on a cold build -- the run that has no previous dump to carry the other
twelve forward either. A cold build needs its marks put back by hand whatever this does, so the
rule bought nothing and is gone.

A secret is published and marked rather than dropped. Dropping it would forget the mark -- the last
dump is where the marks are read from, so a world left out of one sync is unmarked by the next --
and hiding is a question about a reader rather than about the game. So a secret keeps its id, stays
in the graph the depths are measured on, and stays an end of the connections that reach it. The
client is what leaves it out: `yumezu` drops secret worlds as it reads the dump and renumbers the
connections behind them.

[Yume-2kki-Explorer]: https://github.com/Yume-2kki-Explorer/Yume-2kki-Explorer
[ynoproject/wikiwrapper]: https://github.com/ynoproject/wikiwrapper
