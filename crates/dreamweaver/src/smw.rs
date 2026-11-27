//! Queries yume.wiki's Semantic MediaWiki store directly: [the wrapper] omits version history
//! and passes on the store's silent wraparound past [`MAX_OFFSET`] rows.
//!
//! [the wrapper]: https://github.com/ynoproject/wikiwrapper

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::marker::PhantomData;

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Behind Cloudflare, which answers a request lacking [`ORIGIN`] with a challenge page.
const WIKI: &str = "https://yume.wiki/api.php";

/// The wiki explorer's origin, which Cloudflare lets through to the API.
pub const ORIGIN: &str = "https://explorer.yume.wiki";

const YUME2KKI_NAMESPACE: &str = "3002";

/// Every failure here is an HTTP request or the JSON response.
pub type Error = reqwest::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// The namespace prefix on every page title the store answers with.
const PREFIX: &str = "Yume 2kki:";

/// The store's own ceiling. A smaller number only costs more requests for the same rows.
const LIMIT: u32 = 500;

/// The deepest offset the store pages correctly; beyond, the store silently returns the first
/// page again. Queries are shaped to stay under; one that does not is cut short with a warning.
const MAX_OFFSET: u32 = 5000;

/// Newest first, patches included, since half the releases that infoboxes name are patches.
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
                released: row
                    .dated
                    .and_then(|record| record.full_date)
                    .and_then(|date| date.released()),
            })
        })
        .collect())
}

/// Every connection out of worlds whose title begins with one of `initials`, by initial. Each
/// shard stays under [`MAX_OFFSET`]; a soft sync re-reads only the shards edited titles fall in.
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

/// Every location's infobox fields, in one query: pictures, music and maps come as subobjects in
/// the same answer. Index pages are left out; see [`indexes`]. Galleries are page content, so not
/// published.
pub async fn locations(http: &reqwest::Client) -> Result<Vec<Location>> {
    let (rows, indexes) = tokio::try_join!(
        askargs::<LocationRow>(
            http,
            LOCATIONS,
            "Has location image|Has primary author|Japanese name|Has BGM|Has location map|Version \
             added|Versions updated|Version removed|Version gaps",
            "",
        ),
        indexes(http),
    )?;
    Ok(rows
        .into_iter()
        .map(|(subject, row)| row.location(without_namespace(&subject)))
        .filter(|location| !indexes.contains(&location.title))
        .collect())
}

const LOCATIONS: &str = "Category:Yume 2kki Locations";

/// Pages in [`LOCATIONS`] without an infobox: the index page of a multi-page area. Fetched
/// separately because the store cannot query "not in this category".
async fn indexes(http: &reqwest::Client) -> Result<BTreeSet<String>> {
    let rows = askargs::<serde::de::IgnoredAny>(
        http,
        &format!("{LOCATIONS}|Category:Multi-page Locations"),
        "",
        "",
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|(subject, _)| without_namespace(&subject))
        .collect())
}

/// Sorted by the store, in the order the dump has always published.
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

/// Titles, without namespace and deduplicated, of Yume 2kki pages edited, created, deleted or
/// renamed since `when` (ISO 8601). Misses template edits elsewhere; full syncs catch those.
pub async fn changed_since(http: &reqwest::Client, when: &str) -> Result<Vec<String>> {
    let mut titles = std::collections::BTreeSet::new();
    let mut carry: Option<String> = None;
    loop {
        let mut request = wiki_request(http).query(&[
            ("action", "query"),
            ("list", "recentchanges"),
            ("rcnamespace", YUME2KKI_NAMESPACE),
            ("rctype", "edit|new|log"),
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
    /// Everyone credited for the release, in the store's own order.
    pub authors: Vec<String>,
    /// ISO 8601. `None` for a release the store dates unreadably.
    pub released: Option<String>,
}

/// Only the fields the dump carries. The store also holds the colours a world's page is themed in
/// and the authors beyond the primary one, which [`locations`] does not request.
pub struct Location {
    /// The English page title, by which everything else refers to the world.
    pub title: String,
    /// Empty for a world the wiki has no headline picture for.
    pub location_image: String,
    /// The in-game name; absent where the wiki has not recorded one.
    pub original_name: Option<String>,
    /// The credited authors, comma-separated; absent where the wiki credits nobody.
    pub primary_author: Option<String>,
    pub bgms: Vec<Bgm>,
    pub location_maps: Vec<LocationMap>,
    /// As the version history names it. Empty for a world the wiki does not say the release of.
    pub version_added: String,
    /// Every release that changed the world, each optionally suffixed with the kind of change.
    pub versions_updated: Vec<String>,
    pub version_removed: Option<String>,
    /// Spans the world was absent for, each written `<removed>-<readded>`.
    pub version_gaps: Vec<String>,
}

pub struct Bgm {
    /// Where the wiki serves the file from. Empty for a track without a file on the wiki.
    pub path: String,
    /// Usually the track's in-game filename.
    pub title: Option<String>,
    /// Where in the world the track plays.
    pub label: Option<String>,
}

pub struct LocationMap {
    pub path: String,
    /// The wiki's caption, which says which part of the world the map covers.
    pub caption: String,
}

/// A property in an answer is a list of values under a name of the wiki's choosing, so what
/// arrives is one list per field rather than one record per connection.
pub struct Connection {
    /// Title of the origin world.
    pub origin: String,
    /// Title of the destination world.
    pub destination: String,
    /// The wiki's own vocabulary: `No Return`, `Locked`, `Chance` and so on.
    pub attributes: Vec<String>,
    /// The sentence a `Conditional` connection is gated behind.
    pub unlock_condition: Option<String>,
    pub effects_needed: Vec<String>,
    /// The wiki writes one value per season, so a connection open in three has three.
    pub seasons_available: Vec<String>,
    pub chance_percentage: Option<String>,
    pub is_removed: bool,
}

pub struct Author {
    /// As the English wiki writes the name.
    pub name: String,
    /// The author's own spelling, where that differs.
    pub original_name: Vec<String>,
}

#[derive(Deserialize)]
struct RecentChanges {
    query: Changes,
    /// Where the next page starts, absent on the last; nested under `continue`.
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

/// Every page of one query, paged by row offset; stops short of [`MAX_OFFSET`] with a warning.
/// Each row is paired with the subject holding the row: a page title or subobject name.
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
        let answer: Answer<R> = wiki_request(http)
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
        let next = answer.next;
        all.extend(answer.into_subject_rows());
        match next {
            // An offset that stops growing means the store wrapped.
            Some(next) if next > offset && next <= MAX_OFFSET => offset = next,
            Some(next) if next > MAX_OFFSET => {
                tracing::warn!("{conditions} is too large to read past row {next}");
                return Ok(all);
            }
            _ => return Ok(all),
        }
    }
}

fn wiki_request(http: &reqwest::Client) -> reqwest::RequestBuilder {
    http.get(WIKI).header(reqwest::header::ORIGIN, ORIGIN)
}

#[derive(Deserialize)]
struct Answer<R> {
    query: Found<R>,
    #[serde(rename = "query-continue-offset")]
    next: Option<u32>,
}

impl<R> Answer<R> {
    /// Rows paired with the page or subobject holding each.
    fn into_subject_rows(self) -> impl Iterator<Item = (String, R)> {
        self.query.results.into_iter().flat_map(|subject| {
            subject
                .into_iter()
                .map(|(name, found)| (name, found.printouts))
        })
    }
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

/// Strips [`PREFIX`]; the rest of dreamweaver names worlds without the prefix.
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

/// A monolingual text value; only the text is read.
#[derive(Deserialize)]
struct Monolingual {
    #[serde(rename = "Text", default, deserialize_with = "first_in_cell")]
    text: Option<String>,
}

/// A store date: a unix timestamp, plus a store-specific `raw` field.
#[derive(Deserialize)]
struct Date {
    timestamp: String,
}

impl Date {
    /// In the dump's own ISO 8601 form. `None` for an unparseable timestamp: the reader shows the
    /// release undated.
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
    dated: Option<DateRecord>,
}

/// The store files a release date as a record; only the `Full date` is read.
#[derive(Deserialize)]
struct DateRecord {
    #[serde(rename = "Full date", default, deserialize_with = "first_in_cell")]
    full_date: Option<Date>,
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
            // Readers group worlds by this string; five worlds are credited to two people.
            primary_author: (!self.authors.is_empty()).then(|| self.authors.join(", ")),
            bgms: self.bgms.into_iter().map(BgmRow::bgm).collect(),
            location_maps: self.maps.into_iter().map(MapRow::map).collect(),
            version_added: self.added.unwrap_or_default(),
            versions_updated: self.updated,
            version_removed: self.removed,
            version_gaps: self.gaps,
        }
    }
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
            // The reader refuses a null.
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
    /// The resolved file address; the `File:` page the store also names would need a request
    /// each.
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
            // The store's truth value, not JSON's.
            is_removed: self.is_removed.is_some_and(|flag| flag == "t"),
        })
    }
}

#[cfg(test)]
mod tests {
    /// A misread field silently drops a connection from the dump; pinned to the wiki's own bytes.
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
            .into_subject_rows()
            .next()
            .and_then(|(_, row)| row.connection())
            .expect("the one connection");
        assert_eq!(connection.origin, "Snow Village");
        assert_eq!(connection.destination, "Ice Cave");
        assert_eq!(connection.attributes, ["Seasonal", "Chance"]);
        // `crate::model` picks which season to publish.
        assert_eq!(connection.seasons_available, ["Fall", "Summer", "Winter"]);
        assert_eq!(connection.chance_percentage.as_deref(), Some("10%"));
        assert!(
            !connection.is_removed,
            "the store writes its own truth values"
        );
    }

    /// A debug build refuses a second value in a one-value field, so the model gets widened; a
    /// release build takes the first.
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
    /// writes a subobject's fields under `item` beside a description of the property.
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
        let (subject, row) = answer.into_subject_rows().next().expect("the one world");
        let world = row.location(super::without_namespace(&subject));
        assert_eq!(world.title, "3D Structures Path");
        assert_eq!(world.version_added, "0.116a");
        assert_eq!(world.versions_updated, ["0.122g", "0.124f patch 2"]);
        assert_eq!(world.version_removed, None);
        assert_eq!(world.version_gaps, ["0.120d patch 21-0.120e"]);
        let bgm = world.bgms.first().expect("the one track");
        assert_eq!(bgm.title.as_deref(), Some("46202"));
        assert_eq!(bgm.label.as_deref(), Some("Aooh's Trap"));
        assert_eq!(bgm.path, "");
        let map = world.location_maps.first().expect("the one map");
        assert_eq!(map.caption, "Map of 3D Structures Path");
        assert!(map.path.ends_with("3D_Structures_Path_map.png"));
    }

    /// Readers group worlds by one author string; co-authors are comma-joined, as in the wrapper.
    #[test]
    fn a_world_credited_to_two_people_is_credited_to_both() {
        let row = |authors: &[&str]| super::LocationRow {
            image: None,
            authors: authors.iter().map(|name| (*name).to_owned()).collect(),
            japanese: None,
            bgms: Vec::new(),
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
        assert_eq!(
            row(&[]).location("FC Caverns".to_owned()).primary_author,
            None
        );
    }

    /// The store writes a monolingual name as an object.
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
        let (_, row) = answer.into_subject_rows().next().expect("the one author");
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

    #[test]
    fn a_release_date_is_read_out_of_the_record_the_store_files_it_in() {
        let answer = r#"{"query":{"results":[{"Yume 2kki:Version History/0130-0126# d925":{
            "printouts":{"Version":["0.129e patch 4"],"Has contributing author":["Nabisae"],
            "Version/Date":[{
                "Full date":{"label":"Full date","key":"Full_date","typeid":"_dat",
                    "item":[{"timestamp":"1792540800","raw":"1/2026/10/21"}]},
                "Year":{"label":"Year","key":"Year","typeid":"_num","item":[2026]}}]}}}]}}"#;
        let answer: super::Answer<super::VersionRow> =
            serde_json::from_str(answer).expect("the store's own answer");
        let (_, row) = answer.into_subject_rows().next().expect("the one release");
        let released = row
            .dated
            .and_then(|record| record.full_date)
            .and_then(|date| date.released());
        assert_eq!(released.as_deref(), Some("2026-10-21T00:00:00.000Z"));
    }

    /// Release dates share the dump's stamp format, so a reader parses both alike.
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

    /// The continuation is nested under `continue`; a misread would stop after the first page of
    /// edits.
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
        assert_eq!(titles, ["Snow Village", "Authors"]);
        assert!(busy.carry.and_then(|carry| carry.rccontinue).is_some());
    }
}
