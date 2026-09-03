//! The Yume 2kki world graph, as published by yume.wiki and captured in `data.json`.
//!
//! The dump carries far more per world than a layout needs — images, BGM, version history — so
//! only the fields the visualization draws are deserialized; serde skips the rest without
//! allocating for it.

use egui_material_icons::{
    MaterialIcon,
    icons::{ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_RANGE, ICON_BLOCK},
};
use serde::Deserialize;

/// The dump is embedded rather than fetched: it is a fixed asset of this demo, and compiling it
/// in keeps startup synchronous on both native and wasm, where the app has no async path.
const DATA: &str = include_str!("../data.json");

/// The dump as it is published: the worlds, and the release history the wiki dates them by.
#[derive(Deserialize)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    /// Every release the wiki knows of, newest first. Most of them added no world at all, so the
    /// catalog is built out of [`Dump::versions`] rather than out of this directly.
    #[serde(rename = "versionInfoData")]
    releases: Vec<Release>,
}

#[derive(Deserialize)]
pub struct World {
    /// Shown in the overlay for the selected world and for every step of its route.
    pub title: String,
    /// Who made the world, as the wiki credits them. The overlay names them, and lights every
    /// world of theirs when the name is clicked.
    pub author: String,
    /// Where the wiki serves this world's picture from, at the size the wiki holds it. Packed
    /// into the thumbnail atlas by `tools/atlas`, and fetched from here again once the view comes
    /// close enough for the atlas to have run out of detail: see `detail`.
    #[serde(rename = "filename")]
    pub image: String,
    /// The release this world first appeared in, as the wiki names it. `None` for the few worlds
    /// the wiki does not date. See [`Dump::versions`].
    #[serde(rename = "verAdded")]
    added: Option<String>,
    /// Where the wiki serves this world's maps from, and what it captions each of them, both as
    /// the `|`-separated lists the dump publishes. Read together and never apart: see
    /// [`World::maps`].
    #[serde(rename = "mapUrl")]
    map_url: Option<String>,
    #[serde(rename = "mapLabel")]
    map_label: Option<String>,
    pub connections: Vec<Connection>,
}

/// One of the maps the wiki draws of a world.
pub struct Map {
    /// The wiki's own caption for it, which is a sentence ("Map of Blood World") rather than a
    /// name: a world with several maps is where it earns its keep, because that is where the
    /// caption is the only thing saying which part of the world each one covers.
    pub label: String,
    /// Where the picture is served from, at the size the wiki holds it.
    pub url: String,
}

impl World {
    /// The maps the wiki has of this world, in the order it lists them. Empty for the few hundred
    /// worlds it has drawn none of.
    ///
    /// The two lists are published in step, but they are walked together rather than trusted to
    /// be: a map the wiki left uncaptioned is one this would otherwise panic on.
    pub fn maps(&self) -> Vec<Map> {
        let (Some(urls), Some(labels)) = (&self.map_url, &self.map_label) else {
            return Vec::new();
        };
        urls.split('|')
            .zip(labels.split('|'))
            .filter(|(url, _)| !url.is_empty())
            .map(|(url, label)| Map {
                label: label.to_owned(),
                url: url.to_owned(),
            })
            .collect()
    }
}

/// One release, as the dump's own version history lists it.
#[derive(Deserialize)]
struct Release {
    name: String,
    /// ISO 8601, of which only the day is ever shown. `None` where the wiki does not date it.
    #[serde(rename = "releaseDate")]
    date: Option<String>,
}

/// A release that added at least one world: what the catalog lists.
pub struct Version {
    /// As the version history names it, or as the worlds do for a release the history has no
    /// entry for.
    pub name: String,
    /// The day it was released, `YYYY-MM-DD`. Empty where the dump does not date it.
    pub released: String,
    /// The worlds it added, in world order.
    pub worlds: Vec<usize>,
}

/// Someone the wiki credits, and everything credited to them.
pub struct Author {
    pub name: String,
    /// In world order.
    pub worlds: Vec<usize>,
}

#[derive(Deserialize)]
pub struct Connection {
    #[serde(rename = "targetId")]
    pub target_id: usize,
    /// What the connection demands of the traveller, and which way it can be walked, as the
    /// bitfield the wiki publishes. See [`Gate`] and [`flag`].
    #[serde(rename = "type")]
    pub flags: u16,
    /// The wiki's own words for whichever of the demands it writes words for, keyed by the flag
    /// that makes the demand: the effects to be wearing, the odds, the season, or the sentence a
    /// locked condition is written out as. Most connections demand nothing and carry none. See
    /// [`Connection::ask`].
    #[serde(rename = "typeParams", default)]
    params: std::collections::HashMap<u16, TypeParams>,
}

/// The wiki's words for one demand. It publishes a Japanese rendering beside the English, which
/// nothing here reads, so serde skips it.
#[derive(Deserialize)]
struct TypeParams {
    params: Option<String>,
}

impl Connection {
    /// What this listing asks of a player walking it: the condition, and the wiki's words for it
    /// where it has any.
    fn ask(&self) -> Ask {
        let gate = Gate::of(self.flags);
        Ask {
            gate,
            detail: gate
                .worded()
                .and_then(|flag| self.params.get(&flag))
                .and_then(|words| words.params.clone())
                .filter(|words| !words.is_empty()),
        }
    }
}

/// What a connection asks of a player walking it one way.
///
/// The condition on its own is what a route is ordered by; the words are what a reader is told.
/// See [`Gate`] and [`Ask::asks`].
#[derive(Clone)]
pub struct Ask {
    pub gate: Gate,
    /// The wiki's words for the condition. `None` where it writes none, and always for a
    /// direction that is inferred rather than listed: nothing was written about a listing that
    /// does not exist. See [`walkable_steps`].
    detail: Option<String>,
}

impl Ask {
    /// What it asks, in the words the panel names it by: the wiki's own where it has any, and the
    /// bare name of the condition otherwise. Empty for a connection that asks nothing.
    pub fn asks(&self) -> String {
        let Some(detail) = self.detail.as_deref() else {
            return self.gate.asks().to_owned();
        };
        match self.gate {
            // Listed as the wiki lists them, comma separated, rather than joined into a sentence:
            // the wiki does not say whether one of them is enough or all of them are needed, and
            // an "and" or an "or" here would be this program saying which.
            Gate::Effect => format!("needs {}", detail.replace(',', ", ")),
            Gate::Chance => format!("{detail} chance"),
            Gate::Seasonal => format!("in {detail}"),
            Gate::LockedCondition => detail.to_owned(),
            _ => self.gate.asks().to_owned(),
        }
    }
    pub fn asks_emoji(&self) -> &'static str {
        match self.gate {
            Gate::Free => "",
            Gate::Effect => "✨",
            Gate::Chance => "🍀",
            Gate::Locked => "🔒",
            Gate::LockedCondition => "🔐",
            Gate::DeadEnd => "↩",
            Gate::Isolated => "🚩",
            Gate::Seasonal => match self.detail.as_deref() {
                Some("Spring") => "🌸",
                Some("Summer") => "☀",
                Some("Fall") => "🍂",
                Some("Winter") => "❄",
                _ => "🗓",
            },
        }
    }
}

/// The connection flags this module reads, from the wiki's own `ConnType`.
///
/// The dump uses two more - `SHORTCUT` and `TRACKED` - which only describe a connection rather
/// than gate it or point it, so nothing here consults them.
pub mod flag {
    /// Walkable from the world that lists it, never back.
    pub const ONE_WAY: u16 = 1 << 0;
    /// Walkable only back to the world that lists it, never from it.
    pub const NO_ENTRY: u16 = 1 << 1;
    /// This side opens a connection the far side reports as [`LOCKED`].
    pub const UNLOCK: u16 = 1 << 2;
    pub const LOCKED: u16 = 1 << 3;
    /// Accessible only from isolated section: the far side of a [`DEAD_END`].
    pub const DEAD_END: u16 = 1 << 4;
    /// Leads somewhere with no way onward.
    pub const ISOLATED: u16 = 1 << 5;
    pub const EFFECT: u16 = 1 << 6;
    pub const CHANCE: u16 = 1 << 7;
    pub const LOCKED_CONDITION: u16 = 1 << 8;
    /// Where a shortcut comes out, walked backwards into the shortcut.
    pub const EXIT_POINT: u16 = 1 << 10;
    pub const SEASONAL: u16 = 1 << 11;
}

/// What a connection asks of the traveller before it can be walked.
///
/// Ordered by how readily a canonical route accepts it: [`Gate::Free`] is a connection with no
/// demand at all, and the rest are the conditions in the order the wiki's own path finder falls
/// back through them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Gate {
    Free,
    Effect,
    Chance,
    Seasonal,
    /// Unlocked from opposite entrance.
    Locked,
    /// Also where a shortcut comes out. The reference only ever admits [`flag::EXIT_POINT`]
    /// together with the whole locked group, so it belongs at that group's strictest end rather
    /// than beside [`Gate::Locked`], which would let a route through it sooner than the reference
    /// would.
    LockedCondition,
    /// Leads to an isolated section of the destination.
    DeadEnd,
    /// Opposite of DeadEnd, return to said world is only accessible from isolated section.
    Isolated,
}

impl Gate {
    /// The condition a set of flags imposes.
    ///
    /// Harshest wins where a connection carries several: they are demands the traveller has to
    /// meet together, so the route is only as free as its strictest one.
    fn of(flags: u16) -> Gate {
        [
            (flag::DEAD_END, Gate::DeadEnd),
            (flag::ISOLATED, Gate::Isolated),
            (
                flag::LOCKED_CONDITION | flag::EXIT_POINT,
                Gate::LockedCondition,
            ),
            (flag::LOCKED, Gate::Locked),
            (flag::SEASONAL, Gate::Seasonal),
            (flag::CHANCE, Gate::Chance),
            (flag::EFFECT, Gate::Effect),
        ]
        .into_iter()
        .find(|(flag, _)| flags & flag != 0)
        .map_or(Gate::Free, |(_, gate)| gate)
    }

    /// Which flag's `typeParams` carries the wiki's words for this condition, if any does.
    ///
    /// The dump writes words for exactly these four. The rest of the conditions are the whole of
    /// what they say, and [`Gate::asks`] is all there is to read out for them.
    fn worded(self) -> Option<u16> {
        match self {
            Gate::Effect => Some(flag::EFFECT),
            Gate::Chance => Some(flag::CHANCE),
            Gate::Seasonal => Some(flag::SEASONAL),
            Gate::LockedCondition => Some(flag::LOCKED_CONDITION),
            Gate::Free | Gate::Locked | Gate::DeadEnd | Gate::Isolated => None,
        }
    }

    /// What the condition is called where the wiki writes no words of its own for it. Empty for
    /// one that asks nothing, which is most of them: a row with nothing after the title is a way
    /// a player can simply walk.
    fn asks(self) -> &'static str {
        match self {
            Gate::Free => "",
            Gate::Effect => "needs an effect",
            Gate::Chance => "by chance",
            Gate::Seasonal => "seasonal",
            Gate::Locked => "unlocked from opposite entrance",
            Gate::LockedCondition => "locked, conditional",
            Gate::DeadEnd => "only from isolated section",
            Gate::Isolated => "leads to isolated section",
        }
    }
}

/// One connection of a world, from that world's side of it: the world at the far end, and what it
/// asks in each of the two directions.
///
/// A direction with no gate at all is a direction there is no way to walk, which is what makes a
/// connection one-way. See [`connections`].
pub struct Step {
    pub world: usize,
    /// What it asks of a player walking there. `None` where there is no way there.
    pub out: Option<Ask>,
    /// What it asks of a player walking back. `None` where there is no way back.
    pub back: Option<Ask>,
}

impl Step {
    /// Which way a player can walk it, as the arrow the panel draws before a title: both ways,
    /// only there, only back, or — for the connection the dump lists but neither side can walk —
    /// no way at all.
    pub fn arrow(&self) -> MaterialIcon {
        match (self.out.is_some(), self.back.is_some()) {
            (true, true) => ICON_ARROW_RANGE,
            (true, false) => ICON_ARROW_FORWARD,
            (false, true) => ICON_ARROW_BACK,
            (false, false) => ICON_BLOCK,
        }
    }

    /// Whether it can be walked one way and not the other, which is what the drawing draws as
    /// marching dashes.
    pub fn one_way(&self) -> bool {
        self.out.is_some() != self.back.is_some()
    }
}

/// Per world, every world it is joined to, each with what the connection asks in either direction.
///
/// One entry per connection rather than one per listing: a connection is nearly always listed by
/// both of the worlds it joins, and it is still the one connection, so each of them carries it
/// once. A world's own listings come first, in the dump's order, and the connections only the far
/// side lists follow them.
///
/// Both the lines the visualization draws and the ways on it offers a reader are read from here,
/// so the panel names a connection one-way on exactly the connections drawn that way.
pub fn connections(worlds: &[World]) -> Vec<Vec<Step>> {
    let gates: std::collections::HashMap<_, _> = walkable_steps(worlds)
        .into_iter()
        .enumerate()
        .flat_map(|(from, steps)| steps.into_iter().map(move |(to, ask)| ((from, to), ask)))
        .collect();

    let mut joined: Vec<Vec<usize>> = vec![Vec::new(); worlds.len()];
    for (from, world) in worlds.iter().enumerate() {
        for connection in &world.connections {
            let to = connection.target_id;
            // A world connected to itself is no way anywhere, and the graph draws no line for it.
            if to != from && !joined[from].contains(&to) {
                joined[from].push(to);
            }
        }
    }
    // The same connections again from the far side, for the world that did not list them itself.
    for (from, world) in worlds.iter().enumerate() {
        for connection in &world.connections {
            let to = connection.target_id;
            if to != from && !joined[to].contains(&from) {
                joined[to].push(from);
            }
        }
    }

    joined
        .into_iter()
        .enumerate()
        .map(|(from, joined)| {
            joined
                .into_iter()
                .map(|to| Step {
                    world: to,
                    out: gates.get(&(from, to)).cloned(),
                    back: gates.get(&(to, from)).cloned(),
                })
                .collect()
        })
        .collect()
}

/// Where the wiki serves its pictures from, and the path the page asks for them under instead.
///
/// The page cannot ask the wiki directly. Its edge answers a request from another origin with a
/// challenge page rather than a picture, and the browser sets `Origin` itself and refuses to let
/// the header `detail::ORIGIN` carries stand in for it -- so the one thing that gets the native
/// build its pictures is the one thing a page may not do. Asking this host instead makes the
/// request same-origin, and the host is expected to proxy it on to the wiki.
///
/// A relative path, so it is whichever host served the page: see `download::URL` for the same
/// arrangement. The native and Android builds send the header themselves and reach the wiki
/// directly, so they keep the dump's own addresses and this does not exist for them.
#[cfg(target_family = "wasm")]
const WIKI_IMAGES: &str = "https://yume.wiki/images/";
#[cfg(target_family = "wasm")]
const PROXIED_IMAGES: &str = "/img/";

/// Parses the embedded dump.
///
/// Panics on a malformed dump: it is a compile-time asset, so a failure here is a broken build
/// rather than something the running app could recover from.
pub fn load() -> Dump {
    #[allow(unused_mut)]
    let mut dump =
        serde_json::from_str::<Dump>(DATA).expect("data.json is not the expected world dump");
    // Every picture address the app fetches at runtime passes through here, and only here: the
    // world's own and its maps'. See [`WIKI_IMAGES`]. The atlas tool reads `data.json` itself and
    // is not touched by this, which is right -- it runs at build time and has no page to be on.
    #[cfg(target_family = "wasm")]
    for world in &mut dump.worlds {
        world.image = world.image.replace(WIKI_IMAGES, PROXIED_IMAGES);
        if let Some(urls) = &mut world.map_url {
            // Rewritten whole rather than entry by entry: every address in the `|`-separated list
            // carries the same prefix, and a label list is never in this field. See
            // [`World::maps`].
            *urls = urls.replace(WIKI_IMAGES, PROXIED_IMAGES);
        }
    }
    dump
}

/// How a release is named across the two halves of the dump, which do not spell it identically:
/// the version history and the worlds disagree on case, and on a stray trailing dash.
fn same_release(name: &str) -> String {
    name.trim().trim_end_matches('-').to_lowercase()
}

impl Dump {
    /// The releases that added worlds, newest first, each carrying what it added.
    ///
    /// Ordered by the version history, which is already newest first and is the only ordering the
    /// dump gives: version names do not sort. The handful of releases the worlds name but the
    /// history does not know are left at the end, where they cannot claim a date they have not
    /// got.
    pub fn versions(&self) -> Vec<Version> {
        let rank: std::collections::HashMap<String, (usize, &Release)> = self
            .releases
            .iter()
            .enumerate()
            .map(|(at, release)| (same_release(&release.name), (at, release)))
            .collect();

        let mut versions: Vec<Version> = Vec::new();
        let mut at = std::collections::HashMap::new();
        for (world, added) in self.worlds.iter().enumerate() {
            let Some(added) = added.added.as_deref() else {
                continue;
            };
            let key = same_release(added);
            let version = *at.entry(key.clone()).or_insert_with(|| {
                let (name, released) = match rank.get(&key) {
                    // The day alone: the dump times every release to midnight, so the rest of the
                    // stamp says nothing.
                    Some((_, release)) => (
                        release.name.clone(),
                        release
                            .date
                            .as_deref()
                            .unwrap_or_default()
                            .chars()
                            .take(10)
                            .collect(),
                    ),
                    None => (added.to_string(), String::new()),
                };
                versions.push(Version {
                    name,
                    released,
                    worlds: Vec::new(),
                });
                versions.len() - 1
            });
            versions[version].worlds.push(world);
        }
        // Stable, so the releases the history does not carry — all of which sort last together —
        // keep the order the worlds named them in.
        versions.sort_by_key(|version| {
            rank.get(&same_release(&version.name))
                .map_or(usize::MAX, |(at, _)| *at)
        });
        versions
    }

    /// Everyone credited, most prolific first, each carrying their worlds, and, per world, which
    /// of them made it.
    ///
    /// Busiest first because that is the order the catalog reads best in with nothing typed into
    /// it: with only the first few shown, the names worth offering unasked are the ones with the
    /// most behind them. Ties break by name, so the order is fixed rather than however the worlds
    /// happened to be listed.
    pub fn authors(&self) -> (Vec<Author>, Vec<usize>) {
        let mut at = std::collections::HashMap::new();
        let mut authors: Vec<Author> = Vec::new();
        for (world, by) in self.worlds.iter().enumerate() {
            let author = *at.entry(by.author.as_str()).or_insert_with(|| {
                authors.push(Author {
                    name: by.author.clone(),
                    worlds: Vec::new(),
                });
                authors.len() - 1
            });
            authors[author].worlds.push(world);
        }
        authors.sort_by(|a, b| {
            b.worlds
                .len()
                .cmp(&a.worlds.len())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        // After the sort, so a world names where its author ended up rather than where they were
        // first met.
        let mut author_of = vec![0; self.worlds.len()];
        for (author, by) in authors.iter().enumerate() {
            for &world in &by.worlds {
                author_of[world] = author;
            }
        }
        (authors, author_of)
    }
}

fn append_encoded(input: &str, output: &mut String) {
    for byte in input.bytes() {
        match byte {
            b' ' => output.push('_'),
            byte if byte.is_ascii_alphanumeric() => output.push(byte as char),
            b'-' | b'_' | b'.' | b'\'' | b'(' | b')' | b',' | b'!' | b'/' => {
                output.push(byte as char)
            }
            byte => output.push_str(&format!("%{byte:02X}")),
        }
    }
}

/// Where a world's page sits on the wiki this dump comes from.
///
/// The dump carries no page address, only image ones, so the address is built from the title the
/// way the wiki builds it: spaces become underscores, and the few characters a title holds that a
/// URL cannot carry raw are percent-encoded. The titles in this dump are ASCII but for a single
/// accent, so the encoder only has to cover the bytes above it.
pub fn wiki_url(title: &str) -> String {
    let mut url = String::from("https://yume.wiki/2kki/");
    append_encoded(title, &mut url);
    url
}

pub fn author_url(author: &str) -> String {
    let mut url = String::from("https://yume.wiki/Category:");
    append_encoded(author, &mut url);
    url
}

/// The canonical route from every world back to world 0, Urotsuki's Room.
pub struct Routes {
    /// Per world, the world one step closer to the origin along its canonical route. `None` for
    /// the origin itself and for anything it cannot reach.
    pub parents: Vec<Option<usize>>,
    /// Per world, how many connections its canonical route is long. `None` where unreachable.
    ///
    /// Measured here rather than read from the dump's own `depth` so that it agrees with the
    /// connections this visualization actually knows about, but it is the same idea: how deep a
    /// player has to go to stand in that world.
    pub depth: Vec<Option<u32>>,
}

impl Routes {
    /// How many worlds hang off each world: those whose canonical route home passes through it.
    ///
    /// The count says how much of the game a world is the way to, which is what the node sizes
    /// show. Sizing reads it through a logarithmic curve, so the ranks a player cares about — a
    /// leaf, a small hub, a gateway — separate while the origin, which everything hangs off, stays
    /// on the same scale as the rest.
    pub fn descendant_counts(&self) -> Vec<u32> {
        // Deepest first, so a world's own descendants are all counted before it hands them up.
        let mut order: Vec<usize> = (0..self.parents.len()).collect();
        order.sort_unstable_by_key(|&world| std::cmp::Reverse(self.depth[world]));
        let mut descendants = vec![0; self.parents.len()];
        for &world in &order {
            if let Some(parent) = self.parents[world] {
                descendants[parent] += descendants[world] + 1;
            }
        }
        descendants
    }

    /// A world and every world that hangs off it: the subtree the canonical routes root there.
    ///
    /// Ordered shallowest first, so a reader walks it outward from the world itself.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        // Shallowest first, so a world's parent has already been decided when it is reached. A
        // world the origin cannot reach sorts before every depth and has no parent to inherit
        // from, which is what keeps it out of every subtree but its own.
        let mut order: Vec<usize> = (0..self.parents.len()).collect();
        order.sort_unstable_by_key(|&world| self.depth[world]);
        let mut inside = vec![false; self.parents.len()];
        inside[root] = true;
        for &world in &order {
            if let Some(parent) = self.parents[world] {
                inside[world] |= inside[parent];
            }
        }
        order.retain(|&world| inside[world]);
        order
    }
}

/// Walks the canonical route to every world, outward from world 0.
///
/// A world's canonical route is the one a player could actually be expected to walk: it asks
/// nothing of them if any route does, and only what it must otherwise. Formally, routes are
/// ordered by the harshest [`Gate`] anywhere along them and only then by length, so an
/// unconditional route wins however long it is, and a world whose every route is conditional
/// takes the mildest condition available. That ordering is why the depth this reports is the
/// higher, honest one: a locked or chance-gated shortcut no longer makes a world look shallow.
///
/// Directed, like the lines the visualization draws. A connection the player can only walk one
/// way is drawn as a run of marching dashes, and it is not a way in, so it cannot carry a route.
pub fn canonical_routes(worlds: &[World]) -> Routes {
    let mut routes = Routes {
        parents: vec![None; worlds.len()],
        depth: vec![None; worlds.len()],
    };
    if worlds.is_empty() {
        return routes;
    }
    let steps = walkable_steps(worlds);

    // Dijkstra over (gate, depth): the route settled for a world is always its own parent's route
    // with one step added, so the parent chain and the depth cannot disagree. Reversed, because
    // [BinaryHeap] is a max-heap. The world and the parent ride along in the key rather than
    // beside it, so that ties resolve the same way on every run.
    let mut queue = std::collections::BinaryHeap::from([std::cmp::Reverse((Gate::Free, 0, 0, 0))]);
    while let Some(std::cmp::Reverse((gate, depth, world, parent))) = queue.pop() {
        if routes.depth[world].is_some() {
            continue;
        }
        routes.depth[world] = Some(depth);
        routes.parents[world] = (world != 0).then_some(parent);
        for (next, step) in &steps[world] {
            if routes.depth[*next].is_none() {
                queue.push(std::cmp::Reverse((
                    gate.max(step.gate),
                    depth + 1,
                    *next,
                    world,
                )));
            }
        }
    }
    routes
}

/// Every step a player can take, as a directed adjacency list carrying what each step demands.
///
/// A connection is nearly always listed by both of the worlds it joins, each with its own flags,
/// and those two listings are the two directions. Where only one side lists it, the other
/// direction is inferred the way the wiki's own path finder infers it: [`flag::ONE_WAY`] means
/// there is no way back, and [`flag::UNLOCK`] means the way back is [`Gate::Locked`].
///
/// The routes walk it directly, and [`connections`] is the same thing read pairwise, so a line
/// drawn as one-way is one-way on exactly the steps a route is denied.
fn walkable_steps(worlds: &[World]) -> Vec<Vec<(usize, Ask)>> {
    let listed: std::collections::HashSet<_> = worlds
        .iter()
        .enumerate()
        .flat_map(|(from, world)| {
            world
                .connections
                .iter()
                .map(move |connection| (from, connection.target_id))
        })
        .collect();

    let mut steps = vec![Vec::new(); worlds.len()];
    for (from, world) in worlds.iter().enumerate() {
        for connection in &world.connections {
            let (to, flags) = (connection.target_id, connection.flags);
            if to == from {
                continue;
            }
            if flags & flag::NO_ENTRY == 0 {
                steps[from].push((to, connection.ask()));
            }
            if !listed.contains(&(to, from)) && flags & flag::ONE_WAY == 0 {
                let gate = if flags & flag::UNLOCK != 0 {
                    Gate::Locked
                } else {
                    Gate::Free
                };
                // No words: the wiki wrote none for a direction it did not list at all.
                steps[to].push((from, Ask { gate, detail: None }));
            }
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    /// A connection is one connection however many of its two worlds list it: both of them carry
    /// it, and they agree on which ways round it can be walked.
    #[test]
    fn a_connection_reads_the_same_from_either_end() {
        let worlds = super::load().worlds;
        let connections = super::connections(&worlds);
        for (from, steps) in connections.iter().enumerate() {
            for step in steps {
                let far = connections[step.world]
                    .iter()
                    .find(|far| far.world == from)
                    .expect("the world at the far end carries it too");
                assert_eq!(step.out.is_some(), far.back.is_some());
                assert_eq!(step.back.is_some(), far.out.is_some());
                assert_eq!(step.one_way(), far.one_way());
            }
        }
    }

    /// A condition is read out in the wiki's own words for it, not in the bare name of the
    /// condition, wherever the wiki writes any.
    #[test]
    fn a_condition_is_read_out_in_the_wikis_own_words() {
        let worlds = super::load().worlds;
        let connections = super::connections(&worlds);
        let asks: Vec<String> = connections
            .iter()
            .flatten()
            .filter_map(|step| step.out.as_ref())
            .map(super::Ask::asks)
            .collect();
        let any = |wanted: &str| asks.iter().any(|asks| asks.contains(wanted));
        assert!(any(" chance"), "no odds are read out");
        assert!(any("in Winter"), "no season is read out");
        assert!(
            asks.iter()
                .any(|asks| asks.starts_with("needs ") && asks != "needs an effect"),
            "no effect is named"
        );
    }

    /// The dump is embedded at compile time, so a shape change breaks the visualization silently
    /// otherwise: this proves it still parses and still carries a connected world graph.
    #[test]
    fn dump_parses() {
        let worlds = super::load().worlds;
        assert!(worlds.len() > 1000, "{} worlds", worlds.len());
        assert!(
            worlds
                .iter()
                .all(|w| w.connections.iter().all(|c| c.target_id < worlds.len()))
        );
        assert_eq!(worlds[0].title, "Urotsuki's Room");
    }

    /// A connection can demand several things at once, and the route is only as free as its
    /// strictest demand: the mildest of them would understate what the player has to have done.
    #[test]
    fn a_connection_is_named_by_its_harshest_demand() {
        use super::{Gate, flag};
        assert_eq!(Gate::of(0), Gate::Free);
        assert_eq!(Gate::of(flag::ONE_WAY | flag::NO_ENTRY), Gate::Free);
        assert_eq!(Gate::of(flag::EFFECT), Gate::Effect);
        assert_eq!(
            Gate::of(flag::CHANCE | flag::LOCKED_CONDITION),
            Gate::LockedCondition
        );
        assert!(Gate::Free < Gate::Effect && Gate::Effect < Gate::LockedCondition);
    }

    /// The origin is where every route ends, so it is the one world that is at no depth and has
    /// no world closer in.
    #[test]
    fn the_origin_roots_the_route_tree() {
        let routes = super::canonical_routes(&super::load().worlds);
        assert_eq!(routes.depth[0], Some(0));
        assert!(routes.parents[0].is_none());
    }

    /// The depth of a world and the route the overlay walks for it are one thing seen twice, so
    /// the walk has to arrive at the origin in exactly as many steps as the depth claims.
    #[test]
    fn depth_is_the_length_of_the_canonical_route() {
        let worlds = super::load().worlds;
        let routes = super::canonical_routes(&worlds);
        for (world, depth) in routes.depth.iter().enumerate() {
            let Some(depth) = *depth else {
                assert!(routes.parents[world].is_none(), "{world} is at no depth");
                continue;
            };
            let mut steps = 0;
            let mut step = world;
            while let Some(parent) = routes.parents[step] {
                steps += 1;
                step = parent;
                assert!(steps <= depth, "{} loops", worlds[world].title);
            }
            assert_eq!(step, 0, "{} walks back to {step}", worlds[world].title);
            assert_eq!(steps, depth, "{} walks {steps} steps", worlds[world].title);
        }
    }

    /// Taking the canonical route rather than the shortest one is the whole point: a world may
    /// only ever turn out deeper than the connections alone would put it, never shallower.
    ///
    /// The comparison is against what a walk that ignores both direction and conditions finds,
    /// which is what this used to report.
    #[test]
    fn conditions_only_ever_push_a_world_deeper() {
        let worlds = super::load().worlds;
        let routes = super::canonical_routes(&worlds);

        let mut neighbours = vec![Vec::new(); worlds.len()];
        for (from, world) in worlds.iter().enumerate() {
            for connection in &world.connections {
                let to = connection.target_id;
                if from != to {
                    neighbours[from].push(to);
                    neighbours[to].push(from);
                }
            }
        }
        let mut shortest = vec![None; worlds.len()];
        shortest[0] = Some(0);
        let mut queue = std::collections::VecDeque::from([0]);
        while let Some(world) = queue.pop_front() {
            let hops = shortest[world].unwrap() + 1;
            for &next in &neighbours[world] {
                if shortest[next].is_none() {
                    shortest[next] = Some(hops);
                    queue.push_back(next);
                }
            }
        }

        let mut deeper = 0;
        for (world, (canonical, shortest)) in routes.depth.iter().zip(&shortest).enumerate() {
            let (Some(canonical), Some(shortest)) = (canonical, shortest) else {
                continue;
            };
            assert!(
                canonical >= shortest,
                "{} is {canonical} deep but {shortest} hops away",
                worlds[world].title
            );
            deeper += (canonical > shortest) as usize;
        }
        // Not a threshold worth tuning: it only has to prove the rule bites at all rather than
        // reproducing the plain walk.
        assert!(deeper > worlds.len() / 4, "only {deeper} worlds moved");
    }

    /// One world in the dump can be left but never entered, so it has no canonical route at all
    /// and the visualization draws it as unreached. Pinned because it is the only one: a second
    /// would more likely be a misread flag than a real hole.
    #[test]
    fn only_the_world_with_no_way_in_is_unreachable() {
        let worlds = super::load().worlds;
        let routes = super::canonical_routes(&worlds);
        let unreachable: Vec<_> = routes
            .depth
            .iter()
            .enumerate()
            .filter(|(_, depth)| depth.is_none())
            .map(|(world, _)| worlds[world].title.as_str())
            .collect();
        assert_eq!(unreachable, ["Gallery of Me"]);
    }

    /// What the node sizes are read off: a world counts everything behind it, however far behind,
    /// and a world with nothing behind it counts nothing.
    #[test]
    fn a_world_counts_every_world_that_comes_after_it() {
        //   0 ── 1 ── 2 ── 3
        //     └── 4
        let routes = super::Routes {
            parents: vec![None, Some(0), Some(1), Some(2), Some(0)],
            depth: vec![Some(0), Some(1), Some(2), Some(3), Some(1)],
        };
        assert_eq!(routes.descendant_counts(), [4, 2, 1, 0, 0]);
    }

    /// The catalog's two lists are readings of the same dump, so between them they have to
    /// account for every world exactly once — no world in two authors' work, none in none.
    #[test]
    fn every_world_is_credited_and_dated_once() {
        let dump = super::load();
        let (authors, author_of) = dump.authors();
        assert!(authors.len() > 100, "{} authors", authors.len());
        for (world, &author) in author_of.iter().enumerate() {
            assert!(
                authors[author].worlds.contains(&world),
                "{} is not among its author's work",
                dump.worlds[world].title
            );
        }
        assert_eq!(
            authors.iter().map(|by| by.worlds.len()).sum::<usize>(),
            dump.worlds.len()
        );
        // Busiest first, which is the order the catalog offers them in.
        assert!(
            authors
                .windows(2)
                .all(|pair| pair[0].worlds.len() >= pair[1].worlds.len())
        );

        // Not every world: a handful are undated, and those belong to no release.
        let versions = dump.versions();
        let dated: usize = versions.iter().map(|version| version.worlds.len()).sum();
        assert!(dated > dump.worlds.len() * 9 / 10, "{dated} dated");
        assert!(versions.iter().all(|version| !version.worlds.is_empty()));
    }

    /// The version history and the worlds spell a release differently, so the join has to be made
    /// on more than equality or those releases would come out twice, undated, at the end.
    #[test]
    fn a_release_is_one_version_however_the_dump_spells_it() {
        let versions = super::load().versions();
        assert_eq!(
            versions
                .iter()
                .filter(|it| it.name == "0.129c patch 13")
                .count(),
            1
        );
        // Newest first, and dated, which is what the history's own order buys.
        assert!(versions[0].released > versions[1].released);
    }

    /// The address a world's title is turned into. Spaces are the wiki's underscores, an
    /// apostrophe rides along as itself, and anything a URL cannot carry is encoded.
    #[test]
    fn a_title_addresses_its_own_wiki_page() {
        assert_eq!(
            super::wiki_url("Urotsuki's Room"),
            "https://yume.wiki/2kki/Urotsuki's_Room"
        );
        assert_eq!(
            super::wiki_url("Fluorescent Cité"),
            "https://yume.wiki/2kki/Fluorescent_Cit%C3%A9"
        );
    }

    /// What the menu lights up. The subtree carries the world it is rooted at, and nothing
    /// off to the side of it, however near that world sits.
    #[test]
    fn a_subtree_is_a_world_and_everything_behind_it() {
        //   0 ── 1 ── 2 ── 3
        //     └── 4
        let routes = super::Routes {
            parents: vec![None, Some(0), Some(1), Some(2), Some(0)],
            depth: vec![Some(0), Some(1), Some(2), Some(3), Some(1)],
        };
        assert_eq!(routes.subtree(1), [1, 2, 3]);
        assert_eq!(routes.subtree(4), [4]);
        assert_eq!(routes.subtree(0), [0, 1, 4, 2, 3]);
    }
}
