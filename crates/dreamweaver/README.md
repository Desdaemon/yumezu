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

There is nothing to run but the server and nothing to tell it to do. It builds the dump when it
comes up, keeps it current by itself, and writes every version of it to `--data` -- so a run that
comes up with the wiki unreachable still serves the last dump it wrote.

| route          | |
|----------------|--|
| `GET /data`    | the dump, byte for byte as the file holds it |
| `GET /data.json` | the same |
| `POST /update` | rebuild all of it, whatever the wiki says about itself |

`/update` is unguarded; do not expose the port.

## Where the data comes from

Everything but one thing is the wiki's own store, asked directly through `api.php` -- see
`src/smw.rs`. A world's infobox, the passages out of it, the people credited for it and the
releases it lived through are all properties and subobjects, so these are queries for structured
data and nothing here reads wiki prose.

The exception is the **galleries**: the pictures on a world's page are page content rather than
properties, and [ynoproject/wikiwrapper] has already done that reading, so `/images` is the one
endpoint still called.

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
world's gallery is its own page. A passage belongs to the page of the world it leaves, so an
edited world can only have changed the letter its own title falls under. The worlds themselves are
re-read every time -- that is one query for all sixteen hundred, and a cache of them would be
something to reconcile rather than something to skip.

What that saves is requests rather than minutes, and it is worth knowing why. The store answers
quickly: the worlds take about twenty seconds and all twenty-seven passage groups together about
thirty. The galleries take two or three minutes, because the wrapper's `/images` hands back fifty
worlds at a time and ignores every attempt to ask it for fewer -- so a sync that re-reads them at
all pays for all sixteen hundred, and any edited world makes it re-read them. Until a world's
gallery can be asked for by name, that is the floor on a sync that has anything to do.

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
answers without any page here being touched. The **full** sync is the backstop. A run does one when
it comes up, since it does not know how old what it read off disk is and may have read nothing at
all; after that, rebuilding regardless is the one thing the server waits to be asked for, and
`POST /update` is the asking. That is also what `lastFullUpdate` marks: a soft sync carries the
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

The reference dump also carries a per-world `size`, computed from the dimensions of the RPG Maker
maps a world is built out of. The store publishes which maps those are but not how big they are, so
the field is left out rather than guessed at.

[Yume-2kki-Explorer]: https://github.com/Yume-2kki-Explorer/Yume-2kki-Explorer
[ynoproject/wikiwrapper]: https://github.com/ynoproject/wikiwrapper
