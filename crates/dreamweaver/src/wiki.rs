//! Where a dump comes from: yume.wiki, and the shapes it is read back in.
//!
//! Most of it is the wiki's own Semantic MediaWiki store, asked directly -- see [`crate::smw`] for
//! the queries and for why they are not [ynoproject/wikiwrapper]'s any more. What is left for the
//! wrapper is [`Client::images`]: a world's gallery is page content rather than a property, so the
//! store has nothing to answer with and the wrapper has already done the reading.
//!
//! The wiki's own `api.php` is behind an edge that answers a plain request with a challenge page,
//! which is what [`ORIGIN`] is for. Two questions go to it rather than to the store, and neither
//! reads content: [`Client::changed_since`] asks which pages have been touched since a moment,
//! which is what turns a scheduled rebuild into a smaller one or into nothing at all, and it is a
//! question about the wiki rather than about what the wiki says.
//!
//! Nothing here parses wiki markup or HTML, and nothing here should. Every field in the dump is
//! something the wiki holds as structured data; a field that would have to be read out of prose is
//! left out instead -- see `main`.
//!
//! [ynoproject/wikiwrapper]: https://github.com/ynoproject/wikiwrapper

use serde::Deserialize;

/// Where the public instance answers, which is what [`Client::new`] talks to.
const WRAPPER: &str = "https://wrapper.yume.wiki";

/// The only game this workspace asks about, and the code the service knows it by.
///
/// Every endpoint takes a `game`, and passing one from a caller that only ever has the one value
/// would be a parameter that exists to be ignored.
const GAME: &str = "2kki";

/// The wiki's own MediaWiki API: what [`Client::changed_since`] asks, and what [`crate::smw`]
/// puts its queries to.
pub const WIKI: &str = "https://yume.wiki/api.php";

/// Yume 2kki's namespace on a wiki that houses a dozen games under one installation. Every page a
/// dump is built out of is in it, and nothing a dump cares about is outside it.
const NAMESPACE: &str = "3002";

/// What the wiki's edge wants to see before it answers an API request rather than serving a
/// challenge page. It is the explorer this program stands in for, which is exactly what a request
/// from here is on behalf of.
pub const ORIGIN: &str = "https://explorer.yume.wiki";

/// Everything that can go wrong here is an HTTP request or the JSON it came back as, both of
/// which `reqwest` already names.
pub type Error = reqwest::Error;
pub type Result<T> = std::result::Result<T, Error>;

/// A connection to the wiki, and to the one wrapper endpoint still in use.
///
/// Cheap to clone -- the pool underneath is shared -- and worth reusing, since a full sync is a
/// few dozen requests to the same two hosts.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
}

impl Client {
    /// A client pointed at the public instance.
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// Every location the wiki documents, with the fields a world page carries in its infobox.
    pub async fn locations(&self) -> Result<Vec<Location>> {
        crate::smw::locations(&self.http).await
    }

    /// The passages out of every world whose title starts with one of `initials`, kept in those
    /// pieces.
    ///
    /// A connection is listed by the world it leads out of, so the same pair usually appears
    /// twice with different attributes -- which is exactly how a one-way passage is told from a
    /// two-way one.
    ///
    /// The wrapper has an endpoint for these and this does not use it: there are more passages
    /// than the store will page through in one query, and the endpoint hands back the first five
    /// and a half thousand of them for ever. See [`crate::smw::connections`] for the pieces the
    /// question is cut into, and [`crate::sync::Fetched`] for who asks for which of them.
    pub async fn connections(
        &self,
        initials: impl Iterator<Item = char>,
    ) -> Result<std::collections::BTreeMap<char, Vec<Connection>>> {
        crate::smw::connections(&self.http, initials).await
    }

    /// Every release the wiki dates, newest first.
    ///
    /// Straight from the store: the wrapper publishes no version history, and a dump without one
    /// can name the release a world arrived in but not say when that was.
    pub async fn versions(&self) -> Result<Vec<crate::smw::Version>> {
        crate::smw::versions(&self.http).await
    }

    /// Every picture each location's page shows, in page order.
    pub async fn images(&self) -> Result<Vec<LocationImages>> {
        self.paged("images", |page: Images| {
            (page.location_images, page.continue_key)
        })
        .await
    }

    /// Everyone the wiki credits, in both the names it credits them under.
    pub async fn authors(&self) -> Result<Vec<Author>> {
        crate::smw::authors(&self.http).await
    }

    /// Every page in Yume 2kki's namespace touched since `when`, an ISO 8601 instant.
    ///
    /// This is what a soft sync steers by. Rebuilding a dump means asking the store for sixteen
    /// hundred worlds and every passage between them, and on most days none of it has moved; the
    /// answer here says whether anything has, and if so which pages, which is enough to re-ask
    /// only the parts of the store that could have changed. See [`crate::sync::Fetched`].
    ///
    /// Edits, new pages and log entries all count, so a world being deleted or renamed reads as a
    /// change the same as one being written. Only Yume 2kki's namespace is looked at: the wiki is
    /// busy with a dozen other games whose edits mean nothing to this dump. That is also the hole
    /// in it -- a template or a file the worlds are built out of lives elsewhere, and an edit to
    /// one changes what the store answers without any page here being touched. A full sync is the
    /// backstop for that, which is why a run does one when it comes up.
    ///
    /// Titles come back without the namespace on the front, spelled as the rest of the program
    /// spells a world, and each of them once however many times it was edited.
    pub async fn changed_since(&self, when: &str) -> Result<Vec<String>> {
        let mut titles = std::collections::BTreeSet::new();
        let mut carry: Option<String> = None;
        loop {
            let mut request = self
                .http
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
                    .map(|change| crate::smw::without_namespace(&change.title)),
            );
            match changes.carry.and_then(|carry| carry.rccontinue) {
                Some(next) => carry = Some(next),
                None => return Ok(titles.into_iter().collect()),
            }
        }
    }

    /// One request, with the game code and whatever else the endpoint wants.
    async fn get<T: serde::de::DeserializeOwned>(
        &self,
        endpoint: &str,
        query: &[(&str, &str)],
    ) -> Result<T> {
        self.http
            .get(format!("{WRAPPER}/{endpoint}"))
            .query(&[("game", GAME)])
            .query(query)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }

    /// Walks an endpoint that answers a page at a time, following `continueKey` to the end.
    ///
    /// `split` says how to take a page apart, since each endpoint names its list after itself.
    /// The pages are gathered rather than streamed: a full sync wants all of it before it can
    /// resolve one world's connections against another's, and the whole of the largest endpoint
    /// is a few megabytes.
    ///
    /// A key that has already been followed ends the walk, which is not belt and braces: the
    /// cursor is the store's row offset, and past a few thousand rows the store answers the first
    /// page again with the count carrying on regardless, so a client that trusts `continueKey`
    /// alone paces round the same rows for ever. Stopping on the repeat costs one wasted page and
    /// is the difference between a sync that finishes and one that does not. It is also why the
    /// largest of these lists is not fetched here at all -- see [`Client::connections`].
    async fn paged<P, T>(
        &self,
        endpoint: &str,
        split: impl Fn(P) -> (Vec<T>, Option<String>),
    ) -> Result<Vec<T>>
    where
        P: serde::de::DeserializeOwned,
    {
        let mut all = Vec::new();
        let mut followed = std::collections::HashSet::new();
        let mut cursor: Option<String> = None;
        loop {
            let query: &[(&str, &str)] = match &cursor {
                Some(key) => &[("continueKey", key)],
                None => &[],
            };
            let (page, next) = split(self.get(endpoint, query).await?);
            all.extend(page);
            match next {
                Some(next) if followed.insert(next.clone()) => cursor = Some(next),
                _ => return Ok(all),
            }
        }
    }
}

/// One page of the wiki's answer to [`Client::changed_since`].
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

/// One change, of which only the page it happened to is read.
#[derive(Deserialize)]
struct Change {
    title: String,
}

/// A world, as its wiki page describes it.
///
/// Only the fields the dump carries are named. The store also holds the colours a world's wiki
/// page is themed in, the RPG Maker map numbers it is built out of and the authors beyond the
/// primary one; [`crate::smw::locations`] does not ask for what nothing reads.
pub struct Location {
    /// The English page title, which is the identity everything else refers to it by.
    pub title: String,
    /// Where the wiki serves the world's headline picture from. Empty for a world with none.
    pub location_image: String,
    /// As the game itself names it. Absent for the worlds the wiki has not recorded one for.
    pub original_name: Option<String>,
    /// Who the wiki credits the world to, comma-separated where that is more than one person.
    /// Absent where it credits nobody.
    pub primary_author: Option<String>,
    pub bgms: Vec<Bgm>,
    pub location_maps: Vec<LocationMap>,
    /// The release the world first appeared in, as the version history names it. Empty for a page
    /// in the Locations category that carries no infobox at all.
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
    /// The track's own name, which is usually its filename in the game.
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

/// A passage out of one world, as the world it leaves from writes it up.
///
/// Built by hand in [`crate::smw`], like everything else the store answers: a property in an
/// answer is a list of values under a name of the wiki's choosing, so the shape that comes off
/// the wire is not the shape anything wants to read.
pub struct Connection {
    /// Title of the world it leads out of.
    pub origin: String,
    /// Title of the world it leads to.
    pub destination: String,
    /// What the passage is like, in the wiki's own vocabulary: `No Return`, `Locked`, `Chance`
    /// and so on. See the caller for what each one means.
    pub attributes: Vec<String>,
    /// The sentence a `Conditional` passage is gated behind.
    pub unlock_condition: Option<String>,
    /// The effects a `Needs Effect` passage wants the player to be wearing.
    pub effects_needed: Vec<String>,
    /// Which season a `Seasonal` passage is open in.
    pub season_available: Option<String>,
    /// The odds a `Chance` passage opens at, as the wiki writes them.
    pub chance_percentage: Option<String>,
    /// A passage that used to exist and no longer does.
    pub is_removed: bool,
}

/// One page of [`Client::images`].
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Images {
    location_images: Vec<LocationImages>,
    continue_key: Option<String>,
}

/// Every picture on one world's page.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationImages {
    /// The world's title, as [`Location::title`] gives it.
    pub title: String,
    pub images: Vec<Image>,
}

/// One picture, at the size the wiki serves it in a gallery.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub url: String,
}

/// Someone the wiki credits.
pub struct Author {
    /// As the English wiki writes the name.
    pub name: String,
    /// As the author writes it themselves, where that differs.
    pub original_name: Option<String>,
}

#[cfg(test)]
mod tests {
    /// What a soft sync reads and how far it reads: the pages come out of this list, and the
    /// continuation is nested under `continue` rather than sitting beside the results, so a
    /// misread there would stop the walk one page in and leave the rest of a week's editing
    /// unnoticed. Both answers, as the wiki actually writes them.
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
            .map(|change| crate::smw::without_namespace(&change.title))
            .collect();
        // Without the namespace, which is how the rest of the program spells a world -- and how a
        // sync recognises the two pages that are not worlds at all.
        assert_eq!(titles, ["Snow Village", "Authors"]);
        assert!(busy.carry.and_then(|carry| carry.rccontinue).is_some());
    }
}
