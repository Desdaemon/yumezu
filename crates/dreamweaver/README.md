# dreamweaver

Builds and serves `data.json`, the world dump yumezu draws.

It reimplements the data half of [Yume-2kki-Explorer]'s `app.js`: the same document, in the same
shape, for the same reader. What it does not reimplement is how that program gets there. `app.js`
keeps a MySQL database, scrapes a dozen wiki pages for the parts of the dump that live in prose,
and runs a worker thread to reconcile the two. dreamweaver asks yume.wiki's Semantic MediaWiki
store for the structured data it holds -- which is nearly all of it -- and keeps the result in one
JSON file, which is both what it serves and what it reads back when it restarts.

```
dreamweaver [--listen 127.0.0.1:5000] [--data data.json] [--sync-every 6]
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
| `GET /data`    | the dump, byte for byte as the file holds it -- or `503 needs update` |
| `GET /data.json` | the same |
| `POST /pollUpdate` | what the running sync is doing: `{"task": ..., "done": ...}` |

There is no clock in the server, only requests. A `GET /data` arriving more than `--sync-every`
hours after the wiki was last asked about starts a sync and answers `503 needs update` instead of
the dump; every other one is answered from the file. A client that gets the 503 polls
`/pollUpdate` until it says `done` and then asks again -- so the whole of the wait, including the
minute a server with no dump at all takes to build its first, is one loop on the client's side.

The 503 is also why a dump is never served while it is being rebuilt, and why an empty one is
never served at all: an empty document is indistinguishable from a wiki with no worlds in it, and
the reader on the other end would draw the second.

`/pollUpdate` is how the reader says what the wait is for -- it is the reference implementation's own route, answered in the reference's own shape,
with the stage named by one of its task names. This program fetches the authors, the releases and
the passages as one concurrent question, so it names four stages where the reference names two
dozen. See `src/progress.rs`.

## Where the data comes from

All of it is the wiki's own store, asked directly through `api.php` -- see `src/smw.rs`. A world's
infobox, the passages out of it, the people credited for it and the releases it lived through are
all properties and subobjects, so these are queries for structured data and nothing here reads wiki
prose. Nothing is asked of [ynoproject/wikiwrapper] any more.

The one thing the wrapper answered that the store cannot is the **galleries** -- the pictures on a
world's page are page content rather than properties -- and they are no longer published. They were
the whole of what a second host and a second shape of answer were for, and nothing reads them.

Two of the queries exist because the wrapper could not answer them at all, and they are worth
knowing about:

- The **version history** has no endpoint on it, so `versionInfoData` used to go out empty and a
  reader could name the release a world arrived in but not say when that was. The store keeps a
  subobject per release, patches included, and one query dates all twelve hundred of them.
- The **connections** have an endpoint, and it cannot reach the end of them. The store refuses to
  look more than about five and a half thousand rows into a result set, and instead of saying so it
  answers with the first page again while the offset carries on counting -- which is what the
  wrapper's `continueKey` passes on when it appears to wrap. Yume 2kki has more passages than that,
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
Yume 2kki namespace have been edited since the dump was built. Most days the answer is none, and
the pass costs one small request and stops.

When the answer is not none, the list of pages is also a list of which answers are now stale, and
only those are asked for again. The author list is one page. The version history is a handful. A
passage belongs to the page of the world it leaves, so an
edited world can only have changed the letter its own title falls under. The worlds themselves are
re-read every time -- that is one query for all sixteen hundred, and a cache of them would be
something to reconcile rather than something to skip.

What that saves is requests rather than minutes: the store answers quickly, with the worlds taking
about twenty seconds and all twenty-seven passage groups together about thirty.

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
answers without any page here being touched. The **full** sync is the backstop, and a sync with no
dump to compare against is one: what a server does when it comes up with nothing on disk, and what
it falls back to when the dump is older than the wiki's memory or the wiki cannot be asked. There is
no route for demanding one -- deleting `--data` and restarting is the ask. That is also what
`lastFullUpdate` marks: a soft sync carries the
stamp over rather than moving it, so a reader can tell a dump that took the wiki at its word from
one this program checked for itself.

The wiki's edge answers a plain request with a challenge page, so every request carries
`Origin: https://explorer.yume.wiki` -- the explorer this program stands in for.

## What is not in the dump

`effectData`, `menuThemeData`, `wallpaperData` and `bgmTrackData` are published as empty lists.
None of them says anything about how the worlds join up, which is the whole of what this dump is
read for, and none of them is in the store: effects and menu themes live in prose and a table on
their pages, and reading those would make this the second program scraping them. The fields stay in
the shape so a reader written against the reference dump keeps working.

Per-world `images` is left out as well: it is the gallery on a world's page, which the store does
not hold at all.

Per-world `size` is left out too, and something else is published in its place. The reference works
it out from the dimensions of the RPG Maker maps a world is built out of, scraped off the wiki's
`Map IDs` pages -- a table of `#id`, width and height -- and shares each map's area between the
worlds that use it. The store holds no width and no height for a map anywhere: `Has map ID`,
`Has map type`, `Map ID annotation` and `Is map part of game` are the only map properties the wiki
has, so the area cannot be had without scraping the same tables, which is the thing this program
exists not to do.

What the store does hold is which maps a world is, as subobjects on the world's own page, and those
go out as **`mapIds`** -- a field the reference dump has no equivalent of. It answers for 1574 of
1577 worlds: 3126 map numbers, 2765 of them distinct, 199 used by more than one world.

## The order the worlds go out in

A world's published id is its index in `worldData`, so the order is an interface: it is what a
client's caches are keyed by and what the thumbnail atlas is packed in. It is the game's map
numbering -- the origin first, then each world by the earliest map it is built out of, then by
title where two worlds share that map.

The origin is the world built on **map 2**, named by the number rather than by its title: the wiki
renames a world far more readily than the game renumbers a map. **Map 1** is the debug room, which
the wiki documents as a location like any other and the game never walks the player into; the dump
carries it like any other world and marks it `secret`, which is the one thing it says about a world
not meant to be shown. Nothing here acts on that mark -- see [what a secret is](#what-a-secret-is).

The point is that nothing in it reads the last dump. The reference's ids are its database's insert
order, which cannot be reproduced by anything but that database; this program used to imitate it by
carrying the previous dump's order forward, which meant a run that came up with nothing published a
different dump from one that did not -- alphabetical, opening on `3D Structures Path` rather than
the room the player wakes up in. Map numbers are handed out by the game in the order the maps were
made, so ordering by them is very nearly ordering by when the world was added, and it is the same
answer every time: a cold build and a warm build of the same wiki publish byte-identical ids.

"Relatively" stable, because a world already published moves if the wiki corrects which maps it is.
A world added next week takes map numbers above every number now in use and lands at the end,
moving nothing.

Map 2 is named explicitly and has to be: map 1 is lower, so the debug room would otherwise open the
dump. It is also the one place in the order a reader depends on.

## What a secret is

`secret` on a world means a reader is not meant to be shown it. Two things set it: a world built out
of map 1, and whatever an operator has marked by hand in `data.json`, which every later sync carries
forward by title.

A secret is published and marked rather than dropped. Dropping it would forget the mark -- the last
dump is where the marks are read from, so a world left out of one sync is unmarked by the next --
and hiding is a question about a reader rather than about the game. So a secret keeps its id, stays
in the graph the depths are measured on, and stays an end of the passages that reach it. The client
is what leaves it out: `yumezu` drops secret worlds as it reads the dump and renumbers the
connections behind them, and `tools/atlas` drops the same worlds so the thumbnail cells still line
up.

[Yume-2kki-Explorer]: https://github.com/Yume-2kki-Explorer/Yume-2kki-Explorer
[ynoproject/wikiwrapper]: https://github.com/ynoproject/wikiwrapper
