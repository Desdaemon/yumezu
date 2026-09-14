//! The Yume 2kki world graph, as published by yume.wiki and served to this app as `data.json` by
//! `dreamweaver`. See [`load`].
//!
//! The dump carries far more per world than a layout needs, so only the fields the visualization
//! draws are deserialized.

use egui_material_icons::{
    MaterialIcon,
    icons::{ICON_ARROW_BACK, ICON_ARROW_FORWARD, ICON_ARROW_RANGE, ICON_BLOCK},
};
use serde::Deserialize;
// The connection and the walk are the routing crate's, so the program that publishes the dump
// reads them exactly as this one does.
use yumezu_routing::walkable_steps;
pub use yumezu_routing::{
    Ask, Connection, Demand, Gate, Routes, Step, Way, connections, hub_world, origin_world,
    routes_from, step_asks, ways,
};

use super::i18n::t;

/// This project's own `dreamweaver`, deployed. Fetched rather than compiled in, so a build is not a
/// snapshot of the wiki -- worlds arrive weekly.
///
/// The reference explorer at `explorer.yume.wiki` answers the same two routes but is not a
/// fallback: it publishes no `cell`, so every world would come out of it wearing the
/// placeholder.
///
/// A local `dreamweaver` is reached through the page instead -- `just serve` proxies to it, see
/// `Trunk.toml`. Only trunk serves `static/`, so a native build has no local host to ask.
#[cfg(not(target_family = "wasm"))]
const SERVER: &str = "https://explorer.yumemiru.dev";

/// Authors whose yume2kki-t tag is not their name in the dump.
///
/// TODO: corrections that belong on yume.wiki rather than here.
static JAPANESE_AUTHOR_OVERRIDES: [(&str, &str); 10] = [
    ("Bean", "bean"),
    ("窯良", "窯良(oneirokamara)"),
    ("コンテンツ", "kontentsu"),
    ("Ouri", "ouri"),
    ("sniperbob", "Sniperbob"),
    ("Mokaccino", "Moka"),
    ("◆gH8PoF17WqX", "Ferdy"),
    ("Nightmare", "†Nightmare†"),
    ("tKp9vEGEfhCD", "◆tKp9vEGEfhCD"),
    ("Nulsdodage", "nulsdodage"),
];

/// The wiki's thirty-five effects, in the order the game gives them, and the only names it writes
/// a condition's effects in. What each is called on screen is `effect-<name>` in the locale files.
pub static EFFECTS: [&str; 35] = [
    "Bike",
    "Boy",
    "Chainsaw",
    "Lantern",
    "Fairy",
    "Spacesuit",
    "Glasses",
    "Rainbow",
    "Wolf",
    "Eyeball Bomb",
    "Telephone",
    "Maiko",
    "Twintails",
    "Penguin",
    "Insect",
    "Spring",
    "Invisible",
    "Gakuran",
    "Plaster Cast",
    "Stretch",
    "Haniwa",
    "Trombone",
    "Cake",
    "Child",
    "Red Riding Hood",
    "Tissue",
    "Bat",
    "Polygon",
    "Teru Teru Bozu",
    "Marginal",
    "Drum",
    "Grave",
    "Crossing",
    "Bunny Ears",
    "Dice",
];

/// The message naming an effect on screen, from the name the wiki writes it by.
pub fn effect_message(effect: &str) -> String {
    format!("effect-{}", effect.to_lowercase().replace(' ', "-"))
}

fn japanese_author(name: &str) -> &str {
    JAPANESE_AUTHOR_OVERRIDES
        .iter()
        .find_map(|&(from, to)| (from == name).then_some(to))
        .unwrap_or(name)
}

#[derive(Clone, Deserialize)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    /// Newest first. Most added no world at all, so the catalog is built out of
    /// [`Dump::versions`] rather than this directly.
    #[serde(rename = "versionInfoData")]
    releases: Vec<Release>,
    /// Read only for the Japanese names: who made what is settled by the worlds themselves, in
    /// [`World::author`].
    #[serde(rename = "authorInfoData", default)]
    credits: Vec<Credit>,
    /// When the dump was built, ISO 8601.
    #[serde(rename = "lastUpdate", default)]
    pub last_update: Option<String>,
    /// When the wiki was last read whole rather than only where it had changed: a dump can be
    /// fresh and still be missing an edit an incremental read did not ask about.
    #[serde(rename = "lastFullUpdate", default)]
    pub last_full_update: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Credit {
    name: String,
    #[serde(rename = "nameJP")]
    name_jp: Option<String>,
}

#[derive(Clone, Deserialize)]
pub struct World {
    /// As the wiki's English pages name it, which is also what its page is at. See [`wiki_url`].
    pub title: String,
    /// As the game itself names it, which the dump publishes for all but a few dozen worlds.
    #[serde(rename = "titleJP")]
    title_jp: Option<String>,
    pub author: String,
    /// Where the wiki serves this world's picture from, at the size the wiki holds it. Packed into
    /// the atlas by `tools/atlas`, and fetched from here again once the view is close enough for
    /// the atlas to have run out of detail. See `detail`.
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
    /// Whether the dump says a reader is not meant to be shown this world. Read only by [`hide`],
    /// which takes those worlds out before anything else sees the dump.
    #[serde(default)]
    secret: bool,
    pub connections: Vec<Connection>,
    /// Its cell in the thumbnail atlas, as the server hands it out: once, and never moved again,
    /// so an atlas packed before this world existed is still right about every world it does
    /// hold. Read through [`World::cell`].
    ///
    /// `None` from a server old enough not to publish one, which costs that run its thumbnails
    /// rather than giving every world somebody else's.
    #[serde(default)]
    cell: Option<usize>,
    /// Whether the player has never stood here, in a run showing only where they have been. Such a
    /// world is drawn because it touches one they have: named for what it is rather than where,
    /// wearing the placeholder picture, with no maps and no page to open. See [`Dump::showing`].
    #[serde(skip)]
    unknown: bool,
}

/// What a world the player has not been to is called instead of its name.
///
/// Held rather than built where it is read, because [`Title::show`] hands out a borrow that a
/// message built on the spot would not outlive. One slot per language.
fn unvisited() -> &'static str {
    static ENGLISH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static JAPANESE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let held = match super::i18n::speaking_japanese() {
        true => &JAPANESE,
        false => &ENGLISH,
    };
    held.get_or_init(|| t!("unvisited-location"))
}

/// Both names are kept rather than the one on screen: the wiki's own pages are named in English
/// whatever the language, and a reader may know a world by either.
pub struct Title {
    /// Always there, and always what [`wiki_url`] is given.
    pub en: String,
    jp: Option<String>,
    /// Whether the names are held back, for a world the player has not been to. Both are still
    /// carried, and neither is shown, searched, or opened.
    unknown: bool,
}

impl Title {
    /// Read rather than stored, so choosing a language renames every world on screen without
    /// anything being rebuilt.
    pub fn show(&self) -> &str {
        if self.unknown {
            return unvisited();
        }
        match &self.jp {
            Some(jp) if super::i18n::speaking_japanese() => jp,
            _ => &self.en,
        }
    }

    /// Whether this world may be named, searched for, and looked up.
    pub fn known(&self) -> bool {
        !self.unknown
    }

    /// Where `needle` falls in this name, and how much name is left over, for whichever name it
    /// fits best. Both are searched whichever is shown, so a reader who knows a world by one does
    /// not have to switch language to find it.
    ///
    /// `needle` must already be lowercased.
    pub fn find(&self, needle: &str) -> Option<(usize, usize)> {
        if self.unknown {
            return None;
        }
        self.names()
            .filter_map(|name| Some((name.to_lowercase().find(needle)?, name.len())))
            .min()
    }

    /// The two wikis name their pages after their own name for a world, so a name is only ever
    /// asked of the wiki that wrote it.
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

impl yumezu_routing::World for World {
    fn title(&self) -> &str {
        &self.title
    }

    fn connections(&self) -> &[Connection] {
        &self.connections
    }
}

impl World {
    pub fn titles(&self) -> Title {
        Title {
            en: self.title.clone(),
            jp: self.title_jp.clone(),
            unknown: self.unknown,
        }
    }

    /// `None` for a world the player has not been to, which wears the atlas's placeholder, and for
    /// one the dump gives no cell.
    pub fn cell(&self) -> Option<usize> {
        self.cell.filter(|_| !self.unknown)
    }

    /// In the order the wiki lists them, and empty for the few hundred worlds it has drawn none
    /// of. The two lists are walked together rather than trusted to be in step: a map the wiki left
    /// uncaptioned would otherwise panic here.
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

/// Both wikis list one person's whole body of work but in different shapes, so which to open is
/// [`Author::wiki_url`]'s to answer rather than the caller's.
pub struct Author {
    /// The English one addresses their page; either one finds them.
    pub name: Title,
    /// In world order.
    pub worlds: Vec<usize>,
}

impl Author {
    pub fn wiki_url(&self) -> String {
        if super::i18n::speaking_japanese() {
            let name = self.name.show();
            yume2kki_t_author_url(japanese_author(name))
        } else {
            author_url(&self.name.en)
        }
    }
}

/// Empty for a condition that asks nothing: a row with nothing after the title is a way a player
/// can walk unconditionally.
pub fn gate_asks(gate: Gate) -> String {
    match gate {
        Gate::Free => String::new(),
        Gate::Effect => t!("gate-effect"),
        Gate::Chance => t!("gate-chance"),
        Gate::Seasonal => t!("gate-seasonal"),
        Gate::Locked => t!("gate-locked"),
        Gate::LockedCondition | Gate::Revisit => t!("gate-locked-condition"),
        Gate::ExitPoint => t!("gate-exit-point"),
        Gate::DeadEnd => t!("gate-dead-end"),
        Gate::Isolated => t!("gate-isolated"),
    }
}

/// Every condition the connection carries, harshest first, a line each. Empty for a connection
/// that asks nothing.
///
/// All of them rather than the harshest alone: a way that is locked *and* wants an effect is not
/// walkable by meeting either, and a reader told only the lock would go and fail.
pub fn asks(ask: &Ask) -> String {
    ask.demands
        .iter()
        .map(demand_asks)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The wiki's own words where it has any, and the bare name of the condition otherwise.
fn demand_asks(demand: &Demand) -> String {
    let Some(detail) = demand.detail.as_deref() else {
        return gate_asks(demand.gate);
    };
    match demand.gate {
        Gate::Effect => t!("gate-effect-detail", effects = effects(detail)),
        Gate::Chance => t!("gate-chance-detail", chance = detail),
        Gate::Seasonal => t!("gate-seasonal-detail", season = detail),
        // The wiki's own sentence, which it writes in English and publishes no Japanese for.
        Gate::LockedCondition | Gate::Revisit => detail.to_owned(),
        _ => gate_asks(demand.gate),
    }
}

/// What a connection asks in effects, in whichever language is being spoken.
///
/// Listed as the wiki lists them and joined no further: the wiki does not say whether one effect is
/// enough or all are needed, and an "and" or "or" would settle it here.
fn effects(detail: &str) -> String {
    corrected(detail)
        .split(',')
        .map(|listed| named_effects(listed.trim()))
        .collect::<Vec<_>>()
        .join(&t!("effect-separator"))
}

/// An entity the dump leaves unescaped, and a name it writes two ways.
///
/// TODO: corrections that belong on yume.wiki rather than here.
fn corrected(detail: &str) -> String {
    detail
        .replace("&comma;", ",")
        .replace("Teru Teru Bōzu", "Teru Teru Bozu")
}

/// The effect names within the wiki's own words, as the locale files name them. What lies between
/// them stays as the wiki wrote it -- a separator, an "or", a note naming where an effect is worn
/// -- because which of those it means is the wiki's to say.
fn named_effects(detail: &str) -> String {
    let mut said = String::with_capacity(detail.len());
    let mut at = 0;
    while at < detail.len() {
        let rest = &detail[at..];
        // A name only where a word starts and ends, so `Springfield` is not the Spring effect.
        let starting = detail[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric());
        let named = starting
            .then(|| {
                EFFECTS.iter().find(|effect| {
                    rest.get(..effect.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(effect))
                        && !rest[effect.len()..].starts_with(char::is_alphanumeric)
                })
            })
            .flatten();
        match named {
            Some(effect) => {
                said.push_str(&super::i18n::format(&effect_message(effect), None));
                at += effect.len();
            }
            None => {
                let next = rest.chars().next().expect("a non-empty string has a char");
                said.push(next);
                at += next.len_utf8();
            }
        }
    }
    said
}

/// One glyph per condition, harshest first, so a way that is locked *and* wants an effect reads as
/// both. Empty for a connection that asks nothing.
pub fn asks_emoji(ask: &Ask) -> String {
    ask.demands.iter().map(demand_emoji).collect()
}

fn demand_emoji(demand: &Demand) -> &'static str {
    match demand.gate {
        Gate::Free => "",
        Gate::Effect => "✨",
        Gate::Chance => "🍀",
        Gate::Locked => "🔒",
        Gate::LockedCondition | Gate::Revisit => "🔐",
        Gate::ExitPoint => "🚪",
        Gate::DeadEnd => "↩",
        Gate::Isolated => "🚩",
        Gate::Seasonal => match demand.detail.as_deref() {
            Some("Spring") => "🌸",
            Some("Summer") => "☀",
            Some("Fall") => "🍂",
            Some("Winter") => "❄",
            _ => "🗓",
        },
    }
}

/// [`ICON_BLOCK`] is the connection the dump lists but neither side can walk.
pub fn arrow(step: &Step) -> MaterialIcon {
    match (step.out.is_some(), step.back.is_some()) {
        (true, true) => ICON_ARROW_RANGE,
        (true, false) => ICON_ARROW_FORWARD,
        (false, true) => ICON_ARROW_BACK,
        (false, false) => ICON_BLOCK,
    }
}

/// The prefix the page rewrites out of every picture address, asking its own host instead.
///
/// A page cannot ask the wiki directly: the edge answers a cross-origin request with a challenge
/// page, and the browser sets `Origin` itself rather than letting the header `detail::ORIGIN`
/// carries stand in -- so what gets the native build its pictures is the one thing a page may not
/// do. The page's own host is same-origin and proxies on to the wiki.
#[cfg(target_family = "wasm")]
const WIKI_IMAGES: &str = "https://yume.wiki/images/";

/// Whole rather than the bare path the host sees, because these addresses reach the network
/// through `reqwest` rather than the document, which has no page to resolve a bare path against.
#[cfg(target_family = "wasm")]
fn proxied_images() -> &'static str {
    static PROXIED_IMAGES: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PROXIED_IMAGES.get_or_init(|| format!("{}/img/", origin()))
}

/// The only host the page may ask for anything unbidden.
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
/// A request straight at the server is cross-origin, which `dreamweaver` sends no
/// `Access-Control-Allow-Origin` to allow, and mixed-content wherever the page is served over
/// https. So the page asks its own host under the same routes -- see the proxies in `Trunk.toml`.
///
/// Also where [`super::thumbnails`] reads the atlas from, which is why this is not private.
pub(super) fn server() -> &'static str {
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
/// A server rebuilding the dump says so rather than serving one it is about to replace, so the wait
/// can be a minute. `GET /pollUpdate` is the reference implementation's route for asking, and
/// `dreamweaver` answers with the same JSON.
///
/// `None` for anything that is not a stage this app has words for -- a server between syncs, a host
/// with no such route, the finer stages only the reference server names -- all of which mean the
/// plain wait on screen.
pub async fn building() -> Option<&'static str> {
    let url = format!("{}/pollUpdate", server());
    let said = match ask(&url).await {
        Ok(said) => said,
        Err(error) => {
            // Not a warning: most hosts have nothing to say about what they are building.
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

/// The names on the left come from `dreamweaver`'s `progress`.
const STAGES: [(&str, &str); 4] = [
    ("init", "dump-task-changes"),
    ("fetchWorldData", "dump-task-worlds"),
    ("fetchConnData", "dump-task-connections"),
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
/// `Err` carries what to say on screen rather than panicking: a document off the network failing to
/// arrive, or arriving as something else, is not worth taking the window down for.
pub async fn load(revealed: bool) -> Result<Option<Dump>, String> {
    let url = url();
    let Some(json) = dump(&url)
        .await
        .map_err(|error| format!("cannot reach {url}: {error}"))?
    else {
        return Ok(None);
    };
    parse(&json, revealed)
        .map(Some)
        .map_err(|error| format!("{url} is not the expected world dump: {error}"))
}

/// `None` for a server that has no dump to send yet. Its own request rather than [`download`], the
/// dump being the one document with an answer that is neither itself nor a failure.
async fn dump(url: &str) -> Result<Option<String>, super::fetch::Error> {
    let response = super::fetch::client().get(url).send().await?;
    // The server is rebuilding. See `dreamweaver`'s `data`.
    if response.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Ok(None);
    }
    Ok(Some(response.error_for_status()?.text().await?))
}

/// For the documents that only ever answer with themselves.
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
fn parse(json: &str, revealed: bool) -> serde_json::Result<Dump> {
    let mut dump = serde_json::from_str::<Dump>(json)?;
    hide(&mut dump.worlds, revealed);
    // Every picture address the app fetches at runtime passes through here and only here.
    // `tools/atlas` reads `data.json` itself and rightly misses this: it runs at build time and has
    // no page to be on.
    #[cfg(target_family = "wasm")]
    for world in &mut dump.worlds {
        world.image = world.image.replace(WIKI_IMAGES, proxied_images());
        if let Some(urls) = &mut world.map_url {
            // Whole rather than entry by entry: every address carries the same prefix.
            *urls = urls.replace(WIKI_IMAGES, proxied_images());
        }
    }
    Ok(dump)
}

const MYSTERY_TEXT: &str = "20050726";

#[derive(Default)]
pub struct Code {
    at: usize,
    done: bool,
}

impl Code {
    pub fn typed(&mut self, key: char) {
        self.at = match MYSTERY_TEXT.chars().nth(self.at) == Some(key) {
            true => self.at + 1,
            // Begun again from this key rather than from the next: a false start is usually the
            // code's own first key struck twice.
            false => usize::from(MYSTERY_TEXT.starts_with(key)),
        };
        if self.at == MYSTERY_TEXT.len() {
            self.at = 0;
            self.done = true;
        }
    }

    /// Whatever was half typed, dropped: the keys have gone to a box on the panel, where they are
    /// somebody's search rather than this.
    pub fn forget(&mut self) {
        self.at = 0;
    }

    pub fn taken(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

/// Drops the worlds the dump marks secret, and renumbers what is left.
///
/// A connection names the world it leads to by index, so taking one out moves every index above it
/// and a reference left pointing at the old one draws a line somewhere else entirely. Done where
/// the dump becomes the app's, so nothing downstream ever sees those worlds.
fn hide(worlds: &mut Vec<World>, revealed: bool) {
    if revealed {
        return;
    }
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
/// there are to drop one.
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
            // `flatten` covers a connection into a dropped world and one out of the dump
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
    /// The dump is the yardstick rather than YNOproject's own catalog, because the dump is what is
    /// on screen. The count therefore comes out under what YNOproject would say, which records
    /// rooms the wiki keeps no world for.
    pub fn visited(&self, visited: &std::collections::HashSet<String>) -> usize {
        self.worlds
            .iter()
            .filter(|world| visited.contains(&world.title))
            .count()
    }

    /// The same dump cut back to the worlds a player has stood in and the ones a step beyond.
    ///
    /// `visited` is titles as the wiki writes them, which is how YNOproject names the places it
    /// records a player having been; the name is the only thing the two lists share.
    ///
    /// The worlds a step beyond are a step the player could actually take, so a connection that
    /// cannot
    /// be walked that way leads nowhere. They are kept as places rather than worlds, so the graph
    /// says there is something there without saying what. See [`World::unknown`].
    ///
    /// A whole dump rather than a mask, because taking a world out renumbers every connection above
    /// it -- the same reason [`hide`] rebuilds rather than skips. One copy per change of frontier,
    /// which is a person pressing a button.
    pub fn showing(&self, visited: &std::collections::HashSet<String>) -> Dump {
        let been: Vec<bool> = self
            .worlds
            .iter()
            .map(|world| visited.contains(&world.title))
            .collect();
        // Outward only: a connection the player could only come back through is not a way onward.
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
            releases: self.releases.clone(),
            credits: self.credits.clone(),
            last_update: self.last_update.clone(),
            last_full_update: self.last_full_update.clone(),
        }
    }

    /// The releases that added worlds, newest first, each carrying what it added.
    ///
    /// Ordered by the version history, the only ordering the dump gives: version names do not sort.
    /// The handful of releases the worlds name but the history does not know go last, undated.
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
    /// Busiest first, because only the first few are shown with nothing typed. Ties break by name,
    /// so the order is fixed rather than however the worlds happened to be listed.
    pub fn authors(&self) -> (Vec<Author>, Vec<usize>) {
        // Only where the two differ: the dump repeats the English name for most people, which is
        // nothing to show or search twice.
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
/// wiki builds it.
pub fn wiki_url(title: &str) -> String {
    let mut url = String::from("https://yume.wiki/2kki/");
    append_encoded(title, &mut url);
    url
}

/// A different wiki with pages of its own rather than a translation of the English one.
const YUME2KKI_T: &str = "https://wikiwiki.jp/yume2kki-t/";

/// YNOproject's list of what that wiki calls each place, which is what the game's own client
/// addresses it by.
const YNOLOCATIONS: &str =
    "https://raw.githubusercontent.com/ynoproject/ynolocations/refs/heads/master/2kki/ja.json";

/// The few dozen worlds, out of fifteen hundred, whose Japanese page is not named after them: an
/// area written up inside another world's page, or a name filed under a longer path.
type Pages = std::collections::HashMap<String, String>;

/// Empty until [`load_pages`] has answered.
static PAGES: std::sync::OnceLock<Pages> = std::sync::OnceLock::new();

/// Started beside the dump rather than on the first Japanese link, because a window opened after
/// the click has passed is a popup the browser blocks. A link clicked in the first moment of a run
/// is therefore addressed without the list, which is right for all but the few dozen worlds in it
/// -- and the same reason failure here is a warning and nothing more.
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

/// The list names places by map rather than by world, and the pairs read here are nested several
/// ways -- a map may name one place, several, or a different one per map it leads on from. None of
/// that nesting matters, so it is walked rather than modelled.
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

/// Every `title`/`urlTitle` pair anywhere under `value`.
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
    // space stays a space rather than the underscore a page name would want.
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

/// For the tests, which have only a dump in hand: a run walks the connections it already built for
/// the lines it draws.
#[cfg(test)]
pub fn canonical_routes(worlds: &[World]) -> Routes {
    routes_from(&connections(worlds), origin_world(worlds))
}

#[cfg(test)]
mod tests {
    #[cfg(target_family = "wasm")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::World;

    // The invented tree of `yumezu_routing::fixture`, as worlds this side reads. Built rather than
    // parsed: `data.json` is the wiki's to distribute and is not in the tree, and what this program
    // does with a dump it has is what these tests are about.
    fn load() -> Vec<World> {
        yumezu_routing::fixture::dream_tree()
            .into_iter()
            .enumerate()
            .map(|(id, place)| World {
                title: place.title.to_owned(),
                title_jp: None,
                author: "Yumemiru".to_owned(),
                image: String::new(),
                added: Some(format!("0.1{id:02}")),
                map_url: None,
                map_label: None,
                secret: false,
                cell: Some(id),
                unknown: false,
                connections: place.connections,
            })
            .collect()
    }

    fn world(title: &str, secret: bool, out: &[usize]) -> World {
        World {
            title: title.to_owned(),
            title_jp: None,
            author: String::new(),
            image: String::new(),
            added: None,
            map_url: None,
            map_label: None,
            secret,
            cell: None,
            unknown: false,
            connections: out
                .iter()
                .map(|&target_id| super::Connection {
                    target_id,
                    flags: 0,
                    type_params: Default::default(),
                })
                .collect(),
        }
    }

    /// 0 Nexus - 1 Debug Room (secret) - 2 Sofa Room, each joined to both the others.
    fn secretive() -> Vec<World> {
        vec![
            world("Nexus", false, &[1, 2]),
            world("Debug Room", true, &[0, 2]),
            world("Sofa Room", false, &[0, 1]),
        ]
    }

    // Getting the renumbering wrong is silent: the graph still draws, with lines to the wrong
    // worlds.
    #[test]
    fn hiding_a_world_renumbers_the_connections_that_outlive_it() {
        let mut worlds = secretive();
        super::hide(&mut worlds, false);

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
            // Sofa Room has moved down to 1, and the connection each wrote to the debug room is
            // gone rather than pointing at whoever took its place.
            [("Nexus", vec![1]), ("Sofa Room", vec![0])]
        );
    }

    #[test]
    fn the_code_is_only_taken_once_it_has_been_typed_whole() {
        fn type_out(code: &mut super::Code, keys: &str) -> bool {
            keys.chars().for_each(|key| code.typed(key));
            code.taken()
        }
        let code = &mut super::Code::default();

        assert!(!type_out(code, "2005072"));
        assert!(!type_out(code, "nexus"));
        // A false start, then the code from the top.
        assert!(type_out(code, "220050726"));
        // Half of it, dropped by a search, so what follows completes nothing.
        code.typed('2');
        code.forget();
        assert!(!type_out(code, "0050726"));
    }

    #[test]
    fn the_code_keeps_the_secret_worlds_and_the_ways_into_them() {
        let mut worlds = secretive();
        super::hide(&mut worlds, true);

        assert_eq!(
            worlds
                .iter()
                .map(|world| (world.title.as_str(), world.connections.len()))
                .collect::<Vec<_>>(),
            [("Nexus", 2), ("Debug Room", 2), ("Sofa Room", 2)]
        );
    }

    #[test]
    fn a_frontier_keeps_one_step_past_what_was_visited() {
        // Each links only the step onward, so the step back is read off the far side's listing.
        let dump = chain(&["Nexus", "Sofa Room", "Far Room", "Farther Room"], 0);
        let visited = ["Nexus".to_owned()].into_iter().collect();
        let shown = dump.showing(&visited);

        assert_eq!(
            shown
                .worlds
                .iter()
                .map(|world| (world.title.as_str(), world.cell()))
                .collect::<Vec<_>>(),
            // Kept as a place rather than a world: no cell, so it wears the placeholder.
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
    }

    #[test]
    fn a_frontier_does_not_reach_through_a_passage_it_cannot_be_walked_down() {
        // Every listed step is one-way *into* the world listing it, so Nexus has no way onward.
        let dump = chain(
            &["Nexus", "Sofa Room", "Far Room", "Farther Room"],
            yumezu_routing::ConnType::NO_ENTRY.bits(),
        );
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
                cell: Some(at),
                unknown: false,
                connections: (at + 1 < titles.len())
                    .then(|| super::Connection {
                        target_id: at + 1,
                        flags,
                        type_params: Default::default(),
                    })
                    .into_iter()
                    .collect(),
            })
            .collect();
        super::Dump {
            worlds,
            releases: Vec::new(),
            credits: Vec::new(),
            last_update: None,
            last_full_update: None,
        }
    }

    #[test]
    fn a_connection_reads_the_same_from_either_end() {
        let worlds = load();
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
    fn a_condition_is_read_out_in_the_words_the_dump_carries() {
        // The words asserted below are the English ones.
        crate::i18n::speak_english();
        let worlds = load();
        let connections = super::connections(&worlds);
        let asks: Vec<String> = connections
            .iter()
            .flatten()
            .filter_map(|step| step.out.as_ref())
            .map(super::asks)
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

    #[test]
    fn an_effect_is_named_and_the_words_around_it_are_left_alone() {
        crate::i18n::speak_english();
        let named = |detail: &str| super::named_effects(&super::corrected(detail));
        assert_eq!(super::effects("Bat&comma; Fairy"), "Bat, Fairy");
        assert_eq!(named("fairy"), "Fairy");
        assert_eq!(named("Teru Teru Bozu"), "Teru Teru Bōzu");
        assert_eq!(
            named("Polygon (Mystery Zone A) or Crossing"),
            "Polygon (Mystery Zone A) or Crossing"
        );
        // A name is a whole word, and a word that merely starts with one is not it.
        assert_eq!(named("Springfield"), "Springfield");
    }

    #[test]
    fn the_origin_roots_the_route_tree() {
        let worlds = load();
        let origin = super::origin_world(&worlds);
        assert_eq!(worlds[origin].title, yumezu_routing::START);
        let routes = super::canonical_routes(&worlds);
        assert_eq!(routes.depth[origin], Some(0));
        assert!(routes.parents[origin].is_none());
        // What a seed at the wrong world does not do: everything unreached is at no depth, and
        // the graph draws that as one layer.
        let unreached: Vec<_> = routes
            .depth
            .iter()
            .enumerate()
            .filter(|(_, depth)| depth.is_none())
            .map(|(world, _)| worlds[world].title.as_str())
            .collect();
        assert_eq!(unreached, [] as [&str; 0]);
    }

    #[test]
    fn directions_start_where_they_are_asked_from() {
        let worlds = load();
        let connections = super::connections(&worlds);
        // Well away from the origin, which is the case the canonical routes never exercise.
        let from = (super::origin_world(&worlds) + worlds.len() / 2) % worlds.len();
        let routes = super::routes_from(&connections, from);
        assert_eq!(routes.depth[from], Some(0));
        assert!(routes.parents[from].is_none());
        let reached = routes.depth.iter().filter(|depth| depth.is_some()).count();
        assert!(reached > 1, "{} leads nowhere", worlds[from].title);

        for to in 0..worlds.len() {
            if routes.depth[to].is_none() {
                continue;
            }
            let mut step = to;
            while let Some(parent) = routes.parents[step] {
                let onward = connections[parent]
                    .iter()
                    .find(|onward| onward.world == step)
                    .expect("a route steps between worlds that are connected");
                assert!(
                    onward.out.is_some(),
                    "{} is walked to {} the way it cannot be",
                    worlds[parent].title,
                    worlds[step].title
                );
                step = parent;
            }
            assert_eq!(step, from, "{} is not walked from", worlds[from].title);
        }
    }

    #[test]
    fn a_free_way_nothing_is_shorter_than_is_offered_alone() {
        let worlds = load();
        let connections = super::connections(&worlds);
        let hub = super::hub_world(&worlds);
        let from = super::origin_world(&worlds);
        // A neighbour walked to for nothing: no second way there is as short, and none is freer.
        let to = connections[from]
            .iter()
            .find(|step| {
                step.out
                    .as_ref()
                    .is_some_and(|ask| ask.gate == super::Gate::Free)
            })
            .expect("the origin walks somewhere for nothing")
            .world;

        let ways = super::ways(&connections, from, to, 5, hub);
        assert_eq!(ways.len(), 1, "an alternative to walking straight there");
        assert_eq!(ways[0].walk, [from, to]);
    }

    #[test]
    fn the_eyeball_bomb_is_a_way_back_from_anywhere() {
        let worlds = load();
        let connections = super::connections(&worlds);
        let hub = super::hub_world(&worlds).expect("the dump has the Nexus");
        // One that does not lead there itself, so the way back is the effect and nothing else.
        let from = connections
            .iter()
            .position(|steps| steps.iter().all(|step| step.world != hub))
            .expect("a world the Nexus is not connected to");

        let ways = super::ways(&connections, from, hub, 5, Some(hub));
        let bomb = ways
            .iter()
            .find(|way| way.walk == [from, hub])
            .expect("no way back with the bomb");
        assert_eq!((bomb.asks, bomb.demands), (super::Gate::Effect, 1));
        assert_eq!(
            super::step_asks(&connections, Some(hub), from, hub)
                .as_ref()
                .and_then(super::Ask::detail),
            Some(yumezu_routing::ESCAPE)
        );
    }

    #[test]
    fn every_way_offered_is_one_a_player_could_walk() {
        let worlds = load();
        let connections = super::connections(&worlds);
        let from = super::origin_world(&worlds);
        // Well away from the origin, so there is more than one way to be had.
        let to = (from + worlds.len() / 2) % worlds.len();
        let hub = super::hub_world(&worlds);
        let ways = super::ways(&connections, from, to, 5, hub);
        assert!(!ways.is_empty(), "no way to {}", worlds[to].title);
        assert!(ways.len() <= 5);

        for way in &ways {
            assert_eq!(
                (way.walk.first(), way.walk.last()),
                (Some(&from), Some(&to))
            );
            let mut seen = std::collections::HashSet::new();
            assert!(
                way.walk.iter().all(|&world| seen.insert(world)),
                "a way walks through the same world twice"
            );
            let (mut asks, mut demands) = (super::Gate::Free, 0);
            for pair in way.walk.windows(2) {
                let ask = super::step_asks(&connections, hub, pair[0], pair[1])
                    .expect("a way is walked the way it can be");
                asks = asks.max(ask.gate);
                demands += u32::from(ask.gate != super::Gate::Free);
            }
            assert_eq!((asks, demands), (way.asks, way.demands));
            let onward: Vec<_> = way
                .walk
                .windows(2)
                .map(|pair| {
                    super::step_asks(&connections, hub, pair[0], pair[1])
                        .is_some_and(|ask| ask.gate.onward())
                })
                .collect();
            assert!(
                onward.windows(2).all(|pair| pair[0] >= pair[1]),
                "a way walks on out of an isolated section"
            );
        }

        let traded: Vec<_> = ways
            .iter()
            .map(|way| (way.demands, way.walk.len(), way.backs_out))
            .collect();
        assert!(
            traded.windows(2).all(|pair| pair[0].0 <= pair[1].0),
            "the ways are not offered by what they ask: {traded:?}"
        );
        for &(demands, connections, backs_out) in &traded {
            let room = match backs_out {
                true => 1,
                false => demands.max(1) as usize,
            };
            assert!(
                traded
                    .iter()
                    .filter(|way| (way.0, way.2) == (demands, backs_out))
                    .count()
                    <= room,
                "a class of demand is offered more ways than it asks things: {traded:?}"
            );
            assert!(
                traded
                    .iter()
                    .all(|&(class, len, _)| class >= demands || len > connections),
                "a way is offered that asks more without saving a connection: {traded:?}"
            );
        }
        let walks: std::collections::HashSet<_> = ways.iter().map(|way| &way.walk).collect();
        assert_eq!(walks.len(), ways.len(), "one way is offered twice");
    }

    #[test]
    fn a_player_who_can_pass_everything_is_never_sent_further() {
        let worlds = load();
        let connections = super::connections(&worlds);
        let from = super::origin_world(&worlds);
        let asking = super::routes_from(&connections, from);
        let open = yumezu_routing::routes_from_passing(&connections, from, super::Gate::Revisit);

        let mut nearer = 0;
        for (to, world) in worlds.iter().enumerate() {
            let Some(depth) = asking.depth[to] else {
                continue;
            };
            let open = open.depth[to].expect("a world reached is reached with nothing in the way");
            assert!(
                open <= depth,
                "{} is further away with every gate open",
                world.title
            );
            nearer += u32::from(open < depth);
        }
        assert!(nearer > 0, "no world is nearer with every gate open");
    }

    // The depth and the route the overlay walks are one thing seen twice.
    #[test]
    fn depth_is_the_length_of_the_canonical_route() {
        let worlds = load();
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

    // Compared against a walk ignoring both direction and conditions.
    #[test]
    fn conditions_only_ever_push_a_world_deeper() {
        let worlds = load();
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

        let mut deeper = Vec::new();
        for (world, (canonical, shortest)) in routes.depth.iter().zip(&shortest).enumerate() {
            let (Some(canonical), Some(shortest)) = (canonical, shortest) else {
                continue;
            };
            assert!(
                canonical >= shortest,
                "{} is {canonical} deep but {shortest} hops away",
                worlds[world].title
            );
            if canonical > shortest {
                deeper.push(worlds[world].title.as_str());
            }
        }
        // The worlds the two conditional shortcuts stand in front of.
        assert_eq!(
            deeper,
            [
                "Static Shoreline",
                "Clockwork Dunes",
                "Chalk Observatory",
                "Hollow Carnival",
                "Glass Aviary",
                "Ember Terrace",
                "Drowned Switchboard",
                "Tin Solarium",
            ]
        );
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
        let bean = super::japanese_author("Bean");
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
        // A slice of the list carrying each of the four ways it writes a place: a bare name, a
        // name and the page it is written up on, a map leading to several places, and one leading
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
        // Its own name, the ordinary case, and what an unread list leaves every name at.
        let plain = "https://wikiwiki.jp/yume2kki-t/%E6%B9%96%E4%B8%8A%E3%81%AE%E6%A9%8B";
        assert_eq!(super::page_url(&pages, "湖上の橋"), plain);
        assert_eq!(super::yume2kki_t_url("湖上の橋"), plain);
        // An area written up inside another world's page: the anchor stays an anchor.
        assert_eq!(
            super::page_url(&pages, "製作者の部屋"),
            "https://wikiwiki.jp/yume2kki-t/%E3%81%86%E3%82%8D%E3%81%A4%E3%81%8D%E9%82%B8#map0230"
        );
        // A place written up on a bigger page, and one filed under a path -- the latter is the only
        // kind the list keeps outside `mapLocations`. The slash stays a slash.
        assert_eq!(
            super::page_url(&pages, "昭和路地：バスツアー"),
            "https://wikiwiki.jp/yume2kki-t/%E6%98%AD%E5%92%8C%E8%B7%AF%E5%9C%B0"
        );
        assert_eq!(
            super::page_url(&pages, "ミニゲームA"),
            "https://wikiwiki.jp/yume2kki-t/%E3%83%9F%E3%83%8B%E3%82%B2%E3%83%BC%E3%83%A0/A"
        );
    }

    // Nothing off to the side, however near.
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

    // A revisit is no way in: a route walked in through one is the last thing `Gate::Revisit`
    // exists for, so a world with any other way in should never be reached by one.
    #[test]
    fn no_route_is_walked_in_through_a_revisit_a_player_could_stand_off() {
        let worlds = load();
        let routes = super::canonical_routes(&worlds);
        let steps = super::walkable_steps(&worlds);
        let gate = |from: usize, to: usize| {
            steps[from]
                .iter()
                .find(|(next, _)| *next == to)
                .map(|(_, ask)| ask.gate)
        };
        // A route ending in an isolated section leaves a player where the world's other
        // connections are behind a wall, so nothing there is a way in to anywhere.
        let stranded = |world: usize| {
            let mut at = world;
            while let Some(parent) = routes.parents[at] {
                if gate(parent, at).is_some_and(|gate| !gate.onward()) {
                    return true;
                }
                at = parent;
            }
            false
        };

        let walked_in: Vec<_> = routes
            .parents
            .iter()
            .enumerate()
            .filter(|(world, parent)| {
                parent.is_some_and(|parent| gate(parent, *world) == Some(super::Gate::Revisit))
            })
            .map(|(world, _)| world)
            .collect();
        // Without this the loop below has nothing to iterate and the test passes proving nothing.
        assert_eq!(
            walked_in
                .iter()
                .map(|&world| worlds[world].title.as_str())
                .collect::<Vec<_>>(),
            ["Ember Terrace"]
        );

        for world in walked_in {
            let standing: Vec<_> = (0..worlds.len())
                .filter(|&from| gate(from, world).is_some_and(|gate| gate != super::Gate::Revisit))
                .filter(|&from| routes.depth[from].is_some() && !stranded(from))
                .map(|from| worlds[from].title.as_str())
                .collect();
            assert_eq!(
                standing,
                [] as [&str; 0],
                "{} is walked in through a revisit",
                worlds[world].title
            );
        }
    }

    // The panel reads a step's demand off `connections`; one missing there drops it silently.
    #[test]
    fn every_canonical_step_is_walkable_where_the_panel_reads_it() {
        let worlds = load();
        let routes = super::canonical_routes(&worlds);
        let connections = super::connections(&worlds);
        for (world, parent) in routes.parents.iter().enumerate() {
            let Some(parent) = *parent else { continue };
            let step = connections[parent]
                .iter()
                .find(|step| step.world == world)
                .unwrap_or_else(|| panic!("{} is joined to no parent", worlds[world].title));
            assert!(
                step.out.is_some(),
                "{} is walked in from {} and no way there",
                worlds[world].title,
                worlds[parent].title
            );
        }
    }
}
