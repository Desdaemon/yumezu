//! The Yume 2kki world graph, as published by yume.wiki and served to this app as `data.json` by
//! `dreamweaver`. See [`load`].
//!
//! The dump carries far more per world than a layout needs, so only the fields the visualization
//! draws are deserialized.
//!
//! The dump itself is here. What the app makes of it is beside it: [`mod@conditions`] reads a
//! connection's conditions out, [`links`] addresses the wiki pages, and [`net`] fetches from the
//! server.

mod conditions;
mod links;
mod net;
#[cfg(test)]
mod tests;

pub use conditions::*;
pub use links::*;
pub use net::*;

use serde::Deserialize;
// The connection and the walk are the routing crate's, so the program that publishes the dump
// reads them exactly as this one does.
use yumezu_routing::walkable_steps;
pub use yumezu_routing::{
    Conditions, Connection, Demand, Gate, Routes, Step, Way, connections, hub_world, origin_world,
    routes_from, step_conditions, ways,
};

use super::fetch;
use super::i18n::{self, t};

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
    /// fresh and still be missing an edit an incremental read did not cover.
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
    /// the atlas by `tools/atlas`, and fetched again once the view outruns the atlas. See
    /// `detail`.
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
    /// Its cell in the thumbnail atlas, as the server hands it out: once, and never moved, so an
    /// atlas packed before this world existed is still right about the worlds it holds. Read
    /// through [`World::cell`].
    ///
    /// `None` from a server old enough not to publish one, which costs that run its thumbnails
    /// rather than giving every world somebody else's.
    #[serde(default)]
    cell: Option<usize>,
    /// Whether the player has never stood here, in a run showing only where they have been. Drawn
    /// because it touches one they have, so it wears the placeholder and has no page to open. See
    /// [`Dump::showing`].
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
    let held = match i18n::speaking_japanese() {
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
            Some(jp) if i18n::speaking_japanese() => jp,
            _ => &self.en,
        }
    }

    /// Whether this world may be named, searched for, and looked up.
    pub fn known(&self) -> bool {
        !self.unknown
    }

    /// Where `needle` falls in this name, and how much name is left over, for whichever name it
    /// fits best. Both are searched whichever is shown.
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
    /// looked up on the wiki that wrote it.
    pub fn wiki_url(&self) -> String {
        match &self.jp {
            Some(jp) if i18n::speaking_japanese() => yume2kki_t_url(jp),
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

    /// In the order the wiki lists them, and empty for the few hundred worlds it has drawn none of.
    /// The two lists are zipped rather than trusted to be in step: an uncaptioned map would panic.
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

impl Version {
    /// Where the release is written up, which is a section of a page on either wiki rather than a
    /// page of its own. See [`version_url`] and [`yume2kki_t_version_url`].
    pub fn wiki_url(&self) -> String {
        if i18n::speaking_japanese() {
            yume2kki_t_version_url(&self.name)
        } else {
            version_url(&self.name)
        }
    }
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
        if i18n::speaking_japanese() {
            let name = self.name.show();
            yume2kki_t_author_url(japanese_author(name))
        } else {
            author_url(&self.name.en)
        }
    }
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
/// A connection names the world it leads to by index, so taking one out moves every index above it.
/// Done where the dump becomes the app's, so nothing downstream sees those worlds.
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
    /// The dump is the yardstick rather than YNOproject's own catalog, which records rooms the wiki
    /// keeps no world for, so the count comes out under what YNOproject would say.
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
    /// The worlds a step beyond are the steps the player could take, so a connection that cannot be
    /// walked that way leads nowhere. Kept as places rather than worlds, so the graph says there is
    /// something there without saying what. See [`World::unknown`].
    ///
    /// A whole dump rather than a mask, taking a world out renumbering every connection above it --
    /// the reason [`hide`] rebuilds rather than skips. One copy per press of the button.
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

/// For the tests, which have only a dump in hand: a run walks the connections it already built for
/// the lines it draws.
#[cfg(test)]
pub fn canonical_routes(worlds: &[World]) -> Routes {
    routes_from(&connections(worlds), origin_world(worlds))
}
