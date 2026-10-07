//! Building a dump out of what the wiki knows.
//!
//! Four fetches describe four parts of the same thing -- worlds, connections, credits, releases --
//! and none knows about the others. Here they are joined into one graph, measured and written out.
//!
//! A rebuild does not have to fetch all four: [`Fetched`] keeps the last answers, and a sync that
//! knows which pages have been edited re-reads only the parts those pages could have changed.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::depth;
use crate::model::{self, ConnType, Connection, Dump, TypeParams, World};
use crate::progress::{self, Progress};
use crate::smw;
use crate::versions;

/// What the last sync fetched, kept so the next one can leave most of it alone.
///
/// Every fetch here is a question about pages, so an account of which pages have been edited says
/// which answers can still be trusted: two or three requests a week instead of thirty.
///
/// The worlds themselves are not kept: they are one query for all sixteen hundred, and a world that
/// changed can add or remove one, which would make this something to reconcile rather than skip.
///
/// No invalidation of its own, which is safe only because it is never read except beside a
/// [`RefreshScope`] saying what has moved, and a run that has just come up has an empty one.
#[derive(Default)]
pub struct Fetched {
    authors: Vec<smw::Author>,
    releases: Vec<smw::Version>,
    /// Keyed by the first character of the world the connection leaves. See
    /// [`crate::smw::connections`].
    connections: BTreeMap<char, Vec<smw::Connection>>,
}

pub enum RefreshScope {
    /// Whatever the wiki says about itself, without querying what has changed.
    Everything,
    /// Titles without the `Yume 2kki:` namespace prefix. An empty list never reaches here: the
    /// caller stands the sync down instead.
    Pages(Vec<String>),
}

impl RefreshScope {
    fn touches_page(&self, page: &str) -> bool {
        match self {
            RefreshScope::Everything => true,
            RefreshScope::Pages(pages) => pages.iter().any(|edited| edited == page),
        }
    }

    /// The version history is written across a dozen subpages of one title.
    fn touches_under(&self, prefix: &str) -> bool {
        match self {
            RefreshScope::Everything => true,
            RefreshScope::Pages(pages) => pages.iter().any(|edited| edited.starts_with(prefix)),
        }
    }

    /// A connection is written up on the page of the world it leaves, so an edited page can only
    /// have changed the piece its own title falls in.
    fn shards(&self, all: &BTreeSet<char>) -> BTreeSet<char> {
        match self {
            RefreshScope::Everything => all.clone(),
            RefreshScope::Pages(pages) => pages
                .iter()
                .filter_map(|edited| edited.chars().next())
                .filter(|initial| all.contains(initial))
                .collect(),
        }
    }
}

/// The wiki's author list, and the title its version history is written under. An edit to either
/// is what makes those answers stale.
const AUTHORS: &str = "Authors";
const VERSION_HISTORY: &str = "Version History";

/// `previous` is the last dump published, consulted only for what an operator has marked on the
/// worlds and when the dump was last rebuilt without querying.
pub async fn run(
    http: &reqwest::Client,
    previous: &Dump,
    refresh: RefreshScope,
    fetched: &mut Fetched,
    progress: &Progress,
) -> smw::Result<Dump> {
    // First: the worlds decide which connection shards to request.
    progress.at(progress::WORLDS);
    let locations = smw::locations(http).await?;
    let initials: BTreeSet<char> = locations
        .iter()
        .filter_map(|location| location.title.chars().next())
        .collect();
    // An empty cache requests everything.
    let cold = fetched.connections.is_empty();
    let want_authors = cold || refresh.touches_page(AUTHORS);
    let want_releases = cold || refresh.touches_under(VERSION_HISTORY);
    let shards = match cold {
        true => initials.clone(),
        false => refresh.shards(&initials),
    };

    let shards_count = shards.len();
    progress.at(progress::CONNECTIONS);
    let (authors, releases, connections) = tokio::try_join!(
        optional(want_authors, smw::authors(http)),
        optional(want_releases, smw::versions(http)),
        smw::connections(http, shards),
    )?;

    if let Some(authors) = authors {
        fetched.authors = authors;
    }
    if let Some(releases) = releases {
        fetched.releases = releases;
    }
    fetched.connections.extend(connections);
    // Otherwise a deleted world stays reachable.
    fetched
        .connections
        .retain(|initial, _| initials.contains(initial));

    /// Which parts were read this time, so a log line reads as what this sync cost rather
    /// than what it ended up holding.
    fn again(read: bool) -> &'static str {
        match read {
            true => "reread",
            false => "kept",
        }
    }
    tracing::info!(
        "read {} worlds and {shards_count} of {} connection groups; {} authors {}, {} releases {}",
        locations.len(),
        initials.len(),
        fetched.authors.len(),
        again(want_authors),
        fetched.releases.len(),
        again(want_releases),
    );
    Ok(assemble(
        locations,
        fetched.connections.values().flatten(),
        &fetched.authors,
        &fetched.releases,
        previous,
        matches!(refresh, RefreshScope::Everything),
    ))
}

/// Awaits `work` only if it is wanted, so several conditional fetches can still run as one.
async fn optional<T>(
    wanted: bool,
    work: impl std::future::Future<Output = smw::Result<T>>,
) -> smw::Result<Option<T>> {
    match wanted {
        true => Ok(Some(work.await?)),
        false => Ok(None),
    }
}

/// Split out from [`run`] so it can be exercised without a network.
fn assemble<'a>(
    locations: Vec<smw::Location>,
    connections: impl Iterator<Item = &'a smw::Connection>,
    authors: &[smw::Author],
    releases: &[smw::Version],
    previous: &Dump,
    full: bool,
) -> Dump {
    let locations = published_worlds(locations);
    let at: HashMap<&str, usize> = locations
        .iter()
        .enumerate()
        .map(|(at, location)| (location.title.as_str(), at))
        .collect();

    // Per pair: the wiki writes a connection once per direction, sometimes twice.
    let mut merged: HashMap<(usize, usize), Attributes> = HashMap::new();
    let mut leaving: Vec<Vec<usize>> = vec![Vec::new(); locations.len()];
    for connection in connections {
        if connection.is_removed {
            continue;
        }
        let (Some(&from), Some(&to)) = (
            at.get(connection.origin.as_str()),
            at.get(connection.destination.as_str()),
        ) else {
            // A hole in the wiki: the end is not a location page.
            continue;
        };
        let merged = merged.entry((from, to)).or_insert_with(|| {
            leaving[from].push(to);
            Attributes::default()
        });
        for attribute in &connection.attributes {
            let Some((flag, wording)) = model::conn_type(attribute, connection) else {
                tracing::debug!("unknown connection attribute {attribute:?}");
                continue;
            };
            merged.flags |= flag;
            if let Some((params, params_jp)) = wording.published() {
                merged.wording_by_flag.insert(
                    flag.bits(),
                    TypeParams {
                        params: Some(params),
                        params_jp,
                    },
                );
            }
        }
    }

    // Sorted: fetch order varies, and two syncs of one wiki must match.
    for leaving in &mut leaving {
        leaving.sort_unstable();
    }

    let removed: Vec<bool> = locations
        .iter()
        .map(|location| location.version_removed.is_some())
        .collect();
    let distances = depth::of(
        &locations
            .iter()
            .enumerate()
            .map(|(at, location)| depth::Node {
                title: location.title.clone(),
                removed: removed[at],
                out: leaving[at]
                    .iter()
                    .map(|&to| (to, merged[&(at, to)].flags))
                    .collect(),
            })
            .collect::<Vec<_>>(),
    );

    // Removed worlds are measured, since a live world can sit behind one, but not published.
    let published: Vec<Option<usize>> = {
        let mut next = 0;
        removed
            .iter()
            .map(|&removed| {
                (!removed).then(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect()
    };
    let secret = marked_secret(previous);
    let live: Vec<&str> = locations
        .iter()
        .enumerate()
        .filter(|(at, _)| published[*at].is_some())
        .map(|(_, location)| location.title.as_str())
        .collect();
    let cells = cells(previous, &live, full);
    let worlds = locations
        .iter()
        .enumerate()
        .filter(|(at, _)| published[*at].is_some())
        .map(|(at, location)| {
            let connections: Vec<Connection> = leaving[at]
                .iter()
                .filter_map(|&to| {
                    let attributes = &merged[&(at, to)];
                    Some(Connection {
                        target_id: published[to]?,
                        bits: attributes.flags.bits(),
                        type_params: attributes.wording_by_flag.clone(),
                    })
                })
                .collect();

            World {
                id: published[at].expect("filtered to published worlds"),
                cell: cells.get(location.title.as_str()).copied(),
                title: location.title.clone(),
                title_jp: text(location.original_name.as_deref()),
                author: location.primary_author.clone().unwrap_or_default(),
                depth: distances[at].0,
                min_depth: distances[at].1,
                filename: encode_uri(&location.location_image),
                map_url: joined(location.location_maps.iter().map(|map| map.path.clone())),
                map_label: joined(location.location_maps.iter().map(|map| map.caption.clone())),
                bgm_url: joined(location.bgms.iter().map(|bgm| bgm.path.clone())),
                // A track with neither still holds its place in the list.
                bgm_label: joined(location.bgms.iter().map(|bgm| {
                    format!(
                        "{}^{}",
                        bgm.title.as_deref().unwrap_or_default(),
                        bgm.label.as_deref().unwrap_or_default()
                    )
                })),
                ver_added: text(Some(&location.version_added)),
                ver_removed: location.version_removed.clone(),
                ver_updated: versions::updates(&location.versions_updated.join(",")),
                ver_gaps: versions::gaps(&location.version_gaps.join(",")),
                removed: false,
                secret: secret.contains(location.title.as_str()),
                connections,
            }
        })
        .collect();

    let (last_update, last_full_update) = stamps(previous, full);
    Dump {
        worlds,
        authors: authors
            .iter()
            .map(|author| crate::model::Author {
                name: author.name.clone(),
                // TODO: do we want multiple names here?
                name_jp: author.original_name.first().cloned(),
            })
            .collect(),
        versions: releases
            .iter()
            .map(|release| crate::model::Version {
                name: release.name.clone(),
                // Unread yet, but published as the reference dump writes it.
                authors: (!release.authors.is_empty()).then(|| release.authors.join(", ")),
                release_date: release.released.clone(),
            })
            .collect(),
        effects: Vec::new(),
        menu_themes: Vec::new(),
        wallpapers: Vec::new(),
        bgm_tracks: Vec::new(),
        last_update,
        last_full_update,
        is_admin: false,
    }
}

/// Gathered from however many rows describe one connection.
#[derive(Default)]
struct Attributes {
    flags: ConnType,
    wording_by_flag: std::collections::BTreeMap<u16, TypeParams>,
}

/// The worlds this dump is about, in [`published_place`] order.
///
/// Secrets included: the mark is carried from one dump to the next by title, so a dropped world is
/// a mark forgotten at the next sync, and hiding is a question about a reader rather than the game.
///
/// A world's published id is its index here, and the order is the wiki's rather than this program's
/// history, so two runs reading the same wiki publish the same ids.
fn published_worlds(mut locations: Vec<smw::Location>) -> Vec<smw::Location> {
    locations.sort_by(|one, other| published_place(one).cmp(&published_place(other)));
    locations
}

/// The origin first, then every other world by title.
///
/// Nothing reads a world's position: the app and [`crate::depth`] both find the origin by title,
/// and a `targetId` only has to agree with the dump it was published in. The order used to be the
/// game's map numbering, to keep the thumbnail atlas from shifting under an insertion -- a job the
/// world's own `cell` now does. What is left is a dump a person can read.
///
/// The origin is named anyway. It costs a comparison, and a dump opening on `3D Structures Path`
/// rather than the room the player wakes up in reads as a mistake.
fn published_place(location: &smw::Location) -> (bool, &str) {
    (location.title != depth::START, &location.title)
}

/// Every published world's cell in the thumbnail atlas, by title.
///
/// A cell is handed out once and never moves: a world keeps whatever the last dump gave it, and one
/// first seen now takes a cell above every cell in use. Not packed by [`World::id`], which is a
/// place in [`published_place`] order and moves under an already-published world, leaving every
/// picture after the newcomer belonging to somebody else.
///
/// So an atlas is still right about every world it was packed with however far the dump has moved
/// on, and a world it has no cell for draws the placeholder until the atlas is packed again.
///
/// Cells are not reused below the highest one held: handing a dropped world's gap to the next world
/// along is the same wrong picture in miniature. A `full` run reclaims the tail, held to the weekly
/// pass so a world the wiki marks removed and then restores keeps its picture.
///
/// Title is the key, as it is for [`marked_secret`], and carries the same cost: a world the wiki
/// renames takes a fresh cell. A cold build hands out cells in publish order, so the atlas has to
/// be packed again after one.
fn cells<'a>(previous: &Dump, live: &[&'a str], full: bool) -> HashMap<&'a str, usize> {
    let mut held: HashMap<&str, usize> = previous
        .worlds
        .iter()
        .filter_map(|world| Some((world.title.as_str(), world.cell?)))
        .collect();
    if full {
        let still: std::collections::HashSet<&str> = live.iter().copied().collect();
        held.retain(|title, _| still.contains(*title));
    }
    let mut next = held.values().map(|cell| cell + 1).max().unwrap_or(0);
    live.iter()
        .map(|&title| {
            let cell = held.get(title).copied().unwrap_or_else(|| {
                next += 1;
                next - 1
            });
            (title, cell)
        })
        .collect()
}

/// Marked by an operator as a spoiler, which no sync should unmark.
fn marked_secret(previous: &Dump) -> std::collections::HashSet<&str> {
    previous
        .worlds
        .iter()
        .filter(|world| world.secret)
        .map(|world| world.title.as_str())
        .collect()
}

/// `None` for an absent *or* empty field: the reader treats an empty string as a value it has, so
/// an empty Japanese title would be shown as a world's name.
fn text(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}

/// Blank entries are kept: the reader reads two of these fields in step, so a gap in one has to be
/// a gap in the other.
fn joined(parts: impl Iterator<Item = String>) -> Option<String> {
    let parts: Vec<String> = parts.collect();
    (!parts.is_empty()).then(|| parts.join("|"))
}

/// Percent-encodes an address the way a browser's `encodeURI` does.
///
/// The wiki serves pictures under the page titles they were uploaded for, so an address can carry a
/// space or a Japanese character verbatim, and readers hand these straight to an HTTP client.
fn encode_uri(url: &str) -> String {
    /// What `encodeURI` leaves alone: the unreserved set, the reserved delimiters, and `#`.
    const KEEP: &str = ";,/?:@&=+$-_.!~*'()#";
    let mut encoded = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.as_bytes().contains(&byte) {
            encoded.push(byte as char);
        } else if byte == b'%' {
            // Unlike `encodeURI`, which would turn `%27` into `%2527`; the wiki serves both.
            encoded.push('%');
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// When the dump was built, and when it was last built without checking whether it needed to be.
///
/// A soft sync publishes a dump as complete as any other, but by trusting the wiki's account of
/// itself, which no soft sync could catch out. So `lastFullUpdate` marks the last time this program
/// saw the whole wiki, and a soft sync carries it over rather than moving it.
fn stamps(previous: &Dump, full: bool) -> (Option<String>, Option<String>) {
    let now = stamp();
    let last_full = match full {
        true => None,
        false => previous.last_full_update.clone(),
    };
    (Some(now.clone()), Some(last_full.unwrap_or(now)))
}

/// How far back the wiki's record of its own recent changes is worth trusting.
///
/// MediaWiki keeps that record for a fixed span and then forgets, answering about a moment further
/// back with what it still has rather than with a complaint -- so an older dump would read that as
/// "nothing has changed" and stay stale for ever. The default is ninety days; a month leaves room
/// for it to be configured tighter.
const HORIZON: time::Duration = time::Duration::days(30);

/// How far before the last dump a soft sync starts looking.
///
/// The store is not written by the edit that changes it: a job queue re-reads the page afterwards,
/// and until it has, a query answers with what the page used to say. A sync querying only what
/// changed since it last ran would take that answer and never ask again.
const MARGIN: time::Duration = time::Duration::hours(1);

/// The moment a soft sync queries the wiki from, or `None` for a dump too old for the question to
/// mean anything, which is a full sync's job.
pub fn looking_from(since: &str, now: time::OffsetDateTime) -> Option<String> {
    let since = parse_iso(since)?;
    (now - since < HORIZON).then(|| iso(since - MARGIN))
}

pub fn parse_iso(stamp: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).ok()
}

/// In the format the reference implementation's dump stamps itself with.
fn stamp() -> String {
    iso(time::OffsetDateTime::now_utc())
}

/// Also what a release is dated with: two conventions would be two things for one reader to know.
pub fn iso(now: time::OffsetDateTime) -> String {
    // The milliseconds are always zero.
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.000Z",
        now.year(),
        now.month() as u8,
        now.day(),
        now.hour(),
        now.minute(),
        now.second()
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_picture_address_is_escaped_the_way_a_browser_would() {
        assert_eq!(
            super::encode_uri("https://yume.wiki/images/a/Uro Room.png"),
            "https://yume.wiki/images/a/Uro%20Room.png"
        );
        assert_eq!(
            super::encode_uri("https://yume.wiki/images/a/夢.png"),
            "https://yume.wiki/images/a/%E5%A4%A2.png"
        );
    }

    #[test]
    fn the_origin_opens_the_dump_and_the_rest_follow_by_title() {
        let world = |title: &str| crate::smw::Location {
            title: title.to_owned(),
            location_image: String::new(),
            original_name: None,
            primary_author: None,
            bgms: Vec::new(),
            location_maps: Vec::new(),
            version_added: String::new(),
            versions_updated: Vec::new(),
            version_removed: None,
            version_gaps: Vec::new(),
        };
        let published = super::published_worlds(vec![
            world("Nexus"),
            world("Chocolate World"),
            world("Urotsuki's Room"),
            world("Debug Room"),
            world("FC Caverns"),
        ]);
        assert_eq!(
            published
                .iter()
                .map(|location| location.title.as_str())
                .collect::<Vec<_>>(),
            [
                "Urotsuki's Room",
                "Chocolate World",
                "Debug Room",
                "FC Caverns",
                "Nexus",
            ]
        );
    }

    /// A dump holding just what [`super::cells`] reads: the titles published last time and the
    /// cells they were given. `None` is a dump written before cells existed.
    fn previously(worlds: &[(&str, Option<usize>)]) -> crate::model::Dump {
        serde_json::from_value(serde_json::json!({
            "worldData": worlds
                .iter()
                .map(|(title, cell)| serde_json::json!({
                    "id": 0,
                    "cell": cell,
                    "title": title,
                    "titleJP": null,
                    "author": "",
                    "depth": 0,
                    "minDepth": 0,
                    "filename": "",
                    "mapUrl": null,
                    "mapLabel": null,
                    "bgmUrl": null,
                    "bgmLabel": null,
                    "verAdded": null,
                    "verRemoved": null,
                    "verUpdated": null,
                    "verGaps": null,
                    "removed": false,
                    "secret": false,
                    "connections": [],
                }))
                .collect::<Vec<_>>(),
            "authorInfoData": [],
            "versionInfoData": [],
            "effectData": [],
            "menuThemeData": [],
            "wallpaperData": [],
            "bgmTrackData": [],
            "lastUpdate": null,
            "lastFullUpdate": null,
            "isAdmin": false,
        }))
        .expect("the fixture is a dump")
    }

    #[test]
    fn a_world_keeps_the_cell_the_last_dump_gave_it() {
        // Cell 5's world was dropped; the cell is not reassigned.
        let previous = previously(&[("Nexus", Some(1)), ("Dropped", Some(5)), ("Hub", Some(0))]);
        let cells = super::cells(&previous, &["New", "Hub", "Nexus", "Newer"], false);
        assert_eq!(cells["Hub"], 0);
        assert_eq!(cells["Nexus"], 1);
        assert_eq!(cells["New"], 6);
        assert_eq!(cells["Newer"], 7);
    }

    #[test]
    fn a_dump_with_no_cells_to_carry_hands_out_the_publish_order() {
        for previous in [
            previously(&[]),
            previously(&[("Hub", None), ("Nexus", None)]),
        ] {
            let cells = super::cells(&previous, &["Hub", "Nexus"], false);
            assert_eq!(cells["Hub"], 0);
            assert_eq!(cells["Nexus"], 1);
        }
    }

    // `lastFullUpdate` tells a dump this program verified from one taken at the wiki's word.
    #[test]
    fn only_a_full_sync_moves_the_stamp_that_says_so() {
        let mut previous = crate::model::Dump {
            last_full_update: Some("2026-01-01T00:00:00.000Z".to_owned()),
            ..Default::default()
        };
        let (soft, kept) = super::stamps(&previous, false);
        assert_eq!(kept, previous.last_full_update);
        assert!(soft.is_some());

        let (_, moved) = super::stamps(&previous, true);
        assert_ne!(moved, previous.last_full_update);

        previous.last_full_update = None;
        let (built, full) = super::stamps(&previous, false);
        assert_eq!(built, full);
    }

    // A spurious request costs one request; a missed one leaves the dump wrong until a full sync.
    #[test]
    fn only_the_pieces_an_edited_page_belongs_to_are_read_again() {
        let letters: std::collections::BTreeSet<char> = "ABS".chars().collect();
        let edited = super::RefreshScope::Pages(vec![
            "Snow Village".to_owned(),
            "Authors".to_owned(),
            "Version History/0089-0000".to_owned(),
        ]);
        assert!(edited.touches_page("Authors"), "the author list was edited");
        assert!(
            edited.touches_under("Version History"),
            "the history is written across subpages, so the prefix is what matches"
        );
        assert!(
            !edited.touches_page("Snow Village/Maps"),
            "and a name is not a prefix"
        );
        // `A` for the author page, which shares a letter with worlds.
        assert_eq!(edited.shards(&letters), "AS".chars().collect());

        let everything = super::RefreshScope::Everything;
        assert!(everything.touches_page("anything at all"));
        assert_eq!(everything.shards(&letters), letters);
    }

    #[test]
    fn a_full_run_reclaims_the_cells_above_the_worlds_it_still_publishes() {
        let previous = previously(&[("Hub", Some(0)), ("Gap", Some(2)), ("Dropped", Some(5))]);
        let full = super::cells(&previous, &["Hub", "Gap", "New"], true);
        assert_eq!(full["Hub"], 0, "a world still published keeps its cell");
        assert_eq!(full["Gap"], 2, "wherever the dropped world's cell sat");
        assert_eq!(
            full["New"], 3,
            "and the next one out is above the highest still held, not in the gap at 1"
        );
        assert_eq!(
            super::cells(&previous, &["Hub", "Gap", "New"], false)["New"],
            6,
            "where a soft sync holds the dropped world's cell and passes over it"
        );
    }

    #[test]
    fn the_wiki_is_queried_from_a_little_before_the_dump_was_built() {
        let now = time::OffsetDateTime::from_unix_timestamp(1_788_393_600).expect("a moment");
        let built = super::iso(now - time::Duration::hours(6));

        // Back an hour: the store indexes after the edit.
        assert_eq!(
            super::looking_from(&built, now).as_deref(),
            Some(super::iso(now - time::Duration::hours(7)).as_str())
        );

        // Past the wiki's memory, "nothing changed" is indistinguishable from "nothing known".
        let stale = super::iso(now - time::Duration::days(45));
        assert_eq!(
            super::looking_from(&stale, now),
            None,
            "read the whole wiki instead"
        );
        assert_eq!(
            super::looking_from("last tuesday", now),
            None,
            "as for a stamp with no moment in it"
        );
    }

    #[test]
    fn an_already_escaped_address_is_left_as_it_is() {
        let escaped = "https://yume.wiki/images/0/Urotsuki%27s_Room.png";
        assert_eq!(super::encode_uri(escaped), escaped);
    }
}
