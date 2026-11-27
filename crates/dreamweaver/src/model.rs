//! The dump's schema; the serde renames keep the reference implementation's field names.

use crate::smw;

use serde::{Deserialize, Serialize};

/// The wiki's own connection bitfield and the shape a connection is published in, both from
/// [`yumezu_routing`], so yumezu reads exactly what dreamweaver writes.
pub use yumezu_routing::{ConnType, Connection, TypeParams};

/// What one of the wiki's connection attributes means, as a flag and any words.
/// `None` for an unknown attribute, which leaves the connection's gates unchanged.
pub fn conn_type(attribute: &str, connection: &smw::Connection) -> Option<(ConnType, Wording)> {
    {
        let plain = |flag| Some((flag, Wording::None));
        match attribute {
            "No Return" => plain(ConnType::ONE_WAY),
            "No Entry" => plain(ConnType::NO_ENTRY),
            "Unlockable" => plain(ConnType::UNLOCK),
            "Locked" => plain(ConnType::LOCKED),
            "Shortcut" => plain(ConnType::SHORTCUT),
            "Exit Point" => plain(ConnType::EXIT_POINT),
            "Dead End" => plain(ConnType::DEAD_END),
            "Return" => plain(ConnType::ISOLATED),
            "Conditional" => Some((
                ConnType::LOCKED_CONDITION,
                Wording::Words(condition(connection.unlock_condition.as_deref()?)),
            )),
            "Needs Effect" => Some((
                ConnType::EFFECT,
                // The wiki does not say whether one effect is enough or all are needed.
                Wording::Words(connection.effects_needed.join(",")),
            )),
            "Chance" => Some((
                ConnType::CHANCE,
                Wording::Words(connection.chance_percentage.clone()?),
            )),
            "Seasonal" => {
                // The reader translates only four seasons; the reference wrapper keeps the first.
                let season = connection.seasons_available.first()?;
                Some((ConnType::SEASONAL, Wording::Translated(season.to_owned())))
            }
            _ => None,
        }
    }
}

/// "Requires to have seen the first four endings." as the bare condition the dump publishes:
/// "Have seen the first four endings".
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

/// What the wiki says about one of a connection's conditions, if anything.
pub enum Wording {
    /// The flag alone, without words.
    None,
    /// The wiki's own English words.
    Words(String),
    /// English words with a Japanese translation: only the four seasons.
    Translated(String),
}

impl Wording {
    /// The words, and their Japanese translation where one exists.
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

/// The reference implementation's `/data` schema. Effects, menu themes, wallpapers and soundtrack
/// entries are wiki prose rather than store entries, so are published empty.
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
    /// When the whole wiki was last read without first querying what had changed; a soft sync
    /// keeps the previous value.
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

/// [`World::id`] is the world's index in the published list, which [`Connection::target_id`]
/// indexes.
impl yumezu_routing::World for World {
    fn title(&self) -> &str {
        &self.title
    }

    fn connections(&self) -> &[Connection] {
        &self.connections
    }
}

#[derive(Serialize, Deserialize)]
pub struct World {
    pub id: usize,
    /// Like `id`, but is assigned from a monotonic counter. See `cells` in [`crate::sync`].
    #[serde(default)]
    pub cell: Option<usize>,
    pub title: String,
    #[serde(rename = "titleJP")]
    pub title_jp: Option<String>,
    /// Empty, never absent, for a world credited to nobody: the reader groups worlds by author.
    pub author: String,
    /// Steps from the starting room along unconditional connections. See `depth`.
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
    /// Set for the debug room and any world an operator marks as a spoiler. A secret world stays
    /// in the dump and the numbering; the client leaves the world out.
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

/// The invented tree of [`yumezu_routing::fixture`], as a dump this program could have published.
#[cfg(test)]
pub fn dream_tree() -> Dump {
    let worlds = yumezu_routing::fixture::dream_tree()
        .into_iter()
        .enumerate()
        .map(|(id, place)| World {
            id,
            cell: Some(id),
            title: place.title.to_owned(),
            // Every optional field is set somewhere, so the round trip covers each.
            title_jp: (id % 2 == 0).then(|| format!("\u{68a6}{id}")),
            author: "Yumemiru".to_owned(),
            depth: 0,
            min_depth: 0,
            filename: format!("{}.png", place.title.replace([' ', '\''], "_")),
            map_url: (id % 3 == 0).then(|| format!("map{id}.png|map{id}b.png")),
            map_label: (id % 3 == 0).then(|| "Ground floor|Upper floor".to_owned()),
            bgm_url: (id % 4 == 0).then(|| format!("bgm{id}.mp3")),
            bgm_label: (id % 4 == 0).then(|| "Lantern^by the water".to_owned()),
            ver_added: Some(format!("0.1{id:02}")),
            ver_removed: (id == 9).then(|| "0.118".to_owned()),
            ver_updated: (id == 5).then(|| {
                vec![VerUpdated {
                    ver_updated: "0.112".to_owned(),
                    update_type: "+".to_owned(),
                }]
            }),
            ver_gaps: (id == 11).then(|| {
                vec![VerGap {
                    ver_removed: "0.113".to_owned(),
                    ver_readded: "0.114".to_owned(),
                }]
            }),
            removed: id == 9,
            secret: false,
            connections: place.connections,
        })
        .collect();

    Dump {
        worlds,
        authors: vec![Author {
            name: "Yumemiru".to_owned(),
            name_jp: None,
        }],
        versions: (0..16)
            .map(|n| Version {
                name: format!("0.1{n:02}"),
                authors: Some("Yumemiru".to_owned()),
                release_date: Some(format!("2020-01-{:02}", n + 1)),
            })
            .collect(),
        effects: vec![],
        menu_themes: vec![],
        wallpapers: vec![],
        bgm_tracks: vec![],
        last_update: Some("2020-01-16T00:00:00Z".to_owned()),
        last_full_update: Some("2020-01-16T00:00:00Z".to_owned()),
        is_admin: false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_dump_is_written_back_exactly_as_it_was_read() {
        let json = serde_json::to_string(&super::dream_tree()).expect("a dump is serializable");
        let read: super::Dump = serde_json::from_str(&json).expect("the dump this program writes");
        assert_eq!(
            serde_json::to_string(&read).expect("a dump is serializable"),
            json
        );
    }

    // A sync that cannot parse the last dump silently loses secret marks and atlas cells.
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
        assert!(world.secret);
        assert_eq!(world.cell, None);
    }
}
