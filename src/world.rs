//! The Yume 2kki world graph, as published by yume.wiki and served to this app as `data.json`
//! by `dreamweaver`. See [`load`].
//!
//! The dump carries far more per world than a layout needs — images, BGM, version history — so
//! only the fields the visualization draws are deserialized; serde skips the rest without
//! allocating for it.

use egui_material_icons::{
    MaterialIcon,
    icons::{ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_RANGE, ICON_BLOCK},
};
use serde::Deserialize;

use super::i18n::t;

/// The host a native run asks for everything: `dreamweaver`, on the machine the app is running
/// on. See `crates/dreamweaver`, which builds the dump out of the wiki and keeps it current.
///
/// Fetched rather than compiled in, so a build is not a snapshot of the wiki: a run draws
/// whatever the server has published since it was built, and worlds arrive weekly. What it costs
/// is a load the first frame has to wait out, which is what the app's loading frame is for.
#[cfg(all(not(target_family = "wasm"), not(feature = "production")))]
const SERVER: &str = "http://127.0.0.1:5000";

/// Where a `production` build asks instead: this project's own `dreamweaver`, deployed. Same
/// program as the one a development build reaches on this machine, so the same routes and the
/// same document -- see `crates/dreamweaver`.
///
/// The reference explorer at `explorer.yume.wiki` answers the same two routes and was what this
/// asked before there was anywhere else to ask. It is not a fallback: its ids are its database's
/// insert order and `dreamweaver`'s are the game's map numbering, so the two number the worlds
/// differently and a thumbnail atlas packed against one does not fit the other.
#[cfg(all(not(target_family = "wasm"), feature = "production"))]
const SERVER: &str = "https://explorer.yumemiru.dev";

/// Translations of authors to their yume2kki-t tag.
/// Rightfully these are corrections that should eventually be done on yume.wiki.
static JAPANESE_AUTHOR_OVERRIDES: phf::Map<&str, &str> = phf::phf_map! {
    "Bean" => "bean",
    "窯良" => "窯良(oneirokamara)",
    "コンテンツ" => "kontentsu",
    "Ouri" => "ouri",
    "sniperbob" => "Sniperbob",
    "Mokaccino" => "Moka",
    "◆gH8PoF17WqX" => "Ferdy",
    "Nightmare" => "†Nightmare†",
    "tKp9vEGEfhCD" => "◆tKp9vEGEfhCD",
    "Nulsdodage" => "nulsdodage"
};

/// The dump as it is published: the worlds, and the release history the wiki dates them by.
#[derive(Deserialize)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    /// Every release the wiki knows of, newest first. Most of them added no world at all, so the
    /// catalog is built out of [`Dump::versions`] rather than out of this directly.
    #[serde(rename = "versionInfoData")]
    releases: Vec<Release>,
    /// Everyone the wiki credits, and how the game itself writes their name where the two
    /// differ. Read only for those Japanese names: who made what is settled by the worlds
    /// themselves, in [`World::author`]. See [`Dump::authors`].
    #[serde(rename = "authorInfoData", default)]
    credits: Vec<Credit>,
}

/// One name the wiki credits, in each of the languages it publishes it in.
#[derive(Deserialize)]
struct Credit {
    name: String,
    #[serde(rename = "nameJP")]
    name_jp: Option<String>,
}

#[derive(Deserialize)]
pub struct World {
    /// As the wiki's English pages name it, which is also what its page is at: see [`wiki_url`].
    pub title: String,
    /// As the game itself names it, which the dump publishes for all but a few dozen worlds. What
    /// the overlay shows while the app is speaking Japanese: see [`Title`].
    #[serde(rename = "titleJP")]
    title_jp: Option<String>,
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
    /// Whether the dump says a reader is not meant to be shown this world: the debug room, and
    /// whatever else the wiki's own explorer holds back as a spoiler. Read only by [`hide`],
    /// which takes those worlds out before anything else sees the dump, so nothing downstream has
    /// to remember they exist.
    #[serde(default)]
    secret: bool,
    pub connections: Vec<Connection>,
}

/// A world's name, in each of the languages the dump publishes one in.
///
/// Both are kept rather than the one being shown, because they are not read for the same thing:
/// the wiki's own pages are named in English, so that is the one an address is built out of
/// whatever is on screen, and a reader may know a world by either.
pub struct Title {
    /// The English name. Always there, and always what [`wiki_url`] is given.
    pub en: String,
    /// The Japanese name, for the worlds the dump has one for.
    jp: Option<String>,
}

impl Title {
    /// The name to show: the Japanese one while the app speaks Japanese and the dump has it, and
    /// the English one otherwise.
    ///
    /// Read rather than stored, so choosing a language renames every world on screen without
    /// anything having to be rebuilt.
    pub fn show(&self) -> &str {
        match &self.jp {
            Some(jp) if super::i18n::speaking_japanese() => jp,
            _ => &self.en,
        }
    }

    /// Where `needle` falls in this name, and how much name is left over, for whichever of the
    /// names it fits best. `None` for a name it appears in neither of.
    ///
    /// Both names, whichever one is being shown: a reader who knows a world by one of them should
    /// not have to be reading in the other language to find it. `needle` is expected already
    /// lowercased, since one is matched against every world.
    pub fn find(&self, needle: &str) -> Option<(usize, usize)> {
        self.names()
            .filter_map(|name| Some((name.to_lowercase().find(needle)?, name.len())))
            .min()
    }

    /// Where to read about this: the Japanese wiki while the app speaks Japanese and there is a
    /// Japanese name to look up, and the English wiki otherwise.
    ///
    /// The two wikis name their pages after their own name for a world, so a name is only ever
    /// asked of the wiki that wrote it: the few dozen worlds the dump leaves unnamed in Japanese
    /// have no page on the Japanese wiki to open.
    pub fn wiki_url(&self) -> String {
        match &self.jp {
            Some(jp) if super::i18n::speaking_japanese() => yume2kki_t_url(jp),
            _ => wiki_url(&self.en),
        }
    }

    /// Every name this is known by, English first. What a search reads.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        [Some(self.en.as_str()), self.jp.as_deref()]
            .into_iter()
            .flatten()
    }
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
    /// Its name, in every language the dump gives one.
    pub fn titles(&self) -> Title {
        Title {
            en: self.title.clone(),
            jp: self.title_jp.clone(),
        }
    }

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
///
/// Both wikis list one person's whole body of work, but they keep it in different shapes, so
/// which of them to open is [`Author::wiki_url`]'s to answer rather than the caller's.
pub struct Author {
    /// Their name, in each language the dump gives one. Named the same way a world is, and read
    /// the same way: the English one addresses their wiki page, either one finds them.
    pub name: Title,
    /// In world order.
    pub worlds: Vec<usize>,
}

impl Author {
    /// Links to yume.wiki or yume2kki-t as approproiate.
    pub fn wiki_url(&self) -> String {
        if super::i18n::speaking_japanese() {
            let name = self.name.show();
            yume2kki_t_author_url(JAPANESE_AUTHOR_OVERRIDES.get(name).copied().unwrap_or(name))
        } else {
            author_url(&self.name.en)
        }
    }
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

/// The wiki's words for one demand.
///
/// It publishes a Japanese rendering beside the English, but only ever for the seasons, and those
/// are four fixed words this app names for itself: see `gate-seasonal-detail`. So serde skips it.
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
            return self.gate.asks();
        };
        match self.gate {
            // Listed as the wiki lists them, comma separated, rather than joined into a sentence:
            // the wiki does not say whether one of them is enough or all of them are needed, and
            // an "and" or an "or" here would be this program saying which.
            Gate::Effect => t!("gate-effect-detail", effects = detail.replace(',', ", ")),
            Gate::Chance => t!("gate-chance-detail", chance = detail),
            Gate::Seasonal => t!("gate-seasonal-detail", season = detail),
            // The wiki's own sentence, which it writes in English and publishes no Japanese for.
            Gate::LockedCondition => detail.to_owned(),
            _ => self.gate.asks(),
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
    fn asks(self) -> String {
        match self {
            Gate::Free => String::new(),
            Gate::Effect => t!("gate-effect"),
            Gate::Chance => t!("gate-chance"),
            Gate::Seasonal => t!("gate-seasonal"),
            Gate::Locked => t!("gate-locked"),
            Gate::LockedCondition => t!("gate-locked-condition"),
            Gate::DeadEnd => t!("gate-dead-end"),
            Gate::Isolated => t!("gate-isolated"),
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
/// Whichever host served the page, so a build is not tied to where it is put: see
/// `download::URL` for the same arrangement. The native and Android builds send the header
/// themselves and reach the wiki directly, so they keep the dump's own addresses and this does
/// not exist for them.
#[cfg(target_family = "wasm")]
const WIKI_IMAGES: &str = "https://yume.wiki/images/";

/// The `/img/` the page asks for its pictures under, written out from the page's own origin.
///
/// Built once and kept, since every address in the dump gets the same one. Whole rather than the
/// bare path the host actually sees, because these addresses do not reach the network through the
/// document. They are handed to `reqwest`, which parses each one by itself and has no page to
/// resolve it against, so a path with no host is not an address at all to it and the request
/// fails before it is sent. The origin in front resolves to exactly what the bare path would
/// have.
#[cfg(target_family = "wasm")]
fn proxied_images() -> &'static str {
    static PROXIED_IMAGES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PROXIED_IMAGES.get_or_init(|| format!("{}/img/", origin()))
}

/// Where the page was served from, which is the only host it may ask for anything without being
/// let: see [`proxied_images`] and [`url`].
#[cfg(target_family = "wasm")]
fn origin() -> &'static str {
    static ORIGIN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ORIGIN.get_or_init(|| {
        web_sys::window()
            .expect("the page has no window")
            .location()
            .origin()
            .expect("the page has no origin to ask for anything through")
    })
}

/// The host this run asks, which the page reads off itself.
///
/// The page cannot ask the server directly, for the reason [`WIKI_IMAGES`] is rewritten: a request
/// straight at it is a cross-origin one it sends no headers to allow, and a blocked mixed-content
/// one wherever the page itself is served over https. So the page asks its own host under the same
/// routes and the host puts them through -- see the proxies in `Trunk.toml`.
///
/// `production` therefore moves only the native builds. `dreamweaver` answers `/data` with no
/// `Access-Control-Allow-Origin` at all, so a page reading it straight off another origin gets
/// nothing; whatever serves the page proxies the routes on instead, to the `dreamweaver` running
/// on this machine locally and to [`SERVER`]'s wherever the page is deployed.
fn server() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return SERVER;
    #[cfg(target_family = "wasm")]
    return origin();
}

/// Where this run asks for the dump.
fn url() -> String {
    format!("{}/data", server())
}

/// What the server says it is building, named as the message to say about it.
///
/// A server that is rebuilding the dump answers [`url`] with `needs update` rather than with a
/// dump it is about to replace -- see [`load`] -- so the wait can be a minute, and this is what
/// the loading frame says during it. `POST /pollUpdate` is the reference implementation's own
/// route for asking, and `dreamweaver` answers in the same shape: see its `progress` module.
///
/// `None` for everything that is not a stage this app has words for: a server between syncs, a
/// host with no such route at all, and the two dozen finer stages the reference server names that
/// this one never reaches. All three mean the same thing on screen -- the plain wait.
pub async fn building() -> Option<&'static str> {
    let url = format!("{}/pollUpdate", server());
    let said = match ask(&url).await {
        Ok(said) => said,
        Err(error) => {
            // Not a warning: a host that does not answer this is a host that has nothing to say
            // about what it is building, which is most of them.
            log::debug!("cannot reach {url}: {error}");
            return None;
        }
    };
    stage(
        serde_json::from_str::<serde_json::Value>(&said)
            .ok()?
            .get("task")?
            .as_str()?,
    )
}

/// What to say about a stage the server named. See [`building`], and `STAGES` below for the ones
/// this app has words for.
fn stage(task: &str) -> Option<&'static str> {
    STAGES
        .iter()
        .find(|(named, _)| *named == task)
        .map(|(_, said)| *said)
}

/// Every stage this app can say something about: what the server calls it, and the message that
/// says it. See `dreamweaver`'s `progress`, which is where the names on the left come from.
const STAGES: [(&str, &str); 4] = [
    ("init", "dump-task-changes"),
    ("fetchWorldData", "dump-task-worlds"),
    ("fetchConnData", "dump-task-passages"),
    ("prepareWorldData", "dump-task-assembling"),
];

/// Asks `url` and reads the answer. A `POST` because that is what the route it is for expects.
async fn ask(url: &str) -> Result<String, super::fetch::Error> {
    Ok(super::fetch::client()
        .post(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Fetches the dump and parses it.
///
/// `Ok(None)` is the server saying it is building one, which is a wait rather than a failure: it
/// is also what starts that build, and asking again once it has finished is how the dump is got.
/// [`building`] is what to say meanwhile, and `app`'s `Dump` is what does the waiting.
///
/// `Err` carries what to say on screen rather than panicking, which is the whole of what fetching
/// costs over compiling in: a document off the network can fail to arrive, or arrive as something
/// else, and neither is worth taking the window down for. See `app`'s `Dump`, which names the
/// failure and leaves the app standing.
pub async fn load() -> Result<Option<Dump>, String> {
    let url = url();
    let Some(json) = dump(&url)
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?
    else {
        return Ok(None);
    };
    parse(&json)
        .map(Some)
        .map_err(|error| format!("{url} is not the expected world dump: {error}"))
}

/// The dump's bytes, as the server sends them, or `None` for a server that has none to send yet.
///
/// Its own request rather than [`download`], because the dump is the one document with an answer
/// that is neither itself nor a failure.
async fn dump(url: &str) -> Result<Option<String>, super::fetch::Error> {
    let response = super::fetch::client().get(url).send().await?;
    // The server is rebuilding, and would rather say so than serve a document it is about to
    // replace or one it has never built. See `dreamweaver`'s `data`.
    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.text().await?))
}

/// Whatever `url` answers with, for the documents that only ever answer with themselves.
async fn download(url: &str) -> Result<String, super::fetch::Error> {
    Ok(super::fetch::client()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Reads the dump, and rewrites the addresses in it that this platform cannot use as they stand.
fn parse(json: &str) -> serde_json::Result<Dump> {
    let mut dump = serde_json::from_str::<Dump>(json)?;
    hide(&mut dump.worlds);
    // Every picture address the app fetches at runtime passes through here, and only here: the
    // world's own and its maps'. See [`WIKI_IMAGES`]. The atlas tool reads `data.json` itself and
    // is not touched by this, which is right -- it runs at build time and has no page to be on.
    #[cfg(target_family = "wasm")]
    for world in &mut dump.worlds {
        world.image = world.image.replace(WIKI_IMAGES, proxied_images());
        if let Some(urls) = &mut world.map_url {
            // Rewritten whole rather than entry by entry: every address in the `|`-separated list
            // carries the same prefix, and a label list is never in this field. See
            // [`World::maps`].
            *urls = urls.replace(WIKI_IMAGES, proxied_images());
        }
    }
    Ok(dump)
}

/// Drops the worlds the dump marks secret, and renumbers what is left.
///
/// A connection names the world it leads to by that world's index in [`Dump::worlds`], so taking
/// a world out is not a matter of skipping it where it is drawn: every index above it moves, and
/// a reference left pointing at the old one would draw a line to somewhere else entirely. Done
/// here, at the one place the dump becomes the app's, so the rest of the app sees a graph that
/// simply does not have those worlds in it -- including the layout, which would otherwise pull
/// the visible graph towards a node nobody can see.
///
/// The thumbnail atlas is packed the same way, by index, so `tools/atlas` drops the same worlds:
/// the two agree on what a cell counts, or every picture after the first secret is somewhere
/// else's.
fn hide(worlds: &mut Vec<World>) {
    let mut kept = 0;
    let at: Vec<Option<usize>> = worlds
        .iter()
        .map(|world| {
            (!world.secret).then(|| {
                kept += 1;
                kept - 1
            })
        })
        .collect();
    if kept == worlds.len() {
        return;
    }
    log::info!("{} worlds are not for showing", worlds.len() - kept);

    let mut world = 0;
    worlds.retain(|_| {
        world += 1;
        at[world - 1].is_some()
    });
    for world in worlds.iter_mut() {
        world.connections.retain_mut(|connection| {
            // `flatten` covers both a passage into a hidden world and one out of the dump
            // altogether, which is a dump that disagrees with itself rather than anything this
            // can draw.
            match at.get(connection.target_id).copied().flatten() {
                Some(target) => {
                    connection.target_id = target;
                    true
                }
                None => false,
            }
        });
    }
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
        // Only where the two names differ: the dump gives a Japanese name for everyone it
        // credits, and for most of them it is the English one over again, which is nothing to
        // show or to search twice.
        let jp: std::collections::HashMap<&str, &str> = self
            .credits
            .iter()
            .filter_map(|credit| {
                let name_jp = credit.name_jp.as_deref()?;
                (name_jp != credit.name).then_some((credit.name.as_str(), name_jp))
            })
            .collect();
        let mut at = std::collections::HashMap::new();
        let mut authors: Vec<Author> = Vec::new();
        for (world, by) in self.worlds.iter().enumerate() {
            let author = *at.entry(by.author.as_str()).or_insert_with(|| {
                authors.push(Author {
                    name: Title {
                        en: by.author.clone(),
                        jp: jp.get(by.author.as_str()).map(|&name| name.to_owned()),
                    },
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
                .then_with(|| a.name.en.to_lowercase().cmp(&b.name.en.to_lowercase()))
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

/// The Japanese wiki, which is a different wiki with pages of its own rather than a translation
/// of the English one.
const YUME2KKI_T: &str = "https://wikiwiki.jp/yume2kki-t/";

/// Where YNOproject keeps the list of what that wiki calls each place, which is what the game's
/// own client addresses it by. See [`Pages`].
const YNOLOCATIONS: &str =
    "https://raw.githubusercontent.com/ynoproject/ynolocations/refs/heads/master/2kki/ja.json";

/// Every world whose Japanese page is not simply named after it: an area the wiki writes up
/// inside another world's page, or a name it files under a longer path. A few dozen, out of the
/// fifteen hundred the dump names in Japanese.
type Pages = std::collections::HashMap<String, String>;

/// The overrides, once they have arrived. Empty until then -- see [`load_pages`].
static PAGES: std::sync::OnceLock<Pages> = std::sync::OnceLock::new();

/// Fetches the location list and keeps what [`yume2kki_t_url`] reads out of it.
///
/// Started beside the dump rather than on the first Japanese link, because a link is opened from
/// a click and a click cannot wait: on the page, a window opened after the gesture has passed is
/// a popup the browser blocks. A link clicked in the first moment of a run is therefore addressed
/// without the list, which is the right address for all but the few dozen worlds in it.
///
/// Failure is a warning and nothing more, for the same reason. The list is served with an `ETag`
/// and a `max-age`, so a second run mostly revalidates rather than downloads -- see `fetch`.
pub async fn load_pages() {
    let pages = match download(YNOLOCATIONS).await {
        Ok(json) => parse_pages(&json),
        Err(error) => {
            log::warn!("cannot reach {YNOLOCATIONS}: {error}");
            return;
        }
    };
    log::info!(
        "{} japanese pages are named after something else",
        pages.len()
    );
    let _ = PAGES.set(pages);
}

/// Reads the list: the pages named twice, and nothing else it says.
///
/// It names places by map rather than by world, and most of it is which map is which, which this
/// has no use for. The pairs it does read are nested several ways -- a map may name one place,
/// several, or a different one per map it leads on from -- and none of that nesting says anything
/// either, so the document is walked rather than modelled.
fn parse_pages(json: &str) -> Pages {
    let mut pages = Pages::new();
    let Ok(list) = serde_json::from_str::<serde_json::Value>(json) else {
        log::warn!("{YNOLOCATIONS} is not JSON");
        return pages;
    };
    // Whole-name overrides first, so a per-map one wins where the list gives both.
    if let Some(titles) = list["locationUrlTitles"].as_object() {
        for (title, page) in titles {
            if let Some(page) = page.as_str() {
                pages.insert(title.clone(), page.to_owned());
            }
        }
    }
    collect_url_titles(&list["mapLocations"], &mut pages);
    pages
}

/// Every `title`/`urlTitle` pair anywhere under `value`. See [`parse_pages`].
fn collect_url_titles(value: &serde_json::Value, pages: &mut Pages) {
    match value {
        serde_json::Value::Object(fields) => match (fields.get("title"), fields.get("urlTitle")) {
            (Some(serde_json::Value::String(title)), Some(serde_json::Value::String(page))) => {
                pages.insert(title.clone(), page.clone());
            }
            _ => {
                for nested in fields.values() {
                    collect_url_titles(nested, pages);
                }
            }
        },
        serde_json::Value::Array(entries) => {
            for nested in entries {
                collect_url_titles(nested, pages);
            }
        }
        _ => {}
    }
}

/// Where a world's page sits on the Japanese wiki, given its Japanese name.
pub fn yume2kki_t_url(title: &str) -> String {
    page_url(PAGES.get().unwrap_or(&Pages::new()), title)
}

/// The address, given the overrides to read it against. An override may name an anchor within a
/// page as well as the page, and that `#` has to stay a `#`.
fn page_url(pages: &Pages, title: &str) -> String {
    let page = pages.get(title).map_or(title, String::as_str);
    let (page, anchor) = match page.split_once('#') {
        Some((page, anchor)) => (page, Some(anchor)),
        None => (page, None),
    };
    let mut url = String::from(YUME2KKI_T);
    append_encoded(page, &mut url);
    if let Some(anchor) = anchor {
        url.push('#');
        append_encoded(anchor, &mut url);
    }
    url
}

/// Where everything one person made is listed on the English wiki, which files it as a category.
pub fn author_url(author: &str) -> String {
    let mut url = String::from("https://yume.wiki/Category:");
    append_encoded(author, &mut url);
    url
}

/// The same listing on the Japanese wiki, which tags a world's page with its author's name
/// rather than giving each author a page: what there is to open is the search for the tag.
pub fn yume2kki_t_author_url(author: &str) -> String {
    let mut url = format!("{YUME2KKI_T}::cmd/taglist?tag=");
    // The wiki tags with the name and the honorific together, and this is a query rather than a
    // path, so the name is encoded as one -- a space stays a space rather than becoming the
    // underscore a page name would want.
    append_query_encoded(author, &mut url);
    append_query_encoded("氏", &mut url);
    url
}

fn append_query_encoded(input: &str, output: &mut String) {
    for byte in input.bytes() {
        match byte {
            byte if byte.is_ascii_alphanumeric() => output.push(byte as char),
            b'-' | b'_' | b'.' | b'~' => output.push(byte as char),
            byte => output.push_str(&format!("%{byte:02X}")),
        }
    }
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
    let origin = origin_world(worlds);
    let steps = walkable_steps(worlds);

    // Dijkstra over (gate, depth): the route settled for a world is always its own parent's route
    // with one step added, so the parent chain and the depth cannot disagree. Reversed, because
    // [BinaryHeap] is a max-heap. The world and the parent ride along in the key rather than
    // beside it, so that ties resolve the same way on every run.
    let mut queue =
        std::collections::BinaryHeap::from([std::cmp::Reverse((Gate::Free, 0, origin, origin))]);
    while let Some(std::cmp::Reverse((gate, depth, world, parent))) = queue.pop() {
        if routes.depth[world].is_some() {
            continue;
        }
        routes.depth[world] = Some(depth);
        routes.parents[world] = (world != origin).then_some(parent);
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

/// Where the game starts, and so where every route ends: the room the player wakes up in.
///
/// Named apart from [`origin`] just above, which is the page's own address and has nothing to do
/// with this one -- the two share a module only because the page reads both.
///
/// By name rather than by position, which is how the reference implementation finds it too -- see
/// its `startLocation`. The dump does usually list it first, but only because it was the first
/// world the reference's database ever held and every dump since has kept that order; a dump built
/// from nothing lists the worlds as the wiki does, alphabetically, and starts at `3D Structures
/// Path`. Seeding the walk there leaves nearly every world unreachable, which the graph draws as
/// one flat layer of worlds at no depth at all.
///
/// The first world if the dump has no such title, which is not a state worth failing over: the
/// walk then reports what it can reach from wherever it started, exactly as it did before.
fn origin_world(worlds: &[World]) -> usize {
    worlds
        .iter()
        .position(|world| world.title == ORIGIN)
        .unwrap_or(0)
}

/// The title [`origin`] looks for, as the wiki's English pages spell it.
const ORIGIN: &str = "Urotsuki's Room";

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
    use super::World;

    /// The dump the assertions below are made against.
    ///
    /// Read off disk rather than fetched, so the tests neither need a server running nor say
    /// anything different depending on what one has published since. It is the same document:
    /// `just dreamweaver` writes exactly what it serves to `data.json`.
    fn load() -> super::Dump {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/data.json");
        let json = std::fs::read_to_string(file)
            .expect("data.json is missing; run `just dreamweaver` to write one");
        super::parse(&json).expect("data.json is not the expected world dump")
    }

    /// A connection names its far end by index, so dropping a world moves every reference above
    /// it. Getting that wrong is silent: the graph still draws, with lines to the wrong worlds.
    #[test]
    fn hiding_a_world_renumbers_the_connections_that_outlive_it() {
        let world = |title: &str, secret: bool, out: &[usize]| World {
            title: title.to_owned(),
            title_jp: None,
            author: String::new(),
            image: String::new(),
            added: None,
            map_url: None,
            map_label: None,
            secret,
            connections: out
                .iter()
                .map(|&target_id| super::Connection {
                    target_id,
                    flags: 0,
                    params: Default::default(),
                })
                .collect(),
        };
        // 0 Nexus - 1 Debug Room (secret) - 2 Sofa Room, each joined to both the others.
        let mut worlds = vec![
            world("Nexus", false, &[1, 2]),
            world("Debug Room", true, &[0, 2]),
            world("Sofa Room", false, &[0, 1]),
        ];
        super::hide(&mut worlds);

        let far = |world: &World| -> Vec<usize> {
            world
                .connections
                .iter()
                .map(|connection| connection.target_id)
                .collect()
        };
        assert_eq!(
            worlds
                .iter()
                .map(|world| (world.title.as_str(), far(world)))
                .collect::<Vec<_>>(),
            // Sofa Room has moved down to 1, and the passage each of them wrote to the debug
            // room is gone rather than pointing at whoever took its place.
            [("Nexus", vec![1]), ("Sofa Room", vec![0])]
        );
    }

    /// A connection is one connection however many of its two worlds list it: both of them carry
    /// it, and they agree on which ways round it can be walked.
    #[test]
    fn a_connection_reads_the_same_from_either_end() {
        let worlds = load().worlds;
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
        // The words below are the English ones, and a test is read out in whatever language the
        // machine running it is set to unless it says otherwise.
        crate::i18n::speak_english();
        let worlds = load().worlds;
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
        let worlds = load().worlds;
        assert!(worlds.len() > 1000, "{} worlds", worlds.len());
        assert!(
            worlds
                .iter()
                .all(|w| w.connections.iter().all(|c| c.target_id < worlds.len()))
        );
        assert!(
            worlds.iter().any(|world| world.title == super::ORIGIN),
            "the dump has no {} to start from",
            super::ORIGIN
        );
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
        let worlds = load().worlds;
        let origin = super::origin_world(&worlds);
        assert_eq!(worlds[origin].title, super::ORIGIN);
        let routes = super::canonical_routes(&worlds);
        assert_eq!(routes.depth[origin], Some(0));
        assert!(routes.parents[origin].is_none());
        // The walk has to reach nearly all of it, which is what a seed at the wrong world does
        // not: everything it cannot reach is at no depth, and the graph draws that as one layer.
        let reached = routes.depth.iter().filter(|depth| depth.is_some()).count();
        assert!(
            reached > worlds.len() * 9 / 10,
            "{reached} of {} worlds are reachable",
            worlds.len()
        );
    }

    /// The depth of a world and the route the overlay walks for it are one thing seen twice, so
    /// the walk has to arrive at the origin in exactly as many steps as the depth claims.
    #[test]
    fn depth_is_the_length_of_the_canonical_route() {
        let worlds = load().worlds;
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
            assert_eq!(
                step,
                super::origin_world(&worlds),
                "{} walks back to {step}",
                worlds[world].title
            );
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
        let worlds = load().worlds;
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
        let origin = super::origin_world(&worlds);
        let mut shortest = vec![None; worlds.len()];
        shortest[origin] = Some(0);
        let mut queue = std::collections::VecDeque::from([origin]);
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

    /// A world with no canonical route is drawn as unreached, and the only worlds that may be
    /// one are those the wiki documents no passage touching at all -- pages in the locations
    /// category that carry nothing else, of which it keeps a few. Anything else unreached is a
    /// misread flag closing a passage that is open.
    ///
    /// The one world that really did document a way out and no way in was `Gallery of Me`, which
    /// the dump marks secret and [`hide`] now takes out before any of this: hiding a world hides
    /// the passages into it too, so what is left has to stay reachable.
    #[test]
    fn a_world_is_unreached_only_where_the_wiki_leaves_no_way_in() {
        let worlds = load().worlds;
        let routes = super::canonical_routes(&worlds);
        let mut touched = vec![false; worlds.len()];
        for (at, world) in worlds.iter().enumerate() {
            for connection in &world.connections {
                touched[at] = true;
                touched[connection.target_id] = true;
            }
        }
        let unreachable: Vec<_> = routes
            .depth
            .iter()
            .enumerate()
            .filter(|(_, depth)| depth.is_none())
            .map(|(world, _)| world)
            .filter(|&world| touched[world])
            .map(|world| worlds[world].title.as_str())
            .collect();
        assert_eq!(unreachable, [] as [&str; 0]);
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
        let dump = load();
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
        // The credits join onto the worlds by name, so a change in either spelling would show up
        // as nobody having a Japanese name at all.
        assert!(
            authors
                .iter()
                .filter(|by| by.name.names().count() == 2)
                .count()
                > 10
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
        let versions = load().versions();
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

    /// The Japanese wiki has no page per author: it tags each world's page with who made it, so
    /// the listing is a search for that tag, honorific and all.
    #[test]
    fn an_author_addresses_a_tag_on_the_japanese_wiki() {
        assert_eq!(
            super::yume2kki_t_author_url("185 Go"),
            "https://wikiwiki.jp/yume2kki-t/::cmd/taglist?tag=185%20Go%E6%B0%8F"
        );
        // And where the wiki writes a name differently from the dump, its own writing of it.
        let bean = super::JAPANESE_AUTHOR_OVERRIDES["Bean"];
        assert_eq!(
            super::yume2kki_t_author_url(bean),
            "https://wikiwiki.jp/yume2kki-t/::cmd/taglist?tag=bean%E6%B0%8F"
        );
        assert_eq!(
            super::yume2kki_t_author_url("かえるD"),
            "https://wikiwiki.jp/yume2kki-t/::cmd/taglist?tag=%E3%81%8B%E3%81%88%E3%82%8BD%E6%B0%8F"
        );
    }

    /// The Japanese wiki names a few dozen pages something other than the world on them, so a
    /// Japanese title is looked up before it is addressed.
    #[test]
    fn a_japanese_title_addresses_the_page_the_wiki_files_it_under() {
        // A slice of the list, in each of the shapes it writes a place in: a bare name, a name
        // and the page it is written up on, a map leading to several places, and one leading
        // somewhere different per map it came from.
        let pages = super::parse_pages(
            r#"{
                "urlRoot": "https://wikiwiki.jp/yume2kki-t/",
                "mapLocations": {
                    "0011": "青い腕の通路",
                    "0058": [
                        "昭和路地",
                        { "title": "昭和路地：バスツアー", "urlTitle": "昭和路地" }
                    ],
                    "0230": {
                        "0229": { "title": "製作者の部屋", "urlTitle": "うろつき邸#map0230" },
                        "else": "うろつき邸"
                    }
                },
                "locationUrlTitles": { "ミニゲームA": "ミニゲーム/A" }
            }"#,
        );
        // Its own name, which is the ordinary case, and what an unread list leaves every name at.
        let plain = "https://wikiwiki.jp/yume2kki-t/%E6%B9%96%E4%B8%8A%E3%81%AE%E6%A9%8B";
        assert_eq!(super::page_url(&pages, "湖上の橋"), plain);
        assert_eq!(super::yume2kki_t_url("湖上の橋"), plain);
        // An area written up inside another world's page: the page, and the anchor within it,
        // which stays an anchor rather than being encoded away.
        assert_eq!(
            super::page_url(&pages, "製作者の部屋"),
            "https://wikiwiki.jp/yume2kki-t/%E3%81%86%E3%82%8D%E3%81%A4%E3%81%8D%E9%82%B8#map0230"
        );
        // A place the wiki writes up on a bigger page, and one it files under a path, which is
        // the one shape the list keeps outside its maps. The slash stays a slash.
        assert_eq!(
            super::page_url(&pages, "昭和路地：バスツアー"),
            "https://wikiwiki.jp/yume2kki-t/%E6%98%AD%E5%92%8C%E8%B7%AF%E5%9C%B0"
        );
        assert_eq!(
            super::page_url(&pages, "ミニゲームA"),
            "https://wikiwiki.jp/yume2kki-t/%E3%83%9F%E3%83%8B%E3%82%B2%E3%83%BC%E3%83%A0/A"
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
    /// Every stage names a message that exists. `format` reads an unknown name out as itself, so
    /// a stage whose message was renamed or never written would put `dump-task-worlds` on screen
    /// rather than a sentence.
    #[test]
    fn every_stage_the_server_can_name_is_something_this_app_can_say() {
        super::super::i18n::speak_english();
        for (task, said) in super::STAGES {
            assert_eq!(super::stage(task), Some(said), "{task} names {said}");
            assert_ne!(
                super::super::i18n::format(said, None),
                said,
                "{said} is not a message any language has"
            );
        }
        assert_eq!(super::stage("fetchEffectData"), None, "a stage with no words");
    }

}
