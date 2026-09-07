# Acknowledgements

## Code

**`crates/force_graph_3d`** is the 2D [force_graph](https://github.com/t-mw/force-graph-rs) crate
by Tobias Mansfield-Williams, taken into three dimensions and rewritten for speed: the node state
is a struct of arrays, the repulsion pass is vectorized and integrated Jacobi-style, and distant
groups of nodes are summed through a Barnes-Hut octree. `force_graph` in turn implements the
algorithm of [Graphoon](https://github.com/rm-code/Graphoon/) by Robert Machmer. Both are MIT.

**`src/text_agent.rs`** is `eframe`'s hidden `<input>`, the thing that gives a page an input
method that winit's web backend does not: carried from `crates/eframe/src/web/` at
[egui](https://github.com/emilk/egui) 0.36.0, (c) Emil Ernerfeldt and the egui contributors, under
MIT OR Apache-2.0.

**`crates/dreamweaver`** publishes `data.json`, which reuses the same format as [Yume-2kki-Explorer](https://github.com/Flashfyre/Yume-2kki-Explorer) by Flashfyre (MIT)
-- as well as serving as the reference implementation. See `crates/dreamweaver/README.md`.

## Data

- [yume.wiki](https://yume.wiki) -- the worlds, the connections, the maps, the authors
  and the version history, read from its Semantic MediaWiki store. Written by the wiki's editors
  and licensed [CC BY-NC-SA 4.0](https://creativecommons.org/licenses/by-nc-sa/4.0/legalcode); see
  [YumeWiki:Copyrights](https://yume.wiki/YumeWiki:Copyrights).
- [explorer.yume.wiki](https://explorer.yume.wiki) -- the world images that `tools/atlas` packs
  into the thumbnail atlas. The wiki's, under the same licence.
- [YNOproject](https://ynoproject.net) -- where the game is played, and, for a signed-in player,
  which worlds they have been to. `unknown_location.png` is its client's own placeholder. Its
  [ynolocations](https://github.com/ynoproject/ynolocations) list maps world titles between
  languages, and [wikiwrapper](https://github.com/ynoproject/wikiwrapper) is what the dump was
  first built out of, before dreamweaver went to the store directly.
- [yume2kki-t](https://wikiwiki.jp/yume2kki-t/) -- the Japanese wiki a world's and an author's
  Japanese pages are linked to.

## Fonts and icons

- [Noto Sans JP](https://github.com/notofonts/noto-cjk) provides fallback Japanese fonts. SIL Open Font License 1.1.
- [Material Symbols](https://github.com/google/material-design-icons), through `egui_material_icons`. Apache-2.0.
