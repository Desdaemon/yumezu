//! What a world is, as the dump publishes it.
//!
//! `data.json` is an interface with a reader on the other side, so the field names and the nesting
//! are the reference implementation's and the serde renames are what keep them that way.

use std::collections::BTreeMap;

use crate::smw;

use serde::{Deserialize, Serialize};

bitflags::bitflags! {
    /// Independent flags rather than a kind: a connection can be locked behind a condition *and*
    /// seasonal *and* one-way. The numbering is the wiki explorer's own and cannot be renumbered
    /// -- it is what the dump publishes and what the app reads back.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct ConnType: i16 {
        /// Walkable from the world that lists it, never back.
        const ONE_WAY = 1 << 0;
        /// Walkable only back to the world that lists it, never from it.
        const NO_ENTRY = 1 << 1;
        /// This side opens a connection the far side reports as [`ConnType::LOCKED`].
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
    /// `None` for a word this does not know: an unrecognised attribute leaves the connection
    /// exactly as walkable as it was, so the wiki growing its vocabulary is not fatal.
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
                // Comma separated and joined no further: the wiki does not say whether one effect
                // is enough or all are needed, so an "and" would settle it here.
                Wording::Words(connection.effects_needed.join(",")),
            )),
            "Chance" => Some((
                Self::CHANCE,
                Wording::Words(connection.chance_percentage.clone()?),
            )),
            "Seasonal" => {
                // The reader has one word to put a route and four it knows how to translate; the
                // reference wrapper narrowed these the same way.
                let season = connection.seasons_available.first()?;
                Some((Self::SEASONAL, Wording::Translated(season.to_owned())))
            }
            _ => None,
        }
    }
}

/// The wiki writes these as instructions -- "Requires to have seen the first four endings." -- and
/// the dump publishes the bare condition a reader shows beside a route.
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

/// What the wiki says about one of a connection's conditions, if it says anything.
pub enum Wording {
    /// The flag is the whole of what there is to say.
    None,
    /// The wiki's own English words.
    Words(String),
    /// English words this program can also give in Japanese, which is only ever the four seasons.
    Translated(String),
}

impl Wording {
    /// The words, and their Japanese where there is a Japanese to give.
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

/// Exactly as the reference implementation's `/data` answers it.
///
/// The empty lists are not oversights: effects, menu themes, wallpapers and soundtrack entries are
/// written as prose and tables rather than held in the wiki's store. They stay as empty lists so a
/// reader written against the reference dump keeps working.
#[derive(Serialize, Deserialize, Default)]
pub struct Dump {
    #[serde(rename = "worldData")]
    pub worlds: Vec<World>,
    #[serde(rename = "authorInfoData")]
    pub authors: Vec<Author>,
    /// Every release the wiki dates, newest first, patches included -- see [`crate::smw`].
    #[serde(rename = "versionInfoData")]
    pub versions: Vec<Version>,
    /// Prose on the wiki's Effects page, so published empty. See [`Dump`].
    #[serde(rename = "effectData")]
    pub effects: Vec<serde_json::Value>,
    /// A table on the wiki's Menu Themes page, so published empty.
    #[serde(rename = "menuThemeData")]
    pub menu_themes: Vec<serde_json::Value>,
    /// The store holds these as collectibles but without the pictures, so published empty.
    #[serde(rename = "wallpaperData")]
    pub wallpapers: Vec<serde_json::Value>,
    /// Templates on the wiki's Soundtrack pages, so published empty.
    #[serde(rename = "bgmTrackData")]
    pub bgm_tracks: Vec<serde_json::Value>,
    /// When this dump was built, ISO 8601.
    #[serde(rename = "lastUpdate")]
    pub last_update: Option<String>,
    /// When the whole wiki was last read without first asking what had changed. A soft sync
    /// carries this over rather than moving it.
    #[serde(rename = "lastFullUpdate")]
    pub last_full_update: Option<String>,
    /// Always false: the reference implementation lets an operator edit the wiki through the
    /// explorer, and nothing here writes to the wiki.
    #[serde(rename = "isAdmin")]
    pub is_admin: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Version {
    pub name: String,
    pub authors: Option<String>,
    #[serde(rename = "releaseDate")]
    pub release_date: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Author {
    pub name: String,
    #[serde(rename = "nameJP")]
    pub name_jp: Option<String>,
}

/// [`World::id`] is the world's place in the published list rather than a database key: the reader
/// indexes straight into the array with a [`Connection::target_id`].
#[derive(Serialize, Deserialize)]
pub struct World {
    pub id: usize,
    /// Like `id`, but is assigned from a monotonic counter. See `cells` in [`crate::sync`].
    #[serde(default)]
    pub cell: Option<usize>,
    pub title: String,
    #[serde(rename = "titleJP")]
    pub title_jp: Option<String>,
    /// Empty rather than absent for a world the wiki credits to nobody: the reader groups by it.
    pub author: String,
    /// Steps from the starting room along connections a player can walk unconditionally. See
    /// `depth`.
    pub depth: i32,
    /// Steps along any connection at all, however conditional.
    #[serde(rename = "minDepth")]
    pub min_depth: i32,
    /// Empty, never absent: the reader takes this as a string and would refuse a null.
    pub filename: String,
    /// The world's maps, both as `|`-separated lists read in step with each other.
    #[serde(rename = "mapUrl")]
    pub map_url: Option<String>,
    #[serde(rename = "mapLabel")]
    pub map_label: Option<String>,
    /// Two more `|`-separated lists read in step: the files, and `<title>^<where it plays>`.
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
    pub removed: bool,
    /// Set for the debug room and whatever else an operator has marked as a spoiler. Published
    /// rather than acted on: the world stays in the dump, in the graph and in the numbering, and
    /// the client is what leaves it out.
    pub secret: bool,
    pub connections: Vec<Connection>,
}

/// The kind is the wiki's own shorthand -- `+` for a major change, `c-` for a removed connection
/// -- and empty for a release that says only that something changed.
#[derive(Serialize, Deserialize)]
pub struct VerUpdated {
    #[serde(rename = "verUpdated")]
    pub ver_updated: String,
    #[serde(rename = "updateType")]
    pub update_type: String,
}

#[derive(Serialize, Deserialize)]
pub struct VerGap {
    #[serde(rename = "verRemoved")]
    pub ver_removed: String,
    #[serde(rename = "verReadded")]
    pub ver_readded: String,
}

#[derive(Serialize, Deserialize)]
pub struct Connection {
    #[serde(rename = "targetId")]
    pub target_id: usize,
    #[serde(rename = "type")]
    pub flags: i16,
    /// Keyed by the flag imposing the condition. Ordered, so two dumps of the same database
    /// compare equal as text.
    #[serde(rename = "typeParams")]
    pub type_params: BTreeMap<i16, TypeParams>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TypeParams {
    pub params: String,
    #[serde(rename = "paramsJP")]
    pub params_jp: Option<String>,
}

#[cfg(test)]
mod tests {
    // The last dump is where the secret marks and the atlas cells are read from, and a sync that
    // cannot parse it silently starts from nothing: marks forgotten, cells handed out afresh. So
    // every field this has ever published has to stay readable, whether it is still published or
    // not.
    #[test]
    fn a_dump_written_before_the_fields_moved_is_still_read() {
        // As published before `cell` existed and while `mapIds` still did.
        let json = r#"{"worldData":[{"id":0,"title":"Debug Room","titleJP":null,"author":"20",
            "depth":1,"minDepth":1,"filename":"","mapUrl":null,"mapLabel":null,"bgmUrl":null,
            "bgmLabel":null,"verAdded":"0.036","verRemoved":null,"verUpdated":null,
            "verGaps":null,"mapIds":[1],"removed":false,"secret":true,"connections":[]}],
            "authorInfoData":[],"versionInfoData":[],"effectData":[],"menuThemeData":[],
            "wallpaperData":[],"bgmTrackData":[],"lastUpdate":null,"lastFullUpdate":null,
            "isAdmin":false}"#;
        let dump: super::Dump = serde_json::from_str(json).expect("a dump this once wrote");
        let world = dump.worlds.first().expect("the one world");
        // The mark survives, which is what a sync carries forward.
        assert!(world.secret);
        // No cell to carry: this world takes a fresh one.
        assert_eq!(world.cell, None);
    }
}
