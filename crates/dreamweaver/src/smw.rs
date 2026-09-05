//! Asking yume.wiki's Semantic MediaWiki store for itself, rather than through [the wrapper].
//!
//! The store is where the wiki keeps the structured half of what it knows: a world's infobox, the
//! passages out of it, the people credited for it and the releases it lived through are all
//! properties and subobjects, not prose. The wrapper reads them and hands them back as JSON, which
//! is why the dump was built out of it to begin with; this module asks the same questions of the
//! same store, and it asks them because for most of the dump the wrapper is now a detour rather
//! than a service:
//!
//! - The **version history** it does not publish at all, so `versionInfoData` went out empty for
//!   want of an endpoint. See [`versions`].
//! - The **connections** it publishes only the first few thousand of. The store refuses to look
//!   further than [`MAX_OFFSET`] rows into a result set, and rather than saying so it answers with
//!   the first page again -- which is what the wrapper's `continueKey` passes on when it appears
//!   to wrap. Yume 2kki has more passages than that, so alphabetically the last sixty-odd worlds'
//!   exits were silently missing from every dump. Asking directly does not lift the cap; it lets
//!   the question be cut into pieces that fit under it. See [`connections`].
//! - The **worlds** and the **authors** it answers correctly, and this asks the store anyway,
//!   because a fetch that goes to the same place as the rest can be steered by the same account of
//!   what has changed -- see [`crate::wiki::Client::changed_since`] -- and because two hops are
//!   one more thing between the dump and the wiki than the dump needs. Both queries were checked
//!   against the wrapper's answers field by field before the switch: identical worlds, pictures,
//!   maps, music and versions, and an author list identical down to its order. The one difference
//!   is that the store writes a world's several primary authors as several values where the
//!   wrapper writes them as one comma-separated string, which [`LocationRow::location`] does too.
//!
//! What remains of the wrapper is the galleries, which the store does not hold: the pictures on a
//! world's page are page content rather than properties.
//!
//! Nothing here reads wiki markup. A subobject is structured data that happens to live on a wiki
//! page, and this module asks for the same properties the wrapper would.
//!
//! [the wrapper]: https://github.com/ynoproject/wikiwrapper

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::wiki::{Author, Bgm, Connection, Location, LocationMap, ORIGIN, Result, WIKI};

/// What every page a dump is built from is titled under, and what the store answers with.
const PREFIX: &str = "Yume 2kki:";

/// How many rows one request asks for. The store's own ceiling; a smaller number only costs more
/// requests for the same rows.
const LIMIT: u32 = 500;

/// How far into a result set the store will look before it stops telling the truth.
///
/// A request past this comes back with the first page and an offset that carries on counting, so
/// there is no answer to trust and nothing to notice it by. Every query here is shaped to stay
/// under it, and one that does not is cut short with a complaint rather than quietly wrapped.
const MAX_OFFSET: u32 = 5000;

/// Every release the wiki dates, newest first.
///
/// Patches as well as versions: a world's infobox names whichever release added it, and half of
/// those are patches, so a history with only the round numbers in it would fail to date them.
pub async fn versions(http: &reqwest::Client) -> Result<Vec<Version>> {
    let rows: Vec<(String, VersionRow)> = ask(
        http,
        "Is part of game::Yume 2kki|Version/Type::+",
        "Version|Version/Type|Has contributing author|Version/Date",
        "sort=Version/Date|order=desc",
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(_, row)| {
            Some(Version {
                name: row.name.into_iter().next()?,
                authors: row.authors,
                released: row.dated.first().and_then(Date::released),
            })
        })
        .collect())
}

/// Every passage leaving a world whose title begins with one of `initials`, kept in those pieces.
///
/// The question is cut up because the whole of it does not fit under [`MAX_OFFSET`]. A piece is
/// every passage out of a world whose page begins with one character -- the store matches a page
/// name with a wildcard -- so the pieces cannot overlap, and the first characters of every world
/// the dump is being built out of are together every passage there is. The largest piece is a few
/// hundred rows.
///
/// They stay in their pieces rather than being poured into one list because a passage belongs to
/// the page that writes it up, so a piece is exactly what one edited world can invalidate: a soft
/// sync re-asks the pieces the wiki says have moved and keeps the rest of the last answer. See
/// [`crate::sync::Fetched`].
pub async fn connections(
    http: &reqwest::Client,
    initials: impl Iterator<Item = char>,
) -> Result<BTreeMap<char, Vec<Connection>>> {
    let mut shards = BTreeMap::new();
    for initial in initials.collect::<BTreeSet<char>>() {
        let rows: Vec<(String, ConnectionRow)> = ask(
            http,
            &format!("~{PREFIX}{initial}*|Is subobject type::connection"),
            "Connection/Origin|Connection/Location|Connection/Attribute|Connection/Unlock \
             conditions|Connection/Effects needed|Connection/Season available|Connection/Chance \
             percentage|Connection/Is removed",
            "",
        )
        .await?;
        shards.insert(
            initial,
            rows.into_iter()
                .filter_map(|(_, row)| row.passage())
                .collect(),
        );
    }
    Ok(shards)
}

/// Every location the wiki documents, with the fields a world page carries in its infobox.
///
/// One query for sixteen hundred worlds: the pictures, music and maps hang off a world as
/// subobjects, and the store writes them into the same answer rather than making a request of each.
pub async fn locations(http: &reqwest::Client) -> Result<Vec<Location>> {
    let rows: Vec<(String, LocationRow)> = ask(
        http,
        "Category:Yume 2kki Locations",
        "Has location image|Has primary author|Japanese name|Has BGM|Has location map|Version \
         added|Versions updated|Version removed|Version gaps",
        "",
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|(subject, row)| row.location(without_namespace(&subject)))
        .collect())
}

/// Everyone the wiki credits, in both the names it credits them under.
///
/// Sorted by the store rather than here, because that is the order the dump has always published
/// them in and an author list is something a reader shows as it comes.
pub async fn authors(http: &reqwest::Client) -> Result<Vec<Author>> {
    let rows: Vec<(String, AuthorRow)> = ask(
        http,
        &format!("-Has subobject::{PREFIX}Authors"),
        "Author/Name|Author/Original Name",
        "sort=Author/Name|order=asc",
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(_, row)| {
            Some(Author {
                name: row.name.into_iter().next()?,
                original_name: row.original.into_iter().next().and_then(Monolingual::text),
            })
        })
        .collect())
}

/// A release, as the store dates it.
pub struct Version {
    /// As the version history names it: `0.129d`, or `0.129c patch 27`.
    pub name: String,
    /// Everyone credited for it, in the store's own order.
    pub authors: Vec<String>,
    /// The day it came out, ISO 8601. `None` for a release the store dates unreadably.
    pub released: Option<String>,
}

/// One request, and the ones after it that the answer says are still to come.
///
/// The store pages by row offset rather than by cursor, so walking one is counting. It stops at
/// the last page, and short of [`MAX_OFFSET`] with a complaint: past that the answers are the
/// first page over again, and taking them would be worse than missing them.
///
/// Each row comes back with the subject it was found on, which for a page query is the page's
/// title and for a subobject query is a name of the store's own devising. Only [`locations`] has
/// any use for it; the rest throw it away, since a subobject says what it belongs to in its own
/// properties.
async fn ask<R: DeserializeOwned>(
    http: &reqwest::Client,
    conditions: &str,
    printouts: &str,
    parameters: &str,
) -> Result<Vec<(String, R)>> {
    let mut all = Vec::new();
    let mut offset = 0;
    loop {
        let separator = if parameters.is_empty() { "" } else { "|" };
        let answer: Answer<R> = http
            .get(WIKI)
            .header(reqwest::header::ORIGIN, ORIGIN)
            .query(&[
                ("action", "askargs"),
                ("format", "json"),
                ("api_version", "3"),
                ("conditions", conditions),
                ("printouts", printouts),
                (
                    "parameters",
                    &format!("limit={LIMIT}|offset={offset}{separator}{parameters}"),
                ),
            ])
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        all.extend(answer.query.results.into_iter().flat_map(|subject| {
            subject
                .into_iter()
                .map(|(name, found)| (name, found.printouts))
        }));
        match answer.next {
            // Only forwards: an offset that has stopped growing is the store having wrapped, and
            // whatever it hands back next has already been read.
            Some(next) if next > offset && next <= MAX_OFFSET => offset = next,
            Some(next) if next > MAX_OFFSET => {
                tracing::warn!("{conditions} is too large to read past row {next}");
                return Ok(all);
            }
            _ => return Ok(all),
        }
    }
}

/// What the store answers with: the rows, and where the ones after them start.
#[derive(Deserialize)]
struct Answer<R> {
    query: Found<R>,
    #[serde(rename = "query-continue-offset")]
    next: Option<u32>,
}

#[derive(Deserialize)]
struct Found<R> {
    /// One entry per subject, keyed by the page or subobject it was found on -- which is a name
    /// this program has no use for, since every row says what it belongs to in its own fields.
    results: Vec<HashMap<String, Subject<R>>>,
}

#[derive(Deserialize)]
struct Subject<R> {
    printouts: R,
}

/// A page the store points at, of which only the title is read.
#[derive(Deserialize)]
struct Page {
    fulltext: String,
}

impl Page {
    /// The title as the rest of the program spells it.
    fn title(&self) -> String {
        without_namespace(&self.fulltext)
    }
}

/// A title without the namespace the store writes into every one of them, which is how every
/// other part of this program spells a world.
pub fn without_namespace(title: &str) -> String {
    title.strip_prefix(PREFIX).unwrap_or(title).to_owned()
}

/// One value of a property, as the store writes it *inside* a subobject.
///
/// The same property is a bare list at the top of an answer and this at the bottom of one: a
/// subobject's fields carry the property's name and type alongside the values, and the values are
/// under `item`. Nothing here reads the description, only what it describes.
#[derive(Deserialize)]
struct Cell<T> {
    #[serde(default = "Vec::new")]
    item: Vec<T>,
}

impl<T> Default for Cell<T> {
    /// An absent property is an empty one, which is what a world with no music or no map has.
    fn default() -> Self {
        Cell { item: Vec::new() }
    }
}

impl<T> Cell<T> {
    /// The first value, for the properties the wiki writes at most once.
    fn first(self) -> Option<T> {
        self.item.into_iter().next()
    }

    /// The first value or an empty string, for the fields the dump publishes as strings: the
    /// reader takes a picture's address as text and would refuse a dump that gave it null.
    fn text(self) -> String
    where
        T: Into<String>,
    {
        self.first().map(Into::into).unwrap_or_default()
    }
}

/// Text the store holds in a particular language, which is how it keeps an author's own spelling
/// of their name. Only the text is read; the language is always the one the name is written in.
#[derive(Deserialize)]
struct Monolingual {
    #[serde(rename = "Text", default)]
    text: Cell<String>,
}

impl Monolingual {
    fn text(self) -> Option<String> {
        self.text.first()
    }
}

/// A date the store holds, which it writes as a unix timestamp and a `raw` field of its own
/// devising.
#[derive(Deserialize)]
struct Date {
    timestamp: String,
}

impl Date {
    /// The instant, in the shape the dump writes its own stamps in.
    ///
    /// `None` for a timestamp that will not parse or does not name a moment, which is a release
    /// the reader shows undated rather than a sync worth failing.
    fn released(&self) -> Option<String> {
        let seconds = self.timestamp.parse().ok()?;
        Some(crate::sync::iso(
            time::OffsetDateTime::from_unix_timestamp(seconds).ok()?,
        ))
    }
}

/// One release's subobject. Every field is a list, since a property can be written more than
/// once, and every list can be empty.
#[derive(Deserialize)]
struct VersionRow {
    #[serde(rename = "Version", default)]
    name: Vec<String>,
    #[serde(rename = "Has contributing author", default)]
    authors: Vec<String>,
    #[serde(rename = "Version/Date", default)]
    dated: Vec<Date>,
}

/// One world's page, in the store's own property names.
///
/// Every field is a list because a property can be written more than once -- and several of these
/// genuinely are, which is the whole reason the dump has to decide what to do with the extras.
#[derive(Deserialize)]
struct LocationRow {
    #[serde(rename = "Has location image", default)]
    image: Vec<String>,
    #[serde(rename = "Has primary author", default)]
    authors: Vec<String>,
    #[serde(rename = "Japanese name", default)]
    japanese: Vec<String>,
    #[serde(rename = "Has BGM", default)]
    bgms: Vec<BgmRow>,
    #[serde(rename = "Has location map", default)]
    maps: Vec<MapRow>,
    #[serde(rename = "Version added", default)]
    added: Vec<String>,
    #[serde(rename = "Versions updated", default)]
    updated: Vec<String>,
    #[serde(rename = "Version removed", default)]
    removed: Vec<String>,
    #[serde(rename = "Version gaps", default)]
    gaps: Vec<String>,
}

impl LocationRow {
    /// The world this row describes, under the title the subject was found at.
    fn location(self, title: String) -> Location {
        Location {
            title,
            location_image: self.image.into_iter().next().unwrap_or_default(),
            original_name: self.japanese.into_iter().next(),
            // Joined rather than kept apart, because the dump publishes one author per world and
            // a reader groups worlds by that string. It is also exactly what the wrapper hands
            // back for the five worlds the wiki credits to two people.
            primary_author: (!self.authors.is_empty()).then(|| self.authors.join(", ")),
            bgms: self.bgms.into_iter().map(BgmRow::bgm).collect(),
            location_maps: self.maps.into_iter().map(MapRow::map).collect(),
            // Empty rather than absent for the handful of pages that are in the Locations
            // category and carry no infobox at all: they are worlds the wiki has not written up,
            // not worlds this program failed to read.
            version_added: self.added.into_iter().next().unwrap_or_default(),
            versions_updated: self.updated,
            version_removed: self.removed.into_iter().next(),
            version_gaps: self.gaps,
        }
    }
}

/// One track heard in a world, as a subobject of the world's page.
#[derive(Deserialize)]
struct BgmRow {
    #[serde(rename = "BGM/Title", default)]
    title: Cell<String>,
    #[serde(rename = "BGM/Label", default)]
    label: Cell<String>,
    /// Where the file itself is served from. Empty for the many tracks the wiki names but holds
    /// no recording of.
    #[serde(rename = "Has media path", default)]
    path: Cell<String>,
}

impl BgmRow {
    fn bgm(self) -> Bgm {
        Bgm {
            path: self.path.text(),
            title: self.title.first(),
            label: self.label.first(),
        }
    }
}

/// One map the wiki draws of a world, likewise.
#[derive(Deserialize)]
struct MapRow {
    #[serde(rename = "Location Map/Caption", default)]
    caption: Cell<String>,
    /// The picture's address. The store also names the `File:` page it lives on, which would need
    /// a request each to turn into addresses; this is the same thing already resolved.
    #[serde(rename = "Has image path", default)]
    path: Cell<String>,
}

impl MapRow {
    fn map(self) -> LocationMap {
        LocationMap {
            path: self.path.text(),
            caption: self.caption.text(),
        }
    }
}

/// One credited person's subobject on the wiki's Authors page.
#[derive(Deserialize)]
struct AuthorRow {
    #[serde(rename = "Author/Name", default)]
    name: Vec<String>,
    #[serde(rename = "Author/Original Name", default)]
    original: Vec<Monolingual>,
}

/// One passage's subobject, in the store's own property names.
#[derive(Deserialize)]
struct ConnectionRow {
    #[serde(rename = "Connection/Origin", default)]
    origin: Vec<Page>,
    #[serde(rename = "Connection/Location", default)]
    destination: Vec<Page>,
    #[serde(rename = "Connection/Attribute", default)]
    attributes: Vec<String>,
    #[serde(rename = "Connection/Unlock conditions", default)]
    unlock_conditions: Vec<String>,
    #[serde(rename = "Connection/Effects needed", default)]
    effects_needed: Vec<String>,
    #[serde(rename = "Connection/Season available", default)]
    seasons_available: Vec<String>,
    #[serde(rename = "Connection/Chance percentage", default)]
    chance_percentages: Vec<String>,
    #[serde(rename = "Connection/Is removed", default)]
    is_removed: Vec<String>,
}

impl ConnectionRow {
    /// The passage this row describes, or `None` for a row missing an end.
    ///
    /// Where the wiki has written a property more than once the first is taken, which is what the
    /// wrapper does with the same rows: a passage open in three seasons is shown as open in the
    /// first of them, because the reader has one word to show and four it knows how to translate.
    fn passage(self) -> Option<Connection> {
        Some(Connection {
            origin: self.origin.first()?.title(),
            destination: self.destination.first()?.title(),
            attributes: self.attributes,
            unlock_condition: self.unlock_conditions.into_iter().next(),
            effects_needed: self.effects_needed,
            season_available: self.seasons_available.into_iter().next(),
            chance_percentage: self.chance_percentages.into_iter().next(),
            // Written as the store's own truth value rather than as JSON's.
            is_removed: self.is_removed.first().is_some_and(|flag| flag == "t"),
        })
    }
}

#[cfg(test)]
mod tests {
    /// The store's answer, as it actually writes one: a list of subjects keyed by the subobject
    /// they were found on, every property a list however many values it holds, and pages written
    /// with the namespace on the front. A shape misread here is a passage silently missing from
    /// the dump rather than a sync that fails, so the shape is pinned to the wiki's own bytes.
    #[test]
    fn a_passage_is_read_out_of_the_store_as_the_store_writes_it() {
        let answer = r#"{"query":{"results":[{"Yume 2kki:Snow Village#Connection-Ice Cave":{
            "printouts":{
            "Connection/Origin":[{"fulltext":"Yume 2kki:Snow Village","namespace":3002}],
            "Connection/Location":[{"fulltext":"Yume 2kki:Ice Cave","namespace":3002}],
            "Connection/Attribute":["Seasonal","Chance"],
            "Connection/Unlock conditions":[],
            "Connection/Effects needed":[],
            "Connection/Season available":["Fall","Summer","Winter"],
            "Connection/Chance percentage":["10%"],
            "Connection/Is removed":["f"]}}}]},"query-continue-offset":500}"#;
        let answer: super::Answer<super::ConnectionRow> =
            serde_json::from_str(answer).expect("the store's own answer");
        assert_eq!(answer.next, Some(500));
        let passage = answer
            .query
            .results
            .into_iter()
            .flat_map(|subject| subject.into_values())
            .map(|found| found.printouts)
            .next()
            .and_then(super::ConnectionRow::passage)
            .expect("the one passage");
        assert_eq!(passage.origin, "Snow Village");
        assert_eq!(passage.destination, "Ice Cave");
        assert_eq!(passage.attributes, ["Seasonal", "Chance"]);
        // Three seasons written, one shown: the reader has one word to put beside a passage and
        // four it knows how to translate. The wrapper takes the first of them too.
        assert_eq!(passage.season_available.as_deref(), Some("Fall"));
        assert_eq!(passage.chance_percentage.as_deref(), Some("10%"));
        assert!(!passage.is_removed, "the store writes its own truth values");
    }

    /// A world's picture, music and maps are not properties of its page but subobjects hanging
    /// off it, and the store writes a subobject's fields differently from a page's: the values are
    /// under `item`, beside a description of the property they belong to. Read that wrong and a
    /// world silently loses its music. The store's own bytes, for one world with one of each.
    #[test]
    fn a_world_is_read_out_of_the_store_with_what_hangs_off_it() {
        let answer = r#"{"query":{"results":[{"Yume 2kki:3D Structures Path":{"printouts":{
            "Has location image":["https://yume.wiki/images/0/02/3DStructures.png"],
            "Has primary author":["Kontentsu"],
            "Japanese name":["\u507d\u6d45\u702c\u306e\u5bb6"],
            "Has BGM":[{
                "BGM/Title":{"label":"BGM/Title","typeid":"_txt","item":["46202"]},
                "BGM/Label":{"label":"BGM/Label","typeid":"_txt","item":["Aooh's Trap"]},
                "Has media path":{"label":"Has media path","typeid":"_uri","item":[]}}],
            "Has location map":[{
                "Location Map/Caption":{"label":"Location Map/Caption","typeid":"_txt",
                    "item":["Map of 3D Structures Path"]},
                "Has image path":{"label":"Has image path","typeid":"_uri",
                    "item":["https://yume.wiki/images/3/3c/3D_Structures_Path_map.png"]}}],
            "Version added":["0.116a"],
            "Versions updated":["0.122g","0.124f patch 2"],
            "Version removed":[],
            "Version gaps":["0.120d patch 21-0.120e"]}}}]}}"#;
        let answer: super::Answer<super::LocationRow> =
            serde_json::from_str(answer).expect("the store's own answer");
        let (subject, row) = answer
            .query
            .results
            .into_iter()
            .flat_map(|subject| subject.into_iter().map(|(at, found)| (at, found.printouts)))
            .next()
            .expect("the one world");
        let world = row.location(super::without_namespace(&subject));
        assert_eq!(world.title, "3D Structures Path");
        assert_eq!(world.version_added, "0.116a");
        assert_eq!(world.versions_updated, ["0.122g", "0.124f patch 2"]);
        assert_eq!(world.version_removed, None);
        assert_eq!(world.version_gaps, ["0.120d patch 21-0.120e"]);
        let bgm = world.bgms.first().expect("the one track");
        assert_eq!(bgm.title.as_deref(), Some("46202"));
        assert_eq!(bgm.label.as_deref(), Some("Aooh's Trap"));
        // The wiki names far more tracks than it holds recordings of, and the dump publishes the
        // address as a string: a track with no file is an empty one, not a missing field.
        assert_eq!(bgm.path, "");
        let map = world.location_maps.first().expect("the one map");
        assert_eq!(map.caption, "Map of 3D Structures Path");
        assert!(map.path.ends_with("3D_Structures_Path_map.png"));
    }

    /// The store writes a world's several primary authors as several values; the dump publishes
    /// one string, and a reader groups the worlds by it. The wrapper joined them with a comma, and
    /// so does this -- five worlds' authorship depends on it.
    #[test]
    fn a_world_credited_to_two_people_is_credited_to_both() {
        let row = |authors: &[&str]| super::LocationRow {
            image: Vec::new(),
            authors: authors.iter().map(|name| (*name).to_owned()).collect(),
            japanese: Vec::new(),
            bgms: Vec::new(),
            maps: Vec::new(),
            added: Vec::new(),
            updated: Vec::new(),
            removed: Vec::new(),
            gaps: Vec::new(),
        };
        assert_eq!(
            row(&["FUMO", "Peperoncino III from Yamada Pref."])
                .location("Rice Bowl World".to_owned())
                .primary_author
                .as_deref(),
            Some("FUMO, Peperoncino III from Yamada Pref.")
        );
        // A world the wiki credits to nobody has no author rather than an empty one, which is
        // what keeps it out of a reader's list of authors.
        assert_eq!(
            row(&[]).location("FC Caverns".to_owned()).primary_author,
            None
        );
    }

    /// An author's own spelling of their name is held as text in a particular language, which the
    /// store writes as a little object rather than as a string.
    #[test]
    fn an_author_is_read_under_both_the_names_the_wiki_gives_them() {
        let answer = r#"{"query":{"results":[{"Yume 2kki:Authors# 56aa30":{"printouts":{
            "Author/Name":["kirin"],
            "Author/Original Name":[{
                "Text":{"label":"Text","key":"_TEXT","typeid":"_txt","item":["\u30ad\u30ea\u30f3"]},
                "Language code":{"label":"Language code","key":"_LCODE","typeid":"__lcode",
                    "item":["ja"]}}]}}}]}}"#;
        let answer: super::Answer<super::AuthorRow> =
            serde_json::from_str(answer).expect("the store's own answer");
        let row = answer
            .query
            .results
            .into_iter()
            .flat_map(|subject| subject.into_values().map(|found| found.printouts))
            .next()
            .expect("the one author");
        assert_eq!(row.name, ["kirin"]);
        assert_eq!(
            row.original
                .into_iter()
                .next()
                .and_then(super::Monolingual::text),
            Some("キリン".to_owned())
        );
    }

    /// A release is dated in seconds and published in the same stamp the dump stamps itself with,
    /// since a reader that takes the day off one has to be able to take it off the other.
    #[test]
    fn a_release_is_dated_the_way_the_dump_dates_itself() {
        let dated = |timestamp: &str| {
            super::Date {
                timestamp: timestamp.to_owned(),
            }
            .released()
        };
        assert_eq!(
            dated("1788393600").as_deref(),
            Some("2026-09-03T00:00:00.000Z")
        );
        assert_eq!(dated("").as_deref(), None, "an undated release is undated");
    }
}
