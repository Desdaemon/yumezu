//! The Yume 2kki world graph, as published by yume.wiki and served to this app as `data.json`
//! by `dreamweaver`. See [`load`].
//!
//! The dump carries far more per world than a layout needs -- images, BGM, version history -- so
//! only the fields the visualization draws are deserialized.

use egui_material_icons::{
    MaterialIcon,
    icons::{ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_RANGE, ICON_BLOCK},
};
use serde::Deserialize;

use super::i18n::t;

/// `dreamweaver`, on the machine the app is running on. Fetched rather than compiled in, so a
/// build is not a snapshot of the wiki -- worlds arrive weekly. The cost is a load the first frame
/// has to wait out, which is what the app's loading frame is for.
#[cfg(all(not(target_family = "wasm"), not(feature = "production")))]
const SERVER: &str = "http://127.0.0.1:5000";

/// This project's own `dreamweaver`, deployed: same program, routes and document as a development
/// build reaches locally.
///
/// The reference explorer at `explorer.yume.wiki` answers the same two routes but is not a
/// fallback: its ids are its database's insert order and `dreamweaver`'s are the game's map
/// numbering, so a thumbnail atlas packed against one does not fit the other.
#[cfg(all(not(target_family = "wasm"), feature = "production"))]
const SERVER: &str = "https://explorer.yumemiru.dev";

/// Authors whose yume2kki-t tag is not their name in the dump.
///
/// TODO: corrections that belong on yume.wiki rather than here.
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

#[derive(Clone, Deserialize)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    /// How many cells the thumbnail atlas has to hold. Carried through [`Dump::showing`] rather
    /// than measured again, because a world's cell is its place in the whole dump and a frontier
    /// keeps only part of it. See [`World::cell`].
    #[serde(skip)]
    pub packed: usize,
    /// Newest first. Most added no world at all, so the catalog is built out of
    /// [`Dump::versions`] rather than out of this directly.
    #[serde(rename = "versionInfoData")]
    releases: Vec<Release>,
    /// Read only for the Japanese names: who made what is settled by the worlds themselves, in
    /// [`World::author`].
    #[serde(rename = "authorInfoData", default)]
    credits: Vec<Credit>,
}

#[derive(Clone, Deserialize)]
struct Credit {
    name: String,
    #[serde(rename = "nameJP")]
    name_jp: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct World {
    /// As the wiki's English pages name it, which is also what its page is at: see [`wiki_url`].
    pub title: String,
    /// As the game itself names it, which the dump publishes for all but a few dozen worlds.
    #[serde(rename = "titleJP")]
    title_jp: Option<String>,
    pub author: String,
    /// Where the wiki serves this world's picture from, at the size the wiki holds it. Packed into
    /// the atlas by `tools/atlas`, and fetched from here again once the view comes close enough
    /// for the atlas to have run out of detail: see `detail`.
    #[serde(rename = "filename")]
    pub image: String,
    /// `None` for the few worlds the wiki does not date. See [`Dump::versions`].
    #[serde(rename = "verAdded")]
    added: Option<String>,
    /// The `|`-separated lists the dump publishes, read together and never apart: see
    /// [`World::maps`].
    #[serde(rename = "mapUrl")]
    map_url: Option<String>,
    #[serde(rename = "mapLabel")]
    map_label: Option<String>,
    /// Whether the dump says a reader is not meant to be shown this world -- the debug room, and
    /// whatever else the wiki's own explorer holds back as a spoiler. Read only by [`hide`], which
    /// takes those worlds out before anything else sees the dump.
    #[serde(default)]
    secret: bool,
    pub connections: Vec<Connection>,
    /// Its place among the worlds the app draws, fixed when the dump was read and kept through
    /// every later filtering of it. Written by [`parse`] and by nothing else. See [`World::cell`].
    #[serde(skip)]
    packed_at: usize,
    /// Whether the player has never stood here, in a run only showing them where they have been.
    /// Such a world is drawn only because it touches one they have, so it is named for what it is
    /// rather than where, wears the placeholder picture, and has no maps and no page to open. See
    /// [`Dump::showing`].
    #[serde(skip)]
    unknown: bool,
}

/// What a world the player has not been to is called instead of its name.
///
/// Held rather than built where it is read, because [`Title::show`] hands out a borrow and a
/// message built on the spot would not outlive the call. One slot per language, so choosing a
/// language renames these worlds along with everything else on screen.
fn unvisited() -> &'static str {
    static ENGLISH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static JAPANESE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let held = match super::i18n::speaking_japanese() {
        true => &JAPANESE,
        false => &ENGLISH,
    };
    held.get_or_init(|| t!("unvisited-location"))
}

/// Both names are kept rather than the one being shown, because they are not read for the same
/// thing: the wiki's own pages are named in English whatever is on screen, and a reader may know a
/// world by either.
pub struct Title {
    /// Always there, and always what [`wiki_url`] is given.
    pub en: String,
    jp: Option<String>,
    /// Whether the names are held back, for a world the player has not been to. Both are still
    /// carried -- the graph is built out of the same dump either way -- and neither is shown,
    /// searched, or opened. See [`Dump::showing`].
    unknown: bool,
}

impl Title {
    /// The Japanese name while the app speaks Japanese and the dump has one, and the English one
    /// otherwise. Read rather than stored, so choosing a language renames every world on screen
    /// without anything being rebuilt.
    pub fn show(&self) -> &str {
        if self.unknown {
            // Here rather than at each caller, so no reading of the name can miss it.
            return unvisited();
        }
        match &self.jp {
            Some(jp) if super::i18n::speaking_japanese() => jp,
            _ => &self.en,
        }
    }

    /// Whether this world may be named, searched for, and looked up. See [`Title::unknown`].
    pub fn known(&self) -> bool {
        !self.unknown
    }

    /// Where `needle` falls in this name, and how much name is left over, for whichever name it
    /// fits best. Both are searched whichever is shown, so a reader who knows a world by one does
    /// not have to switch language to find it.
    ///
    /// `needle` must already be lowercased, one being matched against every world.
    pub fn find(&self, needle: &str) -> Option<(usize, usize)> {
        if self.unknown {
            return None;
        }
        self.names()
            .filter_map(|name| Some((name.to_lowercase().find(needle)?, name.len())))
            .min()
    }

    /// The two wikis name their pages after their own name for a world, so a name is only ever
    /// asked of the wiki that wrote it. The few dozen worlds the dump leaves unnamed in Japanese
    /// have no page on the Japanese wiki to open.
    pub fn wiki_url(&self) -> String {
        match &self.jp {
            Some(jp) if super::i18n::speaking_japanese() => yume2kki_t_url(jp),
            _ => wiki_url(&self.en),
        }
    }

    /// English first. What a search reads.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        let named = !self.unknown;
        [
            named.then_some(self.en.as_str()),
            self.jp.as_deref().filter(|_| named),
        ]
        .into_iter()
        .flatten()
    }
}

pub struct Map {
    /// The wiki's own caption, a sentence ("Map of Blood World") rather than a name. On a world
    /// with several maps it is the only thing saying which part each covers.
    pub label: String,
    pub url: String,
}

impl World {
    pub fn titles(&self) -> Title {
        Title {
            en: self.title.clone(),
            jp: self.title_jp.clone(),
            unknown: self.unknown,
        }
    }

    /// `None` for a world the player has not been to, which wears the atlas's placeholder rather
    /// than its own picture. See `thumbnails::cells`.
    pub fn cell(&self) -> Option<usize> {
        (!self.unknown).then_some(self.packed_at)
    }

    /// In the order the wiki lists them, and empty for the few hundred worlds it has drawn none
    /// of. The two lists are published in step but walked together rather than trusted to be: a
    /// map the wiki left uncaptioned is one this would otherwise panic on.
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

#[derive(Clone, Deserialize)]
struct Release {
    name: String,
    /// ISO 8601, of which only the day is ever shown. `None` where the wiki does not date it.
    #[serde(rename = "releaseDate")]
    date: Option<String>,
}

/// A release that added at least one world: what the catalog lists.
pub struct Version {
    /// As the version history names it, or as the worlds do for a release it has no entry for.
    pub name: String,
    /// `YYYY-MM-DD`, empty where the dump does not date it.
    pub released: String,
    /// In world order.
    pub worlds: Vec<usize>,
}

/// Both wikis list one person's whole body of work but keep it in different shapes, so which to
/// open is [`Author::wiki_url`]'s to answer rather than the caller's.
pub struct Author {
    /// Read the same way a world's is: the English one addresses their page, either one finds
    /// them.
    pub name: Title,
    /// In world order.
    pub worlds: Vec<usize>,
}

impl Author {
    pub fn wiki_url(&self) -> String {
        if super::i18n::speaking_japanese() {
            let name = self.name.show();
            yume2kki_t_author_url(JAPANESE_AUTHOR_OVERRIDES.get(name).copied().unwrap_or(name))
        } else {
            author_url(&self.name.en)
        }
    }
}

#[derive(Clone, Deserialize)]
pub struct Connection {
    #[serde(rename = "targetId")]
    pub target_id: usize,
    /// What the connection demands and which way it can be walked, as the bitfield the wiki
    /// publishes. See [`Gate`] and [`flag`].
    #[serde(rename = "type")]
    pub flags: u16,
    /// The wiki's own words for a demand, keyed by the flag making it: the effects to be wearing,
    /// the odds, the season, or the sentence a locked condition is written out as. Most
    /// connections demand nothing and carry none.
    #[serde(rename = "typeParams", default)]
    params: std::collections::HashMap<u16, TypeParams>,
}

/// The dump publishes a Japanese rendering beside the English, but only for the seasons, and
/// those are four fixed words this app names for itself (`gate-seasonal-detail`), so serde skips
/// it.
#[derive(Clone, Deserialize)]
struct TypeParams {
    params: Option<String>,
}

impl Connection {
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

/// The condition on its own is what a route is ordered by; the words are what a reader is told.
#[derive(Clone)]
pub struct Ask {
    pub gate: Gate,
    /// `None` where the wiki writes no words, and always for a direction that is inferred rather
    /// than listed: nothing was written about a listing that does not exist. See
    /// [`walkable_steps`].
    detail: Option<String>,
}

impl Ask {
    #[cfg(test)]
    pub fn free() -> Self {
        Self {
            gate: Gate::Free,
            detail: None,
        }
    }

    /// The wiki's own words where it has any, and the bare name of the condition otherwise. Empty
    /// for a connection that asks nothing.
    pub fn asks(&self) -> String {
        let Some(detail) = self.detail.as_deref() else {
            return self.gate.asks();
        };
        match self.gate {
            // Comma separated rather than joined into a sentence: the wiki does not say whether
            // one effect is enough or all are needed, and an "and" or "or" would settle it here.
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

/// The connection flags this module reads, from the wiki's own `ConnType`. The dump uses two more
/// -- `SHORTCUT` and `TRACKED` -- which describe a connection rather than gating or pointing it.
pub mod flag {
    /// Walkable from the world that lists it, never back.
    pub const ONE_WAY: u16 = 1 << 0;
    /// Walkable only back to the world that lists it, never from it.
    pub const NO_ENTRY: u16 = 1 << 1;
    /// This side opens a connection the far side reports as [`LOCKED`].
    pub const UNLOCK: u16 = 1 << 2;
    pub const LOCKED: u16 = 1 << 3;
    /// Leads to an isolated section of the world at the far end.
    pub const DEAD_END: u16 = 1 << 4;
    /// The far side of a [`DEAD_END`]: the way back is reachable only from that isolated section.
    pub const ISOLATED: u16 = 1 << 5;
    pub const EFFECT: u16 = 1 << 6;
    pub const CHANCE: u16 = 1 << 7;
    pub const LOCKED_CONDITION: u16 = 1 << 8;
    /// Where a shortcut comes out, walked backwards into the shortcut.
    pub const EXIT_POINT: u16 = 1 << 10;
    pub const SEASONAL: u16 = 1 << 11;
}

/// Ordered by how readily a canonical route accepts it: [`Gate::Free`] demands nothing, and the
/// rest follow in the order the wiki's own path finder falls back through them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Gate {
    Free,
    Effect,
    Chance,
    Seasonal,
    /// Unlocked from the opposite entrance.
    Locked,
    /// Also where a shortcut comes out. The reference only ever admits [`flag::EXIT_POINT`]
    /// together with the whole locked group, so it belongs at that group's strictest end rather
    /// than beside [`Gate::Locked`], which would let a route through it sooner than the reference
    /// would.
    LockedCondition,
    /// Leads to an isolated section of the world at the far end.
    DeadEnd,
    /// The far side of a [`Gate::DeadEnd`]: the way back is reachable only from that isolated
    /// section.
    Isolated,
}

impl Gate {
    /// Harshest wins where a connection carries several: they are demands to be met together, so
    /// the route is only as free as its strictest one.
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

    /// The dump writes words for exactly these four; for the rest [`Gate::asks`] is all there is
    /// to read out.
    fn worded(self) -> Option<u16> {
        match self {
            Gate::Effect => Some(flag::EFFECT),
            Gate::Chance => Some(flag::CHANCE),
            Gate::Seasonal => Some(flag::SEASONAL),
            Gate::LockedCondition => Some(flag::LOCKED_CONDITION),
            Gate::Free | Gate::Locked | Gate::DeadEnd | Gate::Isolated => None,
        }
    }

    /// Empty for a condition that asks nothing, which is most of them: a row with nothing after
    /// the title is a way a player can simply walk.
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

/// One connection of a world, from that world's side. A direction with no gate at all is a
/// direction there is no way to walk, which is what makes a connection one-way.
pub struct Step {
    pub world: usize,
    /// `None` where there is no way there.
    pub out: Option<Ask>,
    /// `None` where there is no way back.
    pub back: Option<Ask>,
}

impl Step {
    /// The arrow the panel draws before a title. [`ICON_BLOCK`] is the connection the dump lists
    /// but neither side can walk.
    pub fn arrow(&self) -> MaterialIcon {
        match (self.out.is_some(), self.back.is_some()) {
            (true, true) => ICON_ARROW_RANGE,
            (true, false) => ICON_ARROW_FORWARD,
            (false, true) => ICON_ARROW_BACK,
            (false, false) => ICON_BLOCK,
        }
    }

    /// What the drawing draws as marching dashes.
    pub fn one_way(&self) -> bool {
        self.out.is_some() != self.back.is_some()
    }
}

/// Per world, every world it is joined to, each with what the connection asks in either direction.
///
/// One entry per connection rather than one per listing: a connection is nearly always listed by
/// both of the worlds it joins and is still one connection, so each carries it once. A world's own
/// listings come first, in the dump's order, then the connections only the far side lists.
///
/// Both the lines the visualization draws and the ways onward it offers come from here, so the
/// panel names a connection one-way on exactly the connections drawn that way.
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

/// The prefix the page rewrites out of every picture address, asking its own host instead.
///
/// A page cannot ask the wiki directly: the edge answers a cross-origin request with a challenge
/// page, and the browser sets `Origin` itself and will not let the header `detail::ORIGIN` carries
/// stand in for it -- so what gets the native build its pictures is the one thing a page may not
/// do. The page's own host is same-origin, and proxies on to the wiki.
#[cfg(target_family = "wasm")]
const WIKI_IMAGES: &str = "https://yume.wiki/images/";

/// Built once and kept, every address in the dump getting the same one. Whole rather than the bare
/// path the host actually sees, because these addresses reach the network through `reqwest` rather
/// than the document: it parses each by itself, with no page to resolve a bare path against.
#[cfg(target_family = "wasm")]
fn proxied_images() -> &'static str {
    static PROXIED_IMAGES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PROXIED_IMAGES.get_or_init(|| format!("{}/img/", origin()))
}

/// The only host the page may ask for anything unbidden. See [`proxied_images`] and [`url`].
#[cfg(target_family = "wasm")]
pub(super) fn origin() -> &'static str {
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
/// A page cannot ask the server directly, for the reason [`WIKI_IMAGES`] is rewritten: a request
/// straight at it is cross-origin, which `dreamweaver` sends no `Access-Control-Allow-Origin` to
/// allow, and mixed-content wherever the page is served over https. So the page asks its own host
/// under the same routes -- see the proxies in `Trunk.toml`.
///
/// `production` therefore moves only the native builds; whatever serves the page proxies the
/// routes on to wherever its own `dreamweaver` is.
fn server() -> &'static str {
    #[cfg(not(target_family = "wasm"))]
    return SERVER;
    #[cfg(target_family = "wasm")]
    return origin();
}

fn url() -> String {
    format!("{}/data", server())
}

/// What the server says it is building, as the message to say about it.
///
/// A server rebuilding the dump would rather say so than serve one it is about to replace -- see
/// [`load`] -- so the wait can be a minute, and this is what the loading frame says during it.
/// `GET /pollUpdate` is the reference implementation's route for asking, and `dreamweaver` answers
/// in the same shape.
///
/// `None` for everything that is not a stage this app has words for: a server between syncs, a
/// host with no such route, and the finer stages only the reference server names. All of them mean
/// the plain wait on screen.
pub async fn building() -> Option<&'static str> {
    let url = format!("{}/pollUpdate", server());
    let said = match ask(&url).await {
        Ok(said) => said,
        Err(error) => {
            // Not a warning: a host that does not answer this has nothing to say about what it is
            // building, which is most of them.
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

fn stage(task: &str) -> Option<&'static str> {
    STAGES
        .iter()
        .find(|(named, _)| *named == task)
        .map(|(_, said)| *said)
}

/// What the server calls each stage, and the message that says it. The names on the left come from
/// `dreamweaver`'s `progress`.
const STAGES: [(&str, &str); 4] = [
    ("init", "dump-task-changes"),
    ("fetchWorldData", "dump-task-worlds"),
    ("fetchConnData", "dump-task-passages"),
    ("prepareWorldData", "dump-task-assembling"),
];

async fn ask(url: &str) -> Result<String, super::fetch::Error> {
    Ok(super::fetch::client()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// `Ok(None)` is the server saying it is building one, which is a wait rather than a failure --
/// asking is also what starts that build. [`building`] is what to say meanwhile.
///
/// `Err` carries what to say on screen rather than panicking: a document off the network can fail
/// to arrive, or arrive as something else, and neither is worth taking the window down for.
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

/// `None` for a server that has no dump to send yet. Its own request rather than [`download`],
/// the dump being the one document with an answer that is neither itself nor a failure.
async fn dump(url: &str) -> Result<Option<String>, super::fetch::Error> {
    let response = super::fetch::client().get(url).send().await?;
    // The server is rebuilding. See `dreamweaver`'s `data`.
    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.text().await?))
}

/// For the documents that only ever answer with themselves. See [`dump`] for the one that does
/// not.
async fn download(url: &str) -> Result<String, super::fetch::Error> {
    Ok(super::fetch::client()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

/// Rewrites the addresses this platform cannot use as they stand.
fn parse(json: &str) -> serde_json::Result<Dump> {
    let mut dump = serde_json::from_str::<Dump>(json)?;
    hide(&mut dump.worlds);
    // After the secrets have gone and before anything else can take a world out, this being the
    // numbering `tools/atlas` packed the thumbnails against. Every later reading of the dump
    // carries it rather than counting again.
    dump.packed = dump.worlds.len();
    for (at, world) in dump.worlds.iter_mut().enumerate() {
        world.packed_at = at;
    }
    // Every picture address the app fetches at runtime passes through here and only here. See
    // [`WIKI_IMAGES`]. `tools/atlas` reads `data.json` itself and rightly misses this: it runs at
    // build time and has no page to be on.
    #[cfg(target_family = "wasm")]
    for world in &mut dump.worlds {
        world.image = world.image.replace(WIKI_IMAGES, proxied_images());
        if let Some(urls) = &mut world.map_url {
            // Whole rather than entry by entry: every address in the list carries the same prefix.
            *urls = urls.replace(WIKI_IMAGES, proxied_images());
        }
    }
    Ok(dump)
}

/// Drops the worlds the dump marks secret, and renumbers what is left.
///
/// A connection names the world it leads to by index, so taking a world out is not a matter of
/// skipping it where it is drawn: every index above it moves, and a reference left pointing at the
/// old one would draw a line somewhere else entirely. Done at the one place the dump becomes the
/// app's, so nothing downstream ever sees those worlds.
///
/// `tools/atlas` drops the same worlds, the atlas being packed by index too: the two agree on what
/// a cell counts, or every picture after the first secret is somewhere else's.
fn hide(worlds: &mut Vec<World>) {
    let keep: Vec<bool> = worlds.iter().map(|world| !world.secret).collect();
    let dropped = keep.iter().filter(|kept| !**kept).count();
    if dropped == 0 {
        return;
    }
    log::info!("{dropped} worlds are not for showing");
    retain(worlds, &keep);
}

/// Keeps the worlds `keep` says yes to, renumbers what is left, and drops every connection to a
/// world that has gone.
///
/// The one place a world leaves the graph, so the renumbering is written once however many reasons
/// there are to drop one. See [`hide`] and [`Dump::showing`].
fn retain(worlds: &mut Vec<World>, keep: &[bool]) {
    let mut kept = 0;
    let at: Vec<Option<usize>> = keep
        .iter()
        .map(|&keep| {
            keep.then(|| {
                kept += 1;
                kept - 1
            })
        })
        .collect();

    let mut world = 0;
    worlds.retain(|_| {
        world += 1;
        at[world - 1].is_some()
    });
    for world in worlds.iter_mut() {
        world.connections.retain_mut(|connection| {
            // `flatten` covers both a passage into a dropped world and one out of the dump
            // altogether, which would be a dump disagreeing with itself.
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

/// The version history and the worlds do not spell a release identically: they disagree on case,
/// and on a stray trailing dash.
fn same_release(name: &str) -> String {
    name.trim().trim_end_matches('-').to_lowercase()
}

impl Dump {
    /// How many of the worlds this dump carries a player has stood in.
    ///
    /// The dump is the yardstick rather than YNOproject's own catalog, because the dump is what is
    /// on screen. The count therefore comes out under what YNOproject would say, which records
    /// rooms the wiki keeps no world for.
    pub fn visited(&self, visited: &std::collections::HashSet<String>) -> usize {
        self.worlds
            .iter()
            .filter(|world| visited.contains(&world.title))
            .count()
    }

    /// The same dump cut back to the worlds a player has stood in and the ones a step beyond
    /// them, with nothing further in it at all.
    ///
    /// `visited` is titles as the wiki writes them, which is how YNOproject names the places it
    /// records a player having been. Matched by name, that being the only thing the two lists
    /// share.
    ///
    /// The worlds a step beyond are kept so the graph has an edge to grow at rather than stopping
    /// dead at the last room the player walked into -- a step they could actually take, so a
    /// passage that cannot be walked that way leads nowhere. They are kept as places rather than
    /// worlds, so the graph says there is something there without saying what. See
    /// [`World::unknown`].
    ///
    /// A whole dump rather than a mask over this one, because taking a world out renumbers every
    /// connection above it: the same reason [`hide`] rebuilds rather than skips. One copy per
    /// change of frontier, which is a person pressing a button.
    pub fn showing(&self, visited: &std::collections::HashSet<String>) -> Dump {
        let been: Vec<bool> = self
            .worlds
            .iter()
            .map(|world| visited.contains(&world.title))
            .collect();
        // Outward only. A passage the player could only come back through is not a way onward, so
        // the world at the far end is not on the frontier however plainly the dump joins them.
        let mut shown = been.clone();
        for (from, onward) in walkable_steps(&self.worlds).into_iter().enumerate() {
            if !been[from] {
                continue;
            }
            for (to, _) in onward {
                shown[to] = true;
            }
        }
        log::info!(
            "{} of the {} worlds visited, {} more within reach",
            been.iter().filter(|been| **been).count(),
            been.len(),
            shown
                .iter()
                .zip(&been)
                .filter(|(shown, been)| **shown && !**been)
                .count(),
        );

        let mut worlds = self.worlds.clone();
        for (world, &been) in worlds.iter_mut().zip(&been) {
            if been {
                continue;
            }
            world.unknown = true;
            // Everything that would name the place by another road: the wiki's captions say which
            // world each map is of, and the full-size picture is the world itself.
            world.image = String::new();
            world.map_url = None;
            world.map_label = None;
        }
        retain(&mut worlds, &shown);
        Dump {
            worlds,
            packed: self.packed,
            releases: self.releases.clone(),
            credits: self.credits.clone(),
        }
    }

    /// The releases that added worlds, newest first, each carrying what it added.
    ///
    /// Ordered by the version history, which is the only ordering the dump gives -- version names
    /// do not sort. The handful of releases the worlds name but the history does not know are left
    /// at the end, undated.
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
                    // The day alone: the dump times every release to midnight.
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
        // Stable, so the releases the history does not carry -- all sorting last together -- keep
        // the order the worlds named them in.
        versions.sort_by_key(|version| {
            rank.get(&same_release(&version.name))
                .map_or(usize::MAX, |(at, _)| *at)
        });
        versions
    }

    /// Everyone credited, each carrying their worlds, and, per world, which of them made it.
    ///
    /// Busiest first because only the first few are shown with nothing typed, and the names worth
    /// offering unasked are the ones with the most behind them. Ties break by name, so the order
    /// is fixed rather than however the worlds happened to be listed.
    pub fn authors(&self) -> (Vec<Author>, Vec<usize>) {
        // Only where the two differ: the dump gives a Japanese name for everyone it credits, and
        // for most it is the English one over again, which is nothing to show or search twice.
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
                        // Only worlds are ever held back, never who made one.
                        unknown: false,
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

/// The dump carries no page address, only image ones, so this is built from the title the way the
/// wiki builds it. The titles are ASCII but for a single accent, so the encoder only has to cover
/// the bytes above it.
pub fn wiki_url(title: &str) -> String {
    let mut url = String::from("https://yume.wiki/2kki/");
    append_encoded(title, &mut url);
    url
}

/// A different wiki with pages of its own rather than a translation of the English one.
const YUME2KKI_T: &str = "https://wikiwiki.jp/yume2kki-t/";

/// YNOproject's list of what that wiki calls each place, which is what the game's own client
/// addresses it by. See [`Pages`].
const YNOLOCATIONS: &str =
    "https://raw.githubusercontent.com/ynoproject/ynolocations/refs/heads/master/2kki/ja.json";

/// The few dozen worlds, out of fifteen hundred, whose Japanese page is not simply named after
/// them: an area written up inside another world's page, or a name filed under a longer path.
type Pages = std::collections::HashMap<String, String>;

/// Empty until [`load_pages`] has answered.
static PAGES: std::sync::OnceLock<Pages> = std::sync::OnceLock::new();

/// Fetches the location list and keeps what [`yume2kki_t_url`] reads out of it.
///
/// Started beside the dump rather than on the first Japanese link, because on the page a window
/// opened after the click has passed is a popup the browser blocks. A link clicked in the first
/// moment of a run is therefore addressed without the list, which is the right address for all but
/// the few dozen worlds in it -- and the same reason failure is a warning and nothing more.
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

/// The list names places by map rather than by world, and most of it is which map is which. The
/// pairs read here are nested several ways -- a map may name one place, several, or a different one
/// per map it leads on from -- and none of that nesting matters, so it is walked, not modelled.
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

pub fn yume2kki_t_url(title: &str) -> String {
    page_url(PAGES.get().unwrap_or(&Pages::new()), title)
}

/// An override may name an anchor within a page as well as the page, and that `#` has to stay one.
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

/// The English wiki files one person's work as a category.
pub fn author_url(author: &str) -> String {
    let mut url = String::from("https://yume.wiki/Category:");
    append_encoded(author, &mut url);
    url
}

/// The Japanese wiki tags a world's page with its author's name rather than giving each author a
/// page, so what there is to open is the search for the tag.
pub fn yume2kki_t_author_url(author: &str) -> String {
    let mut url = format!("{YUME2KKI_T}::cmd/taglist?tag=");
    // The wiki tags with the name and the honorific together. A query rather than a path, so a
    // space stays a space rather than becoming the underscore a page name would want.
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
    /// Per world, how many connections its canonical route is long, and `None` where unreachable.
    /// Measured here rather than read from the dump's own `depth`, so it agrees with the
    /// connections this visualization actually knows about.
    pub depth: Vec<Option<u32>>,
}

impl Routes {
    /// Per world, how many worlds' canonical route home passes through it: how much of the game it
    /// is the way to, which is what the node sizes show. Sizing reads it through a logarithmic
    /// curve, so a leaf, a small hub and a gateway separate while the origin, which everything
    /// hangs off, stays on the same scale as the rest.
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

    /// A world and every world that hangs off it, shallowest first so a reader walks it outward.
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

/// Walks the route to every world a player could actually be expected to walk.
///
/// Routes are ordered by the harshest [`Gate`] anywhere along them and only then by length, so an
/// unconditional route wins however long it is, and a world whose every route is conditional takes
/// the mildest available. That ordering is why the depth reported here is the higher, honest one:
/// a locked or chance-gated shortcut no longer makes a world look shallow.
///
/// Directed, like the lines the visualization draws: a connection the player can only walk one way
/// is not a way in, so it cannot carry a route.
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
    // with one step added, so the parent chain and the depth cannot disagree. Reversed because
    // `BinaryHeap` is a max-heap. The world and the parent ride along in the key rather than
    // beside it, so ties resolve the same way on every run.
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

/// Where the game starts, and so where every route ends: the room the player wakes up in. Not the
/// page's own [`origin`] elsewhere in this module, which is an address.
///
/// By name rather than by position, as the reference implementation finds it too. The dump usually
/// lists it first, but only because it was the first world the reference's database ever held: a
/// dump built from nothing lists the worlds alphabetically and starts at `3D Structures Path`, and
/// seeding the walk there leaves nearly every world unreachable -- one flat layer at no depth.
///
/// The first world if the dump has no such title, which is not worth failing over: the walk then
/// reports what it can reach from wherever it started.
fn origin_world(worlds: &[World]) -> usize {
    worlds
        .iter()
        .position(|world| world.title == ORIGIN)
        .unwrap_or(0)
}

/// As the wiki's English pages spell it.
const ORIGIN: &str = "Urotsuki's Room";

/// Every step a player can take, as a directed adjacency list carrying what each demands.
///
/// A connection is nearly always listed by both of the worlds it joins, each with its own flags,
/// and those two listings are the two directions. Where only one side lists it, the other is
/// inferred the way the wiki's own path finder infers it: [`flag::ONE_WAY`] means there is no way
/// back, and [`flag::UNLOCK`] means the way back is [`Gate::Locked`].
///
/// The routes walk this directly, and [`connections`] is the same thing read pairwise, so a line
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

    // Read off disk rather than fetched, so the tests neither need a server running nor say
    // anything different depending on what one has published since. `just dreamweaver` writes
    // exactly what it serves to `data.json`.
    fn load() -> super::Dump {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/data.json");
        let json = std::fs::read_to_string(file)
            .expect("data.json is missing; run `just dreamweaver` to write one");
        super::parse(&json).expect("data.json is not the expected world dump")
    }

    // Getting the renumbering wrong is silent: the graph still draws, with lines to the wrong
    // worlds.
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
            packed_at: 0,
            unknown: false,
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
            // Sofa Room has moved down to 1, and the passage each wrote to the debug room is gone
            // rather than pointing at whoever took its place.
            [("Nexus", vec![1]), ("Sofa Room", vec![0])]
        );
    }

    #[test]
    fn a_frontier_keeps_one_step_past_what_was_visited() {
        // Each links only the step onward, so the step back is read off the far side's listing.
        let mut dump = chain(&["Nexus", "Sofa Room", "Far Room", "Farther Room"], 0);
        dump.packed = dump.worlds.len();
        for (at, world) in dump.worlds.iter_mut().enumerate() {
            world.packed_at = at;
        }
        let visited = ["Nexus".to_owned()].into_iter().collect();
        let shown = dump.showing(&visited);

        assert_eq!(
            shown
                .worlds
                .iter()
                .map(|world| (world.title.as_str(), world.cell()))
                .collect::<Vec<_>>(),
            // Kept as a place rather than a world: no cell of its own, so it wears the
            // placeholder.
            [("Nexus", Some(0)), ("Sofa Room", None)]
        );
        // Sofa Room's step onward led to a world no longer there, so it is gone rather than
        // pointing at whoever took its place.
        assert_eq!(
            shown
                .worlds
                .iter()
                .map(|world| world
                    .connections
                    .iter()
                    .map(|connection| connection.target_id)
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>(),
            [vec![1], vec![]]
        );
        // The atlas is packed against the whole dump, so a frontier keeps part of that numbering
        // rather than one of its own.
        assert_eq!(shown.packed, 4);
    }

    #[test]
    fn a_frontier_does_not_reach_through_a_passage_it_cannot_be_walked_down() {
        // Every listed step is one-way *into* the world listing it, so from Nexus there is no way
        // onward at all.
        let mut dump = chain(
            &["Nexus", "Sofa Room", "Far Room", "Farther Room"],
            super::flag::NO_ENTRY,
        );
        dump.packed = dump.worlds.len();
        let visited = ["Nexus".to_owned()].into_iter().collect();

        assert_eq!(
            dump.showing(&visited)
                .worlds
                .iter()
                .map(|world| world.title.as_str())
                .collect::<Vec<_>>(),
            // Sofa Room is a step the player cannot take.
            ["Nexus"]
        );
    }

    /// Worlds in a row, each listing the step onward under `flags`.
    fn chain(titles: &[&str], flags: u16) -> super::Dump {
        let worlds = titles
            .iter()
            .enumerate()
            .map(|(at, title)| World {
                title: (*title).to_owned(),
                title_jp: None,
                author: String::new(),
                image: format!("{title}.png"),
                added: None,
                map_url: None,
                map_label: None,
                secret: false,
                packed_at: 0,
                unknown: false,
                connections: (at + 1 < titles.len())
                    .then(|| super::Connection {
                        target_id: at + 1,
                        flags,
                        params: Default::default(),
                    })
                    .into_iter()
                    .collect(),
            })
            .collect();
        super::Dump {
            worlds,
            packed: 0,
            releases: Vec::new(),
            credits: Vec::new(),
        }
    }

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

    #[test]
    fn a_condition_is_read_out_in_the_wikis_own_words() {
        // The words asserted below are the English ones.
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

    // A shape change in the dump would otherwise break the visualization silently.
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

    // The mildest demand would understate what the player has to have done.
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

    #[test]
    fn the_origin_roots_the_route_tree() {
        let worlds = load().worlds;
        let origin = super::origin_world(&worlds);
        assert_eq!(worlds[origin].title, super::ORIGIN);
        let routes = super::canonical_routes(&worlds);
        assert_eq!(routes.depth[origin], Some(0));
        assert!(routes.parents[origin].is_none());
        // What a seed at the wrong world does not do: everything unreached is at no depth, and
        // the graph draws that as one layer.
        let reached = routes.depth.iter().filter(|depth| depth.is_some()).count();
        assert!(
            reached > worlds.len() * 9 / 10,
            "{reached} of {} worlds are reachable",
            worlds.len()
        );
    }

    // The depth and the route the overlay walks are one thing seen twice.
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

    // Compared against a walk ignoring both direction and conditions, which is what this used to
    // report.
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
        // Not a threshold worth tuning: it only has to prove the rule bites at all.
        assert!(deeper > worlds.len() / 4, "only {deeper} worlds moved");
    }

    // Anything unreached that the wiki documents a passage to is a misread flag closing a passage
    // that is open. The one world that really did document a way out and no way in was `Gallery of
    // Me`, which the dump marks secret and `hide` takes out along with the passages into it.
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

        // Not every world: a handful are undated, and belong to no release.
        let versions = dump.versions();
        let dated: usize = versions.iter().map(|version| version.worlds.len()).sum();
        assert!(dated > dump.worlds.len() * 9 / 10, "{dated} dated");
        assert!(versions.iter().all(|version| !version.worlds.is_empty()));
    }

    // The join is made on more than equality, or a release the two halves spell differently comes
    // out twice, undated, at the end.
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
        // Newest first and dated, which is what the history's own order buys.
        assert!(versions[0].released > versions[1].released);
    }

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

    #[test]
    fn a_japanese_title_addresses_the_page_the_wiki_files_it_under() {
        // A slice of the list, in each of the shapes it writes a place in: a bare name, a name and
        // the page it is written up on, a map leading to several places, and one leading somewhere
        // different per map it came from.
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
        // Its own name, the ordinary case, and what an unread list leaves every name at.
        let plain = "https://wikiwiki.jp/yume2kki-t/%E6%B9%96%E4%B8%8A%E3%81%AE%E6%A9%8B";
        assert_eq!(super::page_url(&pages, "湖上の橋"), plain);
        assert_eq!(super::yume2kki_t_url("湖上の橋"), plain);
        // An area written up inside another world's page: the anchor stays an anchor rather than
        // being encoded away.
        assert_eq!(
            super::page_url(&pages, "製作者の部屋"),
            "https://wikiwiki.jp/yume2kki-t/%E3%81%86%E3%82%8D%E3%81%A4%E3%81%8D%E9%82%B8#map0230"
        );
        // A place written up on a bigger page, and one filed under a path -- the one shape the list
        // keeps outside its maps. The slash stays a slash.
        assert_eq!(
            super::page_url(&pages, "昭和路地：バスツアー"),
            "https://wikiwiki.jp/yume2kki-t/%E6%98%AD%E5%92%8C%E8%B7%AF%E5%9C%B0"
        );
        assert_eq!(
            super::page_url(&pages, "ミニゲームA"),
            "https://wikiwiki.jp/yume2kki-t/%E3%83%9F%E3%83%8B%E3%82%B2%E3%83%BC%E3%83%A0/A"
        );
    }

    // The subtree carries the world it is rooted at and nothing off to the side, however near.
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
    // `format` reads an unknown name out as itself, so a stage whose message was renamed or never
    // written would put `dump-task-worlds` on screen rather than a sentence.
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
        assert_eq!(
            super::stage("fetchEffectData"),
            None,
            "a stage with no words"
        );
    }
}
