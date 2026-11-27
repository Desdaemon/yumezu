//! The Yume 2kki world graph from `dreamweaver`'s `data.json`; only the drawn fields are
//! deserialized. See [`load`].

mod conditions;
mod links;
mod net;
#[cfg(test)]
mod tests;

pub use conditions::*;
pub use links::*;
pub use net::*;

use serde::Deserialize;
use yumezu_routing::directed_steps;
pub use yumezu_routing::{
    Conditions, Connection, Demand, Gate, Routes, Step, Way, connections, hub_world, origin,
    routes_from, step_between, step_conditions, ways,
};

use super::fetch;
use super::i18n;
use super::prelude::*;

#[derive(Clone, Deserialize)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    /// Newest first. Most added no world, so the catalog lists [`Dump::versions`] instead.
    #[serde(rename = "versionInfoData")]
    releases: Vec<Release>,
    /// Read only for the Japanese names: who made what is settled by the worlds themselves, in
    /// [`World::author`].
    #[serde(rename = "authorInfoData", default)]
    credits: Vec<Credit>,
    /// When the dump was built, ISO 8601.
    #[serde(rename = "lastUpdate", default)]
    pub last_update: Option<String>,
    /// When the wiki was last read in full: an incremental read can miss an edit.
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
    /// The English wiki's name for the world, which is also the page name. See [`wiki_url`].
    pub title: String,
    /// The game's own name for the world, published for all but a few dozen worlds.
    #[serde(rename = "titleJP")]
    title_jp: Option<String>,
    pub author: String,
    /// The wiki's URL for this world's full-size picture, packed into the atlas by `dreamweaver`
    /// and fetched by `lod` as the high LoD once the view outgrows the atlas.
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
    /// Whether the dump hides this world from the user. Read only by [`hide`], which drops such
    /// worlds on load.
    #[serde(default)]
    secret: bool,
    pub connections: Vec<Connection>,
    /// This world's cell in the thumbnail atlas, assigned once and never moved, so an older atlas
    /// stays right. Read through [`World::cell`]; `None` from a server too old to publish cells.
    #[serde(default)]
    cell: Option<usize>,
    /// Unvisited, in a view of visited worlds: drawn as a neighbour of one, with the placeholder
    /// and no page to open. See [`Dump::showing`].
    #[serde(skip)]
    concealed: bool,
}

/// The entries `needle` matches, best first, or every entry where nothing is typed: a prefix beats
/// a substring and an exact name beats both.
pub fn ranked_matches<'a>(
    entries: impl Iterator<Item = impl Iterator<Item = &'a str>>,
    needle: &str,
) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return (0..entries.count()).collect();
    }
    let mut hits: Vec<_> = entries
        .enumerate()
        .filter_map(|(entry, names)| {
            let (at, length) = names
                .filter_map(|name| Some((name.to_lowercase().find(&needle)?, name.len())))
                .min()?;
            Some((at, length, entry))
        })
        .collect();
    hits.sort_unstable();
    hits.into_iter().map(|(_, _, entry)| entry).collect()
}

/// The name shown for an unvisited world, one slot per language. Stored, since [`Title::show`]
/// returns a borrow that a message built on the spot would not outlive.
fn unvisited() -> &'static str {
    static ENGLISH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    static JAPANESE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let held = match i18n::speaking_japanese() {
        true => &JAPANESE,
        false => &ENGLISH,
    };
    held.get_or_init(|| t!("unvisited-location"))
}

/// Both names are kept: the wiki's pages are named in English whatever the language, and the
/// user may know a world by either.
pub struct Title {
    /// Always there, and always what [`wiki_url`] is given.
    pub en: String,
    jp: Option<String>,
    /// Whether the names are held back, for an unvisited world. Both are still
    /// carried, and neither is shown, searched, or opened.
    concealed: bool,
}

impl Title {
    /// Read rather than stored, so choosing a language renames every world on screen without
    /// anything being rebuilt.
    pub fn show(&self) -> &str {
        if self.concealed {
            return unvisited();
        }
        match &self.jp {
            Some(jp) if i18n::speaking_japanese() => jp,
            _ => &self.en,
        }
    }

    /// Whether this world may be named, searched for, and looked up.
    pub fn is_searchable(&self) -> bool {
        !self.concealed
    }

    /// Each wiki names a page by that wiki's own name for the world, so each name opens on the
    /// wiki that wrote the name.
    pub fn wiki_url(&self) -> String {
        match &self.jp {
            Some(jp) if i18n::speaking_japanese() => yume2kki_t_url(jp),
            _ => wiki_url(&self.en),
        }
    }

    /// English first. What a search reads, whichever is shown.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        let named = !self.concealed;
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
    /// with several maps, the caption alone says which part each covers.
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
            concealed: self.concealed,
        }
    }

    /// `None` for an unvisited world, drawn as the atlas's placeholder, and for a world without a
    /// cell.
    pub fn cell(&self) -> Option<usize> {
        self.cell.filter(|_| !self.concealed)
    }

    /// In the wiki's order; empty for the few hundred worlds the wiki has no map of.
    /// The two lists are zipped, so an uncaptioned map cannot panic.
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
    /// ISO 8601, of which only the day is ever shown. `None` for an undated release.
    #[serde(rename = "releaseDate")]
    date: Option<String>,
}

/// A release that added at least one world: what the catalog lists.
pub struct Version {
    /// The version history's name, or the worlds' name for a release missing from the history.
    pub name: String,
    /// `YYYY-MM-DD`, empty for an undated release.
    pub released: String,
    /// In world order.
    pub worlds: Vec<usize>,
}

impl Version {
    /// The section of a wiki page where the release is written up. See [`version_url`] and
    /// [`yume2kki_t_version_url`].
    pub fn wiki_url(&self) -> String {
        if i18n::speaking_japanese() {
            yume2kki_t_version_url(&self.name)
        } else {
            version_url(&self.name)
        }
    }
}

/// The two wikis list an author's work in different shapes; [`Author::wiki_url`] picks the page.
pub struct Author {
    /// The English name addresses the author's page; either name finds the author.
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
            // A false start is usually the code's first key struck twice.
            false => usize::from(MYSTERY_TEXT.starts_with(key)),
        };
        if self.at == MYSTERY_TEXT.len() {
            self.at = 0;
            self.done = true;
        }
    }

    /// Drops a half-typed code once the keys go to a text box on the panel.
    pub fn forget(&mut self) {
        self.at = 0;
    }

    pub fn taken(&mut self) -> bool {
        std::mem::take(&mut self.done)
    }
}

/// Drops the worlds the dump marks secret, and renumbers what is left.
/// Runs on load, so nothing downstream sees those worlds.
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
/// world that has gone. The one place a world leaves the graph.
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
            // `flatten` covers a dropped target and one missing from the dump.
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
    /// Counted against the dump, so the count runs below YNOproject's, which includes rooms the
    /// wiki has no world for.
    pub fn visited(&self, visited: &std::collections::HashSet<String>) -> usize {
        self.worlds
            .iter()
            .filter(|world| visited.contains(&world.title))
            .count()
    }

    /// The dump cut back to the visited worlds and, unnamed, those one step onward: see
    /// [`World::concealed`]. `visited` holds wiki titles, the only names shared with YNOproject.
    pub fn showing(&self, visited: &std::collections::HashSet<String>) -> Dump {
        let been: Vec<bool> = self
            .worlds
            .iter()
            .map(|world| visited.contains(&world.title))
            .collect();
        let mut shown = been.clone();
        for (from, onward) in directed_steps(&self.worlds).into_iter().enumerate() {
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
            world.concealed = true;
            // The captions and the high-LoD picture would name the world.
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

    /// The releases that added worlds, each with the worlds added, newest first by the version
    /// history, as names do not sort. Releases missing from the history go last, undated.
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
                    // The dump times every release to midnight.
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
        // Stable: releases missing from the history keep the worlds' order.
        versions.sort_by_key(|version| {
            rank.get(&same_release(&version.name))
                .map_or(usize::MAX, |(at, _)| *at)
        });
        versions
    }

    /// Every credited author with their worlds, and each world's author index.
    /// Busiest first, as an empty search shows only the first few; ties break by name.
    pub fn authors(&self) -> (Vec<Author>, Vec<usize>) {
        // The dump repeats the English name for most people.
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
                        concealed: false,
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

        // After the sort, which renumbers authors.
        let mut author_of = vec![0; self.worlds.len()];
        for (author, by) in authors.iter().enumerate() {
            for &world in &by.worlds {
                author_of[world] = author;
            }
        }
        (authors, author_of)
    }
}

/// For the tests, which have only a dump in hand: the app traverses the connections already built
/// for drawing the lines.
#[cfg(test)]
pub fn canonical_routes(worlds: &[World]) -> Routes {
    routes_from(&connections(worlds), origin(worlds))
}
