//! What a world is, as the dump publishes it.
//!
//! The shapes here are not this program's: `data.json` is an interface with a reader on the other
//! side, so the field names and the nesting are the reference implementation's and the serde
//! renames are what keep them that way.

use std::collections::BTreeMap;

use crate::smw;

use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    /// What a passage between two worlds is like.
    ///
    /// Independent flags rather than a kind, because a passage really can be several of these at
    /// once -- locked behind a condition *and* seasonal *and* one-way -- and the reader decides
    /// for itself which of them it cares about. The numbering is the wiki explorer's own and is
    /// load-bearing: it is what the dump publishes and what the app reads back.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct ConnType: i16 {
        /// Walkable from the world that lists it, never back.
        const ONE_WAY = 1 << 0;
        /// Walkable only back to the world that lists it, never from it.
        const NO_ENTRY = 1 << 1;
        /// This side opens a passage the far side reports as [`ConnType::LOCKED`].
        const UNLOCK = 1 << 2;
        const LOCKED = 1 << 3;
        /// Leads to a part of the destination with no way onward.
        const DEAD_END = 1 << 4;
        /// The far side of a dead end: reachable only from that isolated part.
        const ISOLATED = 1 << 5;
        /// Open only to a player wearing particular effects.
        const EFFECT = 1 << 6;
        /// Open at random.
        const CHANCE = 1 << 7;
        /// Open only once something else has happened, which the wiki writes out as a sentence.
        const LOCKED_CONDITION = 1 << 8;
        const SHORTCUT = 1 << 9;
        /// Where a shortcut comes out, walked backwards into the shortcut.
        const EXIT_POINT = 1 << 10;
        const SEASONAL = 1 << 11;
        /// Documented but no longer walkable.
        const INACCESSIBLE = 1 << 12;
        const TRACKED = 1 << 13;
    }
}

impl ConnType {
    /// Reads one of the wiki's own attribute words, and the wording that comes with it.
    ///
    /// `None` for a word this does not know, which is how a vocabulary the wiki grows stays
    /// non-fatal: an unrecognised attribute leaves the passage exactly as walkable as it was.
    pub fn of(attribute: &str, connection: &smw::Connection) -> Option<(Self, Wording)> {
        let plain = |flag| Some((flag, Wording::None));
        match attribute {
            "No Return" => plain(Self::ONE_WAY),
            "No Entry" => plain(Self::NO_ENTRY),
            "Unlockable" => plain(Self::UNLOCK),
            "Locked" => plain(Self::LOCKED),
            "Shortcut" => plain(Self::SHORTCUT),
            "Exit Point" => plain(Self::EXIT_POINT),
            "Dead End" => plain(Self::DEAD_END),
            "Return" => plain(Self::ISOLATED),
            "Conditional" => Some((
                Self::LOCKED_CONDITION,
                Wording::Words(condition(connection.unlock_condition.as_deref()?)),
            )),
            "Needs Effect" => Some((
                Self::EFFECT,
                // Comma separated as the wiki lists them, and joined no further: it does not say
                // whether one effect is enough or all of them are needed, so an "and" here would
                // be this program deciding.
                Wording::Words(connection.effects_needed.join(",")),
            )),
            "Chance" => Some((
                Self::CHANCE,
                Wording::Words(connection.chance_percentage.clone()?),
            )),
            "Seasonal" => {
                // The first of however many the wiki lists. A passage open in three seasons is
                // published as open in the first of them: the reader has one word to put beside a
                // route and four it knows how to translate, and the wrapper narrowed these the
                // same way.
                let season = connection.seasons_available.first()?;
                Some((Self::SEASONAL, Wording::Translated(season.to_owned())))
            }
            _ => None,
        }
    }
}

/// Tidies the wiki's sentence for a conditional passage into the one the dump publishes.
///
/// The wiki writes these as instructions to a reader -- "Requires to have seen the first four
/// endings." -- and the dump publishes them as a bare condition, which is what a reader shows
/// beside a route. The leading verb and the full stop go; the first letter comes back up.
fn condition(sentence: &str) -> String {
    let trimmed = sentence
        .strip_prefix("Requires ")
        .or_else(|| sentence.strip_prefix("Required "))
        .or_else(|| sentence.strip_prefix("Require "))
        .unwrap_or(sentence);
    let trimmed = trimmed.strip_prefix("to ").unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('.').unwrap_or(trimmed);
    let mut chars = trimmed.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// What the wiki says about one of a passage's conditions, if it says anything.
pub enum Wording {
    /// The flag is the whole of what there is to say.
    None,
    /// The wiki's own English words.
    Words(String),
    /// English words this program can also give in Japanese, which is only ever the four seasons.
    Translated(String),
}

impl Wording {
    /// The pair the dump publishes for a condition: the words, and their Japanese where there is
    /// a Japanese to give.
    pub fn published(&self) -> Option<(String, Option<String>)> {
        match self {
            Wording::None => None,
            Wording::Words(words) => Some((words.clone(), None)),
            Wording::Translated(season) => {
                let jp = match season.as_str() {
                    "Spring" => Some("春"),
                    "Summer" => Some("夏"),
                    "Fall" => Some("秋"),
                    "Winter" => Some("冬"),
                    _ => None,
                };
                Some((season.clone(), jp.map(str::to_owned)))
            }
        }
    }
}

/// The whole published dump, exactly as the reference implementation's `/data` answers it.
///
/// The empty lists are not oversights. Effects, menu themes, wallpapers and soundtrack entries are
/// written as prose and tables on their wiki pages rather than held in the wiki's store, and none
/// of them says anything about how the worlds join up, which is what this dump is read for. They
/// stay in the shape so a reader written against the reference dump keeps working.
#[derive(Serialize, Deserialize, Default)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    #[serde(rename = "authorInfoData")]
    pub authors: Vec<Author>,
    /// Every release the wiki dates, newest first, patches included -- see [`crate::smw`].
    #[serde(rename = "versionInfoData")]
    pub versions: Vec<Version>,
    /// Prose on the wiki's Effects page; nothing here reads it. See [`Dump`].
    #[serde(rename = "effectData")]
    pub effects: Vec<serde_json::Value>,
    /// A table on the wiki's Menu Themes page; likewise.
    #[serde(rename = "menuThemeData")]
    pub menu_themes: Vec<serde_json::Value>,
    /// The store does hold these, as collectibles -- but without the pictures, which are on the
    /// page. Likewise.
    #[serde(rename = "wallpaperData")]
    pub wallpapers: Vec<serde_json::Value>,
    /// Templates on the wiki's Soundtrack pages; likewise.
    #[serde(rename = "bgmTrackData")]
    pub bgm_tracks: Vec<serde_json::Value>,
    /// When this dump was built, ISO 8601.
    #[serde(rename = "lastUpdate")]
    pub last_update: Option<String>,
    /// When the whole wiki was last read without first asking what had changed. A soft sync
    /// carries this over rather than moving it -- see `sync::stamps`.
    #[serde(rename = "lastFullUpdate")]
    pub last_full_update: Option<String>,
    /// Always false. The reference implementation lets an operator hold an admin key and edit the
    /// wiki through the explorer; nothing here writes to the wiki.
    #[serde(rename = "isAdmin")]
    pub is_admin: bool,
}

/// A release, as the dump lists it.
#[derive(Serialize, Deserialize)]
pub struct Version {
    pub name: String,
    pub authors: Option<String>,
    #[serde(rename = "releaseDate")]
    pub release_date: Option<String>,
}

/// Someone the wiki credits.
#[derive(Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(rename = "nameJP")]
    pub name_jp: Option<String>,
}

/// A world, as the dump publishes it.
///
/// [`World::id`] is the world's place in the published list rather than its database key: the
/// reader indexes straight into the array with a [`Connection::target_id`], so the two have to be
/// the same number. See `dump`.
#[derive(Serialize, Deserialize)]
pub struct World {
    pub id: usize,
    pub title: String,
    #[serde(rename = "titleJP")]
    pub title_jp: Option<String>,
    /// Empty rather than absent for a world the wiki credits to nobody, which is what the reader
    /// groups by.
    pub author: String,
    /// Steps from the starting room along passages a player can simply walk. See `depth`.
    pub depth: i32,
    /// Steps along any passage at all, however conditional.
    #[serde(rename = "minDepth")]
    pub min_depth: i32,
    /// Where the wiki serves the world's headline picture from. Empty, never absent, for a world
    /// the wiki shows no picture of: the reader takes this field as a string and would refuse a
    /// dump that gave it null.
    pub filename: String,
    /// The world's maps, both as `|`-separated lists read in step with each other.
    #[serde(rename = "mapUrl")]
    pub map_url: Option<String>,
    #[serde(rename = "mapLabel")]
    pub map_label: Option<String>,
    /// The world's music, likewise: the files, and `<title>^<where it plays>` for each.
    #[serde(rename = "bgmUrl")]
    pub bgm_url: Option<String>,
    #[serde(rename = "bgmLabel")]
    pub bgm_label: Option<String>,
    #[serde(rename = "verAdded")]
    pub ver_added: Option<String>,
    #[serde(rename = "verRemoved")]
    pub ver_removed: Option<String>,
    #[serde(rename = "verUpdated")]
    pub ver_updated: Option<Vec<VerUpdated>>,
    #[serde(rename = "verGaps")]
    pub ver_gaps: Option<Vec<VerGap>>,
    /// The RPG Maker maps the world is built out of, by the number the game gives each.
    ///
    /// Not a field the reference dump carries -- it keeps these in its database and publishes only
    /// `size`, the area they add up to. The store holds which maps a world is but not how big any
    /// of them is, so `size` cannot be had from here and this is what there is instead: the count
    /// and the sharing, without the areas. Absent rather than empty for the three pages in the
    /// category whose infobox names no map at all.
    #[serde(rename = "mapIds", default, skip_serializing_if = "Vec::is_empty")]
    pub map_ids: Vec<u32>,
    pub removed: bool,
    /// Whether a reader should be shown this world at all. Set for the debug room, and for
    /// whatever else an operator has marked as a spoiler -- see `sync`'s `SECRET_MAPS` and
    /// `marked_secret`. Published rather than acted on here: the world stays in the dump, in the
    /// graph and in the numbering, and the client is what leaves it out.
    pub secret: bool,
    pub connections: Vec<Connection>,
}

/// A release that changed a world, and what kind of change it was.
///
/// The kind is the wiki's own shorthand -- `+` for a major change, `c-` for a removed connection,
/// and so on -- and empty for a release that says only that something changed.
#[derive(Serialize, Deserialize)]
pub struct VerUpdated {
    #[serde(rename = "verUpdated")]
    pub ver_updated: String,
    #[serde(rename = "updateType")]
    pub update_type: String,
}

/// A span a world was absent for.
#[derive(Serialize, Deserialize)]
pub struct VerGap {
    #[serde(rename = "verRemoved")]
    pub ver_removed: String,
    #[serde(rename = "verReadded")]
    pub ver_readded: String,
}

/// One passage, from the side of the world that lists it.
#[derive(Serialize, Deserialize)]
pub struct Connection {
    #[serde(rename = "targetId")]
    pub target_id: usize,
    #[serde(rename = "type")]
    pub flags: i16,
    /// The wiki's words for whichever conditions it writes words for, keyed by the flag that
    /// imposes them. Ordered, so two dumps of the same database compare equal as text.
    #[serde(rename = "typeParams")]
    pub type_params: BTreeMap<i16, TypeParams>,
}

/// The wiki's words for one condition.
#[derive(Serialize, Deserialize, Clone)]
pub struct TypeParams {
    pub params: String,
    #[serde(rename = "paramsJP")]
    pub params_jp: Option<String>,
}
