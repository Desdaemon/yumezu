//! Asking yume.wiki's Semantic MediaWiki store for itself, rather than through [the wrapper].
//!
//! The store keeps the structured half of what the wiki knows: a world's infobox, the connections
//! out of it, the people credited for it and the releases it lived through are properties and
//! subobjects, not prose. The wrapper reads them and hands them back as JSON, which is why the dump
//! was built out of it to begin with. For most of the dump it is now a detour:
//!
//! - The **version history** it does not publish at all. See [`versions`].
//! - The **connections** it publishes only the first few thousand of. The store refuses to look
//!   further than [`MAX_OFFSET`] rows into a result set, and rather than saying so it answers with
//!   the first page again -- which is what the wrapper's `continueKey` passes on when it appears to
//!   wrap. Yume 2kki has more connections than that, so alphabetically the last sixty-odd worlds'
//!   exits were silently missing from every dump. Asking directly does not lift the cap; it lets
//!   the question be cut into pieces that fit under it. See [`connections`].
//! - The **worlds** and the **authors** it answers correctly, and this asks the store anyway so
//!   every fetch can be steered by the same account of what has changed -- see [`changed_since`].
//!   The one difference from the wrapper is that the store writes a world's several primary authors
//!   as several values rather than one comma-separated string, which [`LocationRow::location`]
//!   rejoins.
//!
//! A world's gallery is page content rather than properties, and is not published: reading it meant
//! a second host and a second response format for pictures nothing shows.
//!
//! [the wrapper]: https://github.com/ynoproject/wikiwrapper

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Behind an edge that answers a plain request with a challenge page, which [`ORIGIN`] is for.
const WIKI: &str = "https://yume.wiki/api.php";

/// What the wiki's edge wants to see before it answers an API request rather than serving a
/// challenge page: the explorer this program stands in for.
const ORIGIN: &str = "https://explorer.yume.wiki";

/// Yume 2kki's namespace on a wiki housing a dozen games under one installation.
const NAMESPACE: &str = "3002";

/// Everything that can go wrong here is an HTTP request or the JSON it came back as.
pub type Error = reqwest::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// What every page a dump is built from is titled under, and what the store answers with.
const PREFIX: &str = "Yume 2kki:";

/// The store's own ceiling. A smaller number only costs more requests for the same rows.
const LIMIT: u32 = 500;

/// How far into a result set the store will look before it stops telling the truth: a request past
/// this comes back with the first page and an offset that carries on counting, so there is nothing
/// to notice it by. Every query here is shaped to stay under it, and one that does not is cut short
/// with a complaint rather than quietly wrapped.
const MAX_OFFSET: u32 = 5000;

/// Newest first, patches included: a world's infobox names whichever release added it and half of
/// those are patches.
pub async fn versions(http: &reqwest::Client) -> Result<Vec<Version>> {
    let rows = askargs::<VersionRow>(
        http,
        "Is part of game::Yume 2kki|Version/Type::+",
        "Version|Has contributing author|Version/Date",
        "sort=Version/Date|order=desc",
    )
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(_, row)| {
            Some(Version {
                name: row.name?,
                authors: row.authors,
                released: row.dated.and_then(|dated| dated.released()),
            })
        })
        .collect())
}

/// Every connection leaving a world whose title begins with one of `initials`, kept in those
/// pieces.
///
/// The question is cut up because the whole of it does not fit under [`MAX_OFFSET`]. A piece is
/// every connection out of a world whose page begins with one character, so the pieces cannot
/// overlap and together cover every connection there is; the largest is a few hundred rows.
///
/// The pieces are kept apart because a connection belongs to the page that writes it up, so a piece
/// is exactly what one edited world can invalidate: a soft sync re-asks only the moved pieces.
pub async fn connections(
    http: &reqwest::Client,
    initials: BTreeSet<char>,
) -> Result<BTreeMap<char, Vec<Connection>>> {
    let mut shards = BTreeMap::new();
    for initial in initials {
        let rows = askargs::<ConnectionRow>(
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
                .filter_map(|(_, row)| row.connection())
                .collect(),
        );
    }
    Ok(shards)
}

/// Every location the wiki documents, with the fields a world page carries in its infobox.
///
/// One query for sixteen hundred worlds: the pictures, music and maps hang off a world as
/// subobjects, and the store writes them into the same answer rather than a request each.
pub async fn locations(http: &reqwest::Client) -> Result<Vec<Location>> {
    let rows = askargs::<LocationRow>(
        http,
        "Category:Yume 2kki Locations",
        "Has location image|Has primary author|Japanese name|Has BGM|Has location map|Map \
         IDs|Version added|Versions updated|Version removed|Version gaps",
        "",
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|(subject, row)| row.location(without_namespace(&subject)))
        .collect())
}

/// Sorted by the store rather than here, that being the order the dump has always published.
pub async fn authors(http: &reqwest::Client) -> Result<Vec<Author>> {
    let rows: Vec<(String, AuthorRow)> = askargs(
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
                name: row.name?,
                original_name: row
                    .original
                    .into_iter()
                    .flat_map(|original| original.text)
                    .collect(),
            })
        })
        .collect())
}

/// Every page in Yume 2kki's namespace touched since `when`, an ISO 8601 instant. What a soft sync
/// steers by.
///
/// Edits, new pages and log entries all count, so a world deleted or renamed reads as a change.
/// Only Yume 2kki's namespace is looked at, which is the hole in it: a template or file the worlds
/// are built out of lives elsewhere, and an edit to one changes what the store answers without any
/// page here being touched. A full sync on start-up is the backstop.
///
/// Titles come back without the namespace, each once however many times it was edited.
pub async fn changed_since(http: &reqwest::Client, when: &str) -> Result<Vec<String>> {
    let mut titles = std::collections::BTreeSet::new();
    let mut carry: Option<String> = None;
    loop {
        let mut request = http
            .get(WIKI)
            .header(reqwest::header::ORIGIN, ORIGIN)
            .query(&[
                ("action", "query"),
                ("list", "recentchanges"),
                ("rcnamespace", NAMESPACE),
                ("rctype", "edit|new|log"),
                // Oldest first, from the newest change already accounted for.
                ("rcdir", "newer"),
                ("rcstart", when),
                ("rclimit", "500"),
                ("rcprop", "title"),
                ("format", "json"),
                ("formatversion", "2"),
            ]);
        if let Some(carry) = &carry {
            request = request.query(&[("rccontinue", carry), ("continue", &"-||".to_owned())]);
        }
        let changes: RecentChanges = request.send().await?.error_for_status()?.json().await?;
        titles.extend(
            changes
                .query
                .recentchanges
                .into_iter()
                .map(|change| without_namespace(&change.title)),
        );
        match changes.carry.and_then(|carry| carry.rccontinue) {
            Some(next) => carry = Some(next),
            None => return Ok(titles.into_iter().collect()),
        }
    }
}

pub struct Version {
    /// As the version history names it: `0.129d`, or `0.129c patch 27`.
    pub name: String,
    /// Everyone credited for it, in the store's own order.
    pub authors: Vec<String>,
    /// ISO 8601. `None` for a release the store dates unreadably.
    pub released: Option<String>,
}

/// Only the fields the dump carries. The store also holds the colours a world's page is themed in
/// and the authors beyond the primary one, which [`locations`] does not ask for.
pub struct Location {
    /// The English page title, which is the identity everything else refers to it by.
    pub title: String,
    /// Empty for a world the wiki has no headline picture for.
    pub location_image: String,
    /// As the game itself names it. Absent for the worlds the wiki has not recorded one for.
    pub original_name: Option<String>,
    /// Who the wiki credits the world to, comma-separated where that is more than one person.
    /// Absent where it credits nobody.
    pub primary_author: Option<String>,
    pub bgms: Vec<Bgm>,
    pub location_maps: Vec<LocationMap>,
    /// The RPG Maker maps the world is built out of, by the number the game gives each. Empty for
    /// the three pages in the category whose infobox names none.
    pub map_ids: Vec<u32>,
    /// As the version history names it. Empty for a page in the category with no infobox at all.
    pub version_added: String,
    /// Every release that changed it, each optionally suffixed with what kind of change it was.
    pub versions_updated: Vec<String>,
    /// The release it was taken out in, for a world that is no longer in the game.
    pub version_removed: Option<String>,
    /// Spans it was absent for and came back from, each written `<removed>-<readded>`.
    pub version_gaps: Vec<String>,
}

/// One track heard in a world.
pub struct Bgm {
    /// Where the wiki serves the file from. Empty for a track it holds no file of.
    pub path: String,
    /// Usually its filename in the game.
    pub title: Option<String>,
    /// Where in the world it plays.
    pub label: Option<String>,
}

/// One map the wiki draws of a world.
pub struct LocationMap {
    pub path: String,
    /// The wiki's caption, which says which part of the world the map covers.
    pub caption: String,
}

/// A property in an answer is a list of values under a name of the wiki's choosing, so what
/// arrives is one list per field rather than one record per connection.
pub struct Connection {
    /// Title of the world it leads out of.
    pub origin: String,
    /// Title of the world it leads to.
    pub destination: String,
    /// The wiki's own vocabulary: `No Return`, `Locked`, `Chance` and so on.
    pub attributes: Vec<String>,
    /// The sentence a `Conditional` connection is gated behind.
    pub unlock_condition: Option<String>,
    /// The effects a `Needs Effect` connection wants the player to be wearing.
    pub effects_needed: Vec<String>,
    /// The wiki writes one value per season, so a connection open in three has three.
    pub seasons_available: Vec<String>,
    /// The odds a `Chance` connection opens at, as the wiki writes them.
    pub chance_percentage: Option<String>,
    /// A connection that used to exist and no longer does.
    pub is_removed: bool,
}

/// Someone the wiki credits.
pub struct Author {
    /// As the English wiki writes the name.
    pub name: String,
    /// As the author writes it themselves, where that differs.
    pub original_name: Vec<String>,
}

#[derive(Deserialize)]
struct RecentChanges {
    query: Changes,
    /// Where the next page starts, absent on the last one. The wiki nests it under `continue`
    /// rather than alongside the results.
    #[serde(rename = "continue")]
    carry: Option<Carry>,
}

#[derive(Deserialize)]
struct Carry {
    rccontinue: Option<String>,
}

#[derive(Deserialize)]
struct Changes {
    recentchanges: Vec<Change>,
}

#[derive(Deserialize)]
struct Change {
    title: String,
}

/// One request, and the ones after it the answer says are still to come.
///
/// The store pages by row offset rather than by cursor. This stops at the last page, and short of
/// [`MAX_OFFSET`] with a complaint: past that the answers are the first page over again, and taking
/// them would be worse than missing them.
///
/// Each row comes back with the subject it was found on -- a page title, or a subobject name of the
/// store's own devising. Only [`locations`] has any use for it.
async fn askargs<R: DeserializeOwned>(
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
            // Only forwards: an offset that has stopped growing is the store having wrapped.
            Some(next) if next > offset && next <= MAX_OFFSET => offset = next,
            Some(next) if next > MAX_OFFSET => {
                tracing::warn!("{conditions} is too large to read past row {next}");
                return Ok(all);
            }
            _ => return Ok(all),
        }
    }
}

#[derive(Deserialize)]
struct Answer<R> {
    query: Found<R>,
    #[serde(rename = "query-continue-offset")]
    next: Option<u32>,
}

#[derive(Deserialize)]
struct Found<R> {
    /// Keyed by the page or subobject a row was found on.
    results: Vec<HashMap<String, Subject<R>>>,
}

#[derive(Deserialize)]
struct Subject<R> {
    printouts: R,
}

#[derive(Deserialize)]
struct Page {
    fulltext: String,
}

impl Page {
    fn title(&self) -> String {
        without_namespace(&self.fulltext)
    }
}

/// Without the namespace the store writes into every title, which is how the rest of this program
/// spells a world.
pub fn without_namespace(title: &str) -> String {
    title.strip_prefix(PREFIX).unwrap_or(title).to_owned()
}

/// Refuses a sequence of more than one element in debug builds only, to catch a property the wiki
/// writes twice where the dump publishes one.
fn first<'de, D, T>(property: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct AtMostOne<T>(PhantomData<fn() -> T>);

    impl<'de, T> serde::de::Visitor<'de> for AtMostOne<T>
    where
        T: Deserialize<'de>,
    {
        type Value = Option<T>;

        fn expecting(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
            fmt.write_str("a sequence of at most one element")
        }

        fn visit_seq<A>(self, mut seq: A) -> std::result::Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let first = seq.next_element()?;
            let mut count = usize::from(first.is_some());
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {
                count += 1;
            }
            if count > 1 {
                if cfg!(any(test, debug_assertions)) {
                    return Err(serde::de::Error::invalid_length(count, &self));
                }
                tracing::warn!("expected at most one element, got {count}");
            }
            Ok(first)
        }
    }

    property.deserialize_seq(AtMostOne(PhantomData))
}

/// [`first`], for a value nested in an object under `item`.
fn first_in_cell<'de, D, T>(property: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(bound = "T: Deserialize<'de>")]
    struct Cell<T> {
        #[serde(default, deserialize_with = "first")]
        item: Option<T>,
    }

    Ok(Cell::<T>::deserialize(property)?.item)
}

/// A monolingual text value -- the name plus its language. Only the text is read; the language is
/// always the one the name is written in.
#[derive(Deserialize)]
struct Monolingual {
    #[serde(rename = "Text", default, deserialize_with = "first_in_cell")]
    text: Option<String>,
}

/// Written by the store as a unix timestamp and a `raw` field of its own devising.
#[derive(Deserialize)]
struct Date {
    timestamp: String,
}

impl Date {
    /// In the same ISO-8601 form the dump writes its own stamps in. `None` for an unparseable
    /// timestamp, which is a release the reader shows undated rather than a sync worth failing.
    fn released(&self) -> Option<String> {
        let seconds = self.timestamp.parse().ok()?;
        Some(crate::sync::iso(
            time::OffsetDateTime::from_unix_timestamp(seconds).ok()?,
        ))
    }
}

#[derive(Deserialize)]
struct VersionRow {
    #[serde(rename = "Version", default, deserialize_with = "first")]
    name: Option<String>,
    #[serde(rename = "Has contributing author", default)]
    authors: Vec<String>,
    #[serde(rename = "Version/Date", default, deserialize_with = "first")]
    dated: Option<Date>,
}

#[derive(Deserialize)]
struct LocationRow {
    #[serde(rename = "Has location image", default, deserialize_with = "first")]
    image: Option<String>,
    #[serde(rename = "Has primary author", default)]
    authors: Vec<String>,
    #[serde(rename = "Japanese name", default, deserialize_with = "first")]
    japanese: Option<String>,
    #[serde(rename = "Has BGM", default)]
    bgms: Vec<BgmRow>,
    #[serde(rename = "Has location map", default)]
    maps: Vec<MapRow>,
    #[serde(rename = "Map IDs", default)]
    map_ids: Vec<MapIdRow>,
    #[serde(rename = "Version added", default, deserialize_with = "first")]
    added: Option<String>,
    #[serde(rename = "Versions updated", default)]
    updated: Vec<String>,
    #[serde(rename = "Version removed", default, deserialize_with = "first")]
    removed: Option<String>,
    #[serde(rename = "Version gaps", default)]
    gaps: Vec<String>,
}

impl LocationRow {
    fn location(self, title: String) -> Location {
        Location {
            title,
            location_image: self.image.unwrap_or_default(),
            original_name: self.japanese,
            // The dump publishes one author per world and a reader groups worlds by that string.
            // Also what the wrapper hands back for the five worlds credited to two people.
            primary_author: (!self.authors.is_empty()).then(|| self.authors.join(", ")),
            bgms: self.bgms.into_iter().map(BgmRow::bgm).collect(),
            location_maps: self.maps.into_iter().map(MapRow::map).collect(),
            map_ids: self.map_ids.into_iter().filter_map(|row| row.id).collect(),
            // Empty rather than absent for the pages with no infobox at all: worlds the wiki has
            // not written up, not worlds this failed to read.
            version_added: self.added.unwrap_or_default(),
            versions_updated: self.updated,
            version_removed: self.removed,
            version_gaps: self.gaps,
        }
    }
}

/// The store also writes an annotation beside the number naming which part of the world that map
/// is. Not read: nothing published names maps one at a time.
#[derive(Deserialize)]
struct MapIdRow {
    #[serde(rename = "Has map ID", default, deserialize_with = "first_in_cell")]
    id: Option<u32>,
}

#[derive(Deserialize)]
struct BgmRow {
    #[serde(rename = "BGM/Title", default, deserialize_with = "first_in_cell")]
    title: Option<String>,
    #[serde(rename = "BGM/Label", default, deserialize_with = "first_in_cell")]
    label: Option<String>,
    /// Absent for the many tracks the wiki names but holds no recording of.
    #[serde(rename = "Has media path", default, deserialize_with = "first_in_cell")]
    path: Option<String>,
}

impl BgmRow {
    fn bgm(self) -> Bgm {
        Bgm {
            // The dump publishes an address as a string, and the reader would refuse a null.
            path: self.path.unwrap_or_default(),
            title: self.title,
            label: self.label,
        }
    }
}

#[derive(Deserialize)]
struct MapRow {
    #[serde(
        rename = "Location Map/Caption",
        default,
        deserialize_with = "first_in_cell"
    )]
    caption: Option<String>,
    /// The store also names the `File:` page, which would need a request each to resolve. This is
    /// the same thing already resolved.
    #[serde(rename = "Has image path", default, deserialize_with = "first_in_cell")]
    path: Option<String>,
}

impl MapRow {
    fn map(self) -> LocationMap {
        LocationMap {
            path: self.path.unwrap_or_default(),
            caption: self.caption.unwrap_or_default(),
        }
    }
}

#[derive(Deserialize)]
struct AuthorRow {
    #[serde(rename = "Author/Name", default, deserialize_with = "first")]
    name: Option<String>,
    #[serde(rename = "Author/Original Name", default)]
    original: Vec<Monolingual>,
}

#[derive(Deserialize)]
struct ConnectionRow {
    #[serde(rename = "Connection/Origin", default, deserialize_with = "first")]
    origin: Option<Page>,
    #[serde(rename = "Connection/Location", default, deserialize_with = "first")]
    destination: Option<Page>,
    #[serde(rename = "Connection/Attribute", default)]
    attributes: Vec<String>,
    #[serde(
        rename = "Connection/Unlock conditions",
        default,
        deserialize_with = "first"
    )]
    unlock_condition: Option<String>,
    #[serde(rename = "Connection/Effects needed", default)]
    effects_needed: Vec<String>,
    /// As of writing, 3 connections may be accessed in more than one season.
    #[serde(rename = "Connection/Season available", default)]
    seasons_available: Vec<String>,
    #[serde(
        rename = "Connection/Chance percentage",
        default,
        deserialize_with = "first"
    )]
    chance_percentage: Option<String>,
    #[serde(rename = "Connection/Is removed", default, deserialize_with = "first")]
    is_removed: Option<String>,
}

impl ConnectionRow {
    /// `None` for a row missing an end.
    fn connection(self) -> Option<Connection> {
        Some(Connection {
            origin: self.origin?.title(),
            destination: self.destination?.title(),
            attributes: self.attributes,
            unlock_condition: self.unlock_condition,
            effects_needed: self.effects_needed,
            seasons_available: self.seasons_available,
            chance_percentage: self.chance_percentage,
            // The store's own truth value rather than JSON's.
            is_removed: self.is_removed.is_some_and(|flag| flag == "t"),
        })
    }
}

#[cfg(test)]
mod tests {
    /// A misread field here is a connection silently missing from the dump rather than a sync that
    /// fails, so it is pinned to the wiki's own bytes.
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
        let connection = answer
            .query
            .results
            .into_iter()
            .flat_map(|subject| subject.into_values())
            .map(|found| found.printouts)
            .next()
            .and_then(super::ConnectionRow::connection)
            .expect("the one connection");
        assert_eq!(connection.origin, "Snow Village");
        assert_eq!(connection.destination, "Ice Cave");
        assert_eq!(connection.attributes, ["Seasonal", "Chance"]);
        // Every season the wiki wrote is kept: which one the dump publishes is `crate::model`'s
        // decision, not this reader's to make by dropping values.
        assert_eq!(connection.seasons_available, ["Fall", "Summer", "Winter"]);
        assert_eq!(connection.chance_percentage.as_deref(), Some("10%"));
        assert!(
            !connection.is_removed,
            "the store writes its own truth values"
        );
    }

    /// A field read as one value is a claim about the wiki, and a debug build holds it to that so
    /// the model is widened rather than the second value quietly lost. A release build takes the
    /// first and carries on.
    #[test]
    fn a_property_the_dump_publishes_once_is_refused_when_the_wiki_writes_two() {
        let one_each = r#"{"query":{"results":[{"Yume 2kki:Authors# 56aa30":{"printouts":{
            "Author/Name":["kirin"]}}}]}}"#;
        let two_names = r#"{"query":{"results":[{"Yume 2kki:Authors# 56aa30":{"printouts":{
            "Author/Name":["kirin","キリン"]}}}]}}"#;
        let read = |json| serde_json::from_str::<super::Answer<super::AuthorRow>>(json);

        assert!(read(one_each).is_ok(), "one name is one name");
        let complaint = match read(two_names) {
            Err(complaint) => complaint.to_string(),
            Ok(_) => panic!("two names read into a field that publishes one"),
        };
        assert!(
            complaint.contains("2") && complaint.contains("at most one"),
            "the complaint says what was written and what was expected: {complaint}"
        );
    }

    /// A world's picture, music and maps are subobjects rather than properties, and the store
    /// writes a subobject's fields under `item` beside a description of the property. Read that
    /// wrong and a world silently loses its music.
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
            "Map IDs":[{
                "Has map ID":{"label":"Has map ID","typeid":"_num","item":[1344]},
                "Map ID annotation":{"label":"Map ID annotation","typeid":"_txt","item":[]}},{
                "Has map ID":{"label":"Has map ID","typeid":"_num","item":[884]},
                "Map ID annotation":{"label":"Map ID annotation","typeid":"_txt",
                    "item":["Main Area"]}}],
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
        // The dump publishes the address as a string: a track with no file is an empty one.
        assert_eq!(bgm.path, "");
        let map = world.location_maps.first().expect("the one map");
        assert_eq!(map.caption, "Map of 3D Structures Path");
        assert!(map.path.ends_with("3D_Structures_Path_map.png"));
        // A number rather than a string, in the order the wiki lists them.
        assert_eq!(world.map_ids, [1344, 884]);
    }

    /// The dump publishes one string and a reader groups the worlds by it. The wrapper joined them
    /// with a comma, and so does this -- five worlds' authorship depends on it.
    #[test]
    fn a_world_credited_to_two_people_is_credited_to_both() {
        let row = |authors: &[&str]| super::LocationRow {
            image: None,
            authors: authors.iter().map(|name| (*name).to_owned()).collect(),
            japanese: None,
            bgms: Vec::new(),
            map_ids: Vec::new(),
            maps: Vec::new(),
            added: None,
            updated: Vec::new(),
            removed: None,
            gaps: Vec::new(),
        };
        assert_eq!(
            row(&["FUMO", "Peperoncino III from Yamada Pref."])
                .location("Rice Bowl World".to_owned())
                .primary_author
                .as_deref(),
            Some("FUMO, Peperoncino III from Yamada Pref.")
        );
        // No author rather than an empty one, which keeps it out of a reader's author list.
        assert_eq!(
            row(&[]).location("FC Caverns".to_owned()).primary_author,
            None
        );
    }

    /// The store writes a name-in-a-language as a little object rather than as a string.
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
        assert_eq!(row.name.as_deref(), Some("kirin"));
        assert_eq!(
            row.original
                .as_slice()
                .iter()
                .flat_map(|it| it.text.as_deref())
                .collect::<Vec<_>>(),
            vec!["キリン"]
        );
    }

    /// The same stamp the dump stamps itself with: a reader that takes the day off one has to be
    /// able to take it off the other.
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

    /// The continuation is nested under `continue` rather than sitting beside the results, so a
    /// misread would stop the walk one page in and leave the rest of a week's editing unnoticed.
    #[test]
    fn the_changed_pages_are_read_out_of_the_wiki_with_the_page_after_them() {
        let quiet = r#"{"batchcomplete":true,"query":{"recentchanges":[]}}"#;
        let busy = r#"{"batchcomplete":true,"continue":{"rccontinue":"20260905023134|8675309",
            "continue":"-||"},"query":{"recentchanges":[
            {"type":"edit","ns":3002,"title":"Yume 2kki:Snow Village"},
            {"type":"log","ns":3002,"title":"Yume 2kki:Authors"}]}}"#;
        let changes = |json| {
            serde_json::from_str::<super::RecentChanges>(json).expect("the wiki's own answer")
        };

        let quiet = changes(quiet);
        assert!(quiet.query.recentchanges.is_empty(), "nothing has moved");
        assert!(
            quiet.carry.and_then(|carry| carry.rccontinue).is_none(),
            "and so there is no page after this one"
        );

        let busy = changes(busy);
        let titles: Vec<String> = busy
            .query
            .recentchanges
            .iter()
            .map(|change| super::without_namespace(&change.title))
            .collect();
        // Without the namespace: how the rest of the program spells a world, and how a sync
        // recognises the two pages that are not worlds at all.
        assert_eq!(titles, ["Snow Village", "Authors"]);
        assert!(busy.carry.and_then(|carry| carry.rccontinue).is_some());
    }
}
