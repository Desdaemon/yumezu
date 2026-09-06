//! Building a dump out of what the wiki knows.
//!
//! Four fetches come back describing four parts of the same thing -- worlds, the passages between
//! them, the people credited for them, the releases they arrived in -- and none of them knows
//! about the others. This is where they are joined into one graph, measured, and written out in
//! the shape the reader expects.
//!
//! A rebuild does not have to fetch all four. [`Fetched`] keeps the last answers, and a sync that
//! knows which pages have been edited re-asks only the parts of the wiki those pages could have
//! changed -- see [`Refresh`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::depth;
use crate::model::{ConnType, Connection, Dump, TypeParams, World};
use crate::progress::{self, Progress};
use crate::smw;
use crate::versions;

/// What the last sync fetched, kept so the next one can leave most of it alone.
///
/// Every fetch here is a question about pages: the authors are one page, the releases a handful,
/// and a passage belongs to the page that writes it up. So an
/// account of which pages have been edited is also an account of which of these answers can still
/// be trusted, and a soft sync spends its requests on the rest. On a wiki where a week's editing
/// touches a dozen worlds, that is two or three requests instead of thirty.
///
/// The worlds themselves are not kept: they are one query for all sixteen hundred, and a world
/// that changed can add or remove one, which would make a cache something to reconcile rather
/// than something to skip.
///
/// It is a cache with no invalidation of its own, which is safe only because it is never read
/// except beside a [`Refresh`] saying what has moved, and because a run that has just come up has
/// an empty one and so fetches everything.
#[derive(Default)]
pub struct Fetched {
    authors: Vec<smw::Author>,
    releases: Vec<smw::Version>,
    /// Passages, in the pieces they were asked in: keyed by the first character of the world they
    /// leave. See [`crate::smw::connections`].
    passages: BTreeMap<char, Vec<smw::Connection>>,
}

/// How much of the wiki a sync re-reads.
pub enum Refresh {
    /// All of it, whatever the wiki says about itself. What a sync does when it has no dump to
    /// ask the wiki about, or one the wiki no longer remembers that far back.
    Everything,
    /// Only what these pages can have changed, spelled without the namespace. An empty list is a
    /// wiki that has not moved, and never reaches here -- the caller stands the sync down instead.
    Pages(Vec<String>),
}

impl Refresh {
    /// Whether one page is in scope, by name.
    fn touches(&self, page: &str) -> bool {
        match self {
            Refresh::Everything => true,
            Refresh::Pages(pages) => pages.iter().any(|edited| edited == page),
        }
    }

    /// Whether any page under a prefix is, which is how the version history is asked about: it is
    /// written across a dozen subpages of one title.
    fn touches_under(&self, prefix: &str) -> bool {
        match self {
            Refresh::Everything => true,
            Refresh::Pages(pages) => pages.iter().any(|edited| edited.starts_with(prefix)),
        }
    }

    /// The pieces of the passage query these pages fall into, kept to the ones that exist.
    ///
    /// A passage is written up on the page of the world it leaves, so an edited page can only
    /// have changed the piece its own title falls in.
    fn shards(&self, all: &BTreeSet<char>) -> BTreeSet<char> {
        match self {
            Refresh::Everything => all.clone(),
            Refresh::Pages(pages) => pages
                .iter()
                .filter_map(|edited| edited.chars().next())
                .filter(|initial| all.contains(initial))
                .collect(),
        }
    }
}

/// The page the wiki keeps its author list on, and the title its version history is written under.
/// Both are the store's own subjects, and an edit to either is what makes those answers stale.
const AUTHORS: &str = "Authors";
const VERSION_HISTORY: &str = "Version History";

/// The map the game starts the player on. The world built out of it is the origin and opens the
/// dump: every reader walks its routes out from there.
///
/// A map number rather than a title, because the wiki renames a world far more readily than the
/// game renumbers a map.
const ORIGIN_MAP: u32 = 2;

/// Maps that are not part of the game to a reader. A world built out of any of them is published
/// as a secret, which is the dump's one way of saying "do not show this": see [`model::World`].
///
/// Map 1 is the debug room. The wiki documents it as a location like any other, so the store hands
/// it back like any other, but the game never walks the player into it. The wiki carries no
/// property for that, so this is the one the program supplies; every other secret is an operator's
/// own mark, and [`marked_secret`] is where those come from.
const SECRET_MAPS: [u32; 1] = [1];

/// Fetches what `refresh` says is worth fetching and builds the dump.
///
/// `previous` is the last dump published, and it is consulted for two things only: what an
/// operator has marked on the worlds, and when the dump was last rebuilt without asking. The order
/// the worlds go out in used to be a third and is not any more -- see [`published_place`].
/// Everything else in the new dump comes from the wiki or from `fetched`, which is the wiki as of
/// the last time this asked.
pub async fn run(
    http: &reqwest::Client,
    previous: &Dump,
    refresh: Refresh,
    fetched: &mut Fetched,
    progress: &Progress,
) -> smw::Result<Dump> {
    // First and on its own: what the worlds are decides which pieces of the passage query there
    // are to ask for.
    progress.at(progress::WORLDS);
    let locations = smw::locations(http).await?;
    let initials: BTreeSet<char> = locations
        .iter()
        .filter_map(|location| location.title.chars().next())
        .collect();
    // An empty cache is a run that has just come up: there is nothing to keep, so everything is
    // asked for however little the wiki says has changed.
    let cold = fetched.passages.is_empty();
    let want_authors = cold || refresh.touches(AUTHORS);
    let want_releases = cold || refresh.touches_under(VERSION_HISTORY);
    let shards = match cold {
        true => initials.clone(),
        false => refresh.shards(&initials),
    };

    let shards_count = shards.len();
    // One stage for the three, because they are one question: they are awaited together, and the
    // passages are the long half of it. See [`progress`].
    progress.at(progress::PASSAGES);
    let (authors, releases, passages) = tokio::try_join!(
        optional(want_authors, smw::authors(http)),
        optional(want_releases, smw::versions(http)),
        smw::connections(http, shards),
    )?;

    // What came back replaces what it was asked in place of; the rest of the last answer stands.
    if let Some(authors) = authors {
        fetched.authors = authors;
    }
    if let Some(releases) = releases {
        fetched.releases = releases;
    }
    fetched.passages.extend(passages);
    // A piece with no worlds left in it is a letter the wiki no longer has a world under, and
    // holding its passages would keep a deleted world reachable.
    fetched
        .passages
        .retain(|initial, _| initials.contains(initial));

    /// Says which of the parts that are asked for whole were asked for this time, so a log line
    /// reads as an account of what this sync cost rather than of what it ended up holding.
    fn again(read: bool) -> &'static str {
        match read {
            true => "read again",
            false => "kept",
        }
    }
    tracing::info!(
        "read {} worlds and {shards_count} of {} passage groups; {} authors {}, {} releases {}",
        locations.len(),
        initials.len(),
        fetched.authors.len(),
        again(want_authors),
        fetched.releases.len(),
        again(want_releases),
    );
    Ok(assemble(
        locations,
        fetched.passages.values().flatten(),
        &fetched.authors,
        &fetched.releases,
        previous,
        matches!(refresh, Refresh::Everything),
    ))
}

/// Awaits `work` only if it is wanted, so several conditional fetches can still be run as one.
async fn optional<T>(
    wanted: bool,
    work: impl std::future::Future<Output = smw::Result<T>>,
) -> smw::Result<Option<T>> {
    match wanted {
        true => Ok(Some(work.await?)),
        false => Ok(None),
    }
}

/// Joins the fetches into a dump. Split out from [`run`] so it can be exercised without a
/// network.
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

    // One entry per pair of worlds rather than one per row. The wiki writes a passage up once per
    // direction, and occasionally twice for the same direction where there is more than one way
    // through; either way it is the one passage, and its conditions are everything the wiki said
    // about it.
    let mut passages: HashMap<(usize, usize), Passage> = HashMap::new();
    let mut leaving: Vec<Vec<usize>> = vec![Vec::new(); locations.len()];
    for connection in connections {
        // A passage the wiki has marked as gone describes a version of the game nobody is
        // playing, and it would otherwise make removed worlds look reachable.
        if connection.is_removed {
            continue;
        }
        let (Some(&from), Some(&to)) = (
            at.get(connection.origin.as_str()),
            at.get(connection.destination.as_str()),
        ) else {
            // Either end being unknown means the wiki documents a passage to a page that is not a
            // location, which is a hole in the wiki rather than in this program.
            continue;
        };
        let passage = passages.entry((from, to)).or_insert_with(|| {
            leaving[from].push(to);
            Passage::default()
        });
        for attribute in &connection.attributes {
            let Some((flag, wording)) = ConnType::of(attribute, connection) else {
                tracing::debug!("unknown passage attribute {attribute:?}");
                continue;
            };
            passage.flags |= flag;
            if let Some((params, params_jp)) = wording.published() {
                passage
                    .wording
                    .insert(flag.bits(), TypeParams { params, params_jp });
            }
        }
    }

    // The order passages are fetched in is not stable between requests, and both the
    // distances and the published dump walk this. Sorted so two syncs of the same wiki produce
    // the same document.
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
                    .map(|&to| (to, passages[&(at, to)].flags))
                    .collect(),
            })
            .collect::<Vec<_>>(),
    );

    // A world the game no longer has is measured -- it is part of the graph, and a live world can
    // sit behind one -- but not published. So the published index is not the fetched index, and
    // every passage has to be renumbered into it.
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
    let worlds = locations
        .iter()
        .enumerate()
        .filter(|(at, _)| published[*at].is_some())
        .map(|(at, location)| {
            let connections: Vec<Connection> = leaving[at]
                .iter()
                .filter_map(|&to| {
                    let passage = &passages[&(at, to)];
                    Some(Connection {
                        target_id: published[to]?,
                        flags: passage.flags.bits(),
                        type_params: passage.wording.clone(),
                    })
                })
                .collect();

            World {
                id: published[at].expect("filtered to published worlds"),
                title: location.title.clone(),
                title_jp: text(location.original_name.as_deref()),
                author: location.primary_author.clone().unwrap_or_default(),
                depth: distances[at].0,
                min_depth: distances[at].1,
                filename: encode_uri(&location.location_image),
                map_url: joined(location.location_maps.iter().map(|map| map.path.clone())),
                map_label: joined(location.location_maps.iter().map(|map| map.caption.clone())),
                map_ids: location.map_ids.clone(),
                bgm_url: joined(location.bgms.iter().map(|bgm| bgm.path.clone())),
                // Two fields packed into one, since the reader takes them apart together with the
                // paths above and a track with neither still has to hold its place in the list.
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
                secret: secret.contains(location.title.as_str())
                    || location.map_ids.iter().any(|id| SECRET_MAPS.contains(id)),
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
                // One field rather than a list, as the reference dump writes it. Nothing reads it
                // yet; it is published because the release it belongs to is.
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

/// One passage between two worlds, gathered from however many rows describe it.
#[derive(Default)]
struct Passage {
    flags: ConnType,
    /// The wiki's words for whichever conditions it writes words for, keyed by the flag imposing
    /// them.
    wording: std::collections::BTreeMap<i16, TypeParams>,
}

/// The worlds this dump is about, in the order they go out in: [`published_place`] order.
///
/// Every world the wiki documents is here, the secrets included. A secret is published and marked
/// rather than dropped, for two reasons: the mark is carried from one dump to the next by title
/// (see [`marked_secret`]), so a dropped world is a mark forgotten at the next sync, and hiding is
/// a question about a reader rather than about the game -- the client is what acts on it.
///
/// A world's published id is its index in this list, so the order is part of the interface: it is
/// what a client's own caches are keyed by, and what the thumbnail atlas is packed in. It is
/// therefore worth being a property of the game rather than of this program's history, and it is:
/// two runs that read the same wiki publish the same ids, whether either of them had a dump to
/// start from or not.
fn published_worlds(mut locations: Vec<smw::Location>) -> Vec<smw::Location> {
    locations.sort_by(|one, other| published_place(one).cmp(&published_place(other)));
    locations
}

/// Where one world belongs: the origin first, then by the earliest RPG Maker map the world is
/// built out of, then by title.
///
/// The map numbers are the game's own, handed out in the order the maps were made -- so this is
/// very nearly the order the worlds were added in, and a world added next week takes a number
/// above every number now in use and lands at the end. That is the "relatively" in relatively
/// stable: a world already published moves only if the wiki corrects which maps it is, and a new
/// one mostly moves nothing.
///
/// [`ORIGIN_MAP`] is named rather than left to that rule, and has to be: the debug room is map 1
/// and would otherwise open the dump. It is also the one place in the order a reader depends on.
///
/// Two worlds sharing their earliest map -- ninety-odd groups do -- are separated by title, and
/// the worlds the wiki names no map for sort after everything by the same rule. Neither is
/// arbitrary in the sense that matters: the same input gives the same answer.
fn published_place(location: &smw::Location) -> (bool, u32, &str) {
    (
        !location.map_ids.contains(&ORIGIN_MAP),
        location.map_ids.iter().copied().min().unwrap_or(u32::MAX),
        &location.title,
    )
}

/// The worlds an operator has marked as a spoiler, which no sync should unmark.
fn marked_secret(previous: &Dump) -> std::collections::HashSet<&str> {
    previous
        .worlds
        .iter()
        .filter(|world| world.secret)
        .map(|world| world.title.as_str())
        .collect()
}

/// `Some` for text that says something, `None` for an absent or empty field.
///
/// The wiki holds both, and the reader treats an empty string as a value it has -- an empty
/// Japanese title would be shown as a world's name.
fn text(value: Option<&str>) -> Option<String> {
    value.filter(|text| !text.is_empty()).map(str::to_owned)
}

/// Packs a list into the one `|`-separated field the dump publishes it as, or `None` for an empty
/// one.
///
/// Entries are kept even when blank: the reader reads two of these fields in step with each
/// other, so a gap in one has to be a gap in the other.
fn joined(parts: impl Iterator<Item = String>) -> Option<String> {
    let parts: Vec<String> = parts.collect();
    (!parts.is_empty()).then(|| parts.join("|"))
}

/// Percent-encodes an address the way a browser's `encodeURI` does.
///
/// The wiki serves pictures under the page titles they were uploaded for, so an address can carry
/// a space or a Japanese character verbatim. Every reader of the dump hands these straight to an
/// HTTP client, and those refuse an address with a raw space in it.
fn encode_uri(url: &str) -> String {
    /// What `encodeURI` leaves alone: the unreserved set, the reserved delimiters that make an
    /// address an address, and `#`.
    const KEEP: &str = ";,/?:@&=+$-_.!~*'()#";
    let mut encoded = String::with_capacity(url.len());
    for byte in url.bytes() {
        if byte.is_ascii_alphanumeric() || KEEP.as_bytes().contains(&byte) {
            encoded.push(byte as char);
        } else if byte == b'%' {
            // Already-encoded input is left as it is rather than encoded twice, which is where
            // `encodeURI` and this part company: it would turn `%27` into `%2527`. The wiki
            // serves both forms, so re-encoding would break the addresses that are already
            // right.
            encoded.push('%');
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

/// The two instants a dump is stamped with: when it was built, and when it was last built without
/// first asking whether it needed to be.
///
/// The second is the one that says something. A soft sync only runs at all once the wiki has said
/// it changed, so the dump it publishes is exactly as complete as any other -- but it was reached
/// by trusting the wiki's own account of itself, and if that account were ever wrong, no soft sync
/// would notice. A full sync is the pass that does not ask. What `lastFullUpdate` marks is the
/// last time this program saw the whole wiki for itself, so it is carried over by a soft sync
/// rather than moved. A dump with no previous stamp is being built for the first time, and that is
/// a full sync however it was asked for.
fn stamps(previous: &Dump, full: bool) -> (Option<String>, Option<String>) {
    let now = stamp();
    let last_full = match full {
        true => None,
        false => previous.last_full_update.clone(),
    };
    (Some(now.clone()), Some(last_full.unwrap_or(now)))
}

/// How much of the wiki's record of its own recent changes is worth trusting.
///
/// MediaWiki keeps that record for a fixed span and then forgets, so a question about a moment
/// further back than it reaches is answered with the changes it still has rather than with a
/// complaint -- and a dump older than that would read an empty answer as "nothing has changed"
/// and stay stale for ever. The wiki's default is ninety days; a month is well inside it and
/// leaves room for the wiki to be configured tighter than the default.
const HORIZON: time::Duration = time::Duration::days(30);

/// How far before the last dump a soft sync starts looking.
///
/// The store is not written by the edit that changes it: a job queue re-reads the page afterwards,
/// and until it has, a query answers with what the page used to say. A sync that asked only about
/// what changed since it last ran would take the stale answer, move its stamp past the edit, and
/// never ask again. Looking back an hour costs a handful of pages re-read and closes that window.
const MARGIN: time::Duration = time::Duration::hours(1);

/// The moment a soft sync asks the wiki about, or `None` for a dump too old for the question to
/// mean anything -- which is a full sync's job. See [`HORIZON`] and [`MARGIN`].
pub fn asked_from(since: &str, now: time::OffsetDateTime) -> Option<String> {
    // A stamp that will not parse was not written by this program, and there is nothing to date
    // the question from. Reading the whole wiki is the safe answer to that.
    let since = moment(since)?;
    (now - since < HORIZON).then(|| iso(since - MARGIN))
}

/// Reads a stamp back out of a dump. `None` for anything [`iso`] did not write.
pub fn moment(stamp: &str) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).ok()
}

/// Now, in the format the reference implementation's dump stamps itself with.
fn stamp() -> String {
    iso(time::OffsetDateTime::now_utc())
}

/// An instant, in that format. Also what a release is dated with, since a dump that wrote its own
/// two stamps one way and its release dates another would be two conventions for one reader.
pub fn iso(now: time::OffsetDateTime) -> String {
    // Written out rather than formatted with a description, because the shape is fixed and the
    // milliseconds are always zero: the reader only ever reads the day out of it.
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
    /// The wiki serves pictures under the page titles they were uploaded for, so an address can
    /// carry a space or a Japanese character. Every reader hands these to an HTTP client, which
    /// refuses them raw.
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

    /// The published order is the game's map numbering, not this program's history: the origin
    /// first, then earliest map, then title. Nothing about it reads the last dump, which is what
    /// makes a run that comes up with nothing publish the same ids as one that did not.
    #[test]
    fn the_worlds_are_published_in_the_order_the_game_made_them() {
        let world = |title: &str, map_ids: &[u32]| crate::smw::Location {
            title: title.to_owned(),
            map_ids: map_ids.to_vec(),
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
            // Map 1: the game never walks the player here, but the dump still carries it -- it
            // is published as a secret, and hiding it is the client's job. See [`SECRET_MAPS`].
            world("Debug Room", &[1]),
            world("Nexus", &[10, 11]),
            // The earliest of its maps places it, not the first one the wiki lists.
            world("Chocolate World", &[620, 12]),
            world("Urotsuki's Room", &[2, 224]),
            // Named no map at all: after everything, and after each other by title.
            world("River Road", &[]),
            world("FC Caverns", &[]),
            // Shares its earliest map with Nexus, so the title separates the two.
            world("Hand Hub", &[10, 99]),
        ]);
        assert_eq!(
            published
                .iter()
                .map(|location| location.title.as_str())
                .collect::<Vec<_>>(),
            [
                "Urotsuki's Room",
                // Named rather than earned: map 1 is lower than the origin's map 2, so only the
                // origin coming first by rule keeps the debug room from opening the dump.
                "Debug Room",
                "Hand Hub",
                "Nexus",
                "Chocolate World",
                "FC Caverns",
                "River Road",
            ]
        );
    }

    /// `lastFullUpdate` is the one thing in a dump that a soft sync must not touch: it is how a
    /// reader tells a dump the wiki was taken at its word for from one this program checked for
    /// itself. A first dump has neither stamp, and gets both.
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

    /// What a soft sync re-reads is decided by which pages the wiki says have been edited, and
    /// the cost of getting it wrong is asymmetric: asking for a piece that had not changed wastes
    /// one request, and failing to ask for one that had leaves the dump quietly wrong until the
    /// next full sync.
    #[test]
    fn only_the_pieces_an_edited_page_belongs_to_are_asked_for_again() {
        let letters: std::collections::BTreeSet<char> = "ABS".chars().collect();
        let edited = super::Refresh::Pages(vec![
            "Snow Village".to_owned(),
            "Authors".to_owned(),
            "Version History/0089-0000".to_owned(),
        ]);
        assert!(edited.touches("Authors"), "the author list was edited");
        assert!(
            edited.touches_under("Version History"),
            "the history is written across subpages, so the prefix is what matches"
        );
        assert!(
            !edited.touches("Snow Village/Maps"),
            "and a name is not a prefix"
        );
        // One world edited, and only the piece its title falls in re-asked -- `S` for the world
        // and `A` for the author page, which is not a world but shares a letter with several.
        assert_eq!(edited.shards(&letters), "AS".chars().collect());

        let everything = super::Refresh::Everything;
        assert!(everything.touches("anything at all"));
        assert_eq!(everything.shards(&letters), letters);
    }

    /// The two corrections a soft sync makes to "everything since the dump was built", both of
    /// which exist because trusting the wiki's account of itself literally would lose edits.
    #[test]
    fn the_wiki_is_asked_about_a_little_before_the_dump_was_built() {
        let now = time::OffsetDateTime::from_unix_timestamp(1_788_393_600).expect("a moment");
        let built = super::iso(now - time::Duration::hours(6));

        // Back an hour, because the store is indexed after the edit that changed it and a query
        // inside that window answers with what the page used to say.
        assert_eq!(
            super::asked_from(&built, now).as_deref(),
            Some(super::iso(now - time::Duration::hours(7)).as_str())
        );

        // A dump older than the wiki's memory cannot be asked what changed since: the answer
        // would be "nothing I still know about", which reads exactly like "nothing".
        let stale = super::iso(now - time::Duration::days(45));
        assert_eq!(
            super::asked_from(&stale, now),
            None,
            "read the whole wiki instead"
        );
        assert_eq!(
            super::asked_from("last tuesday", now),
            None,
            "as for a stamp with no moment in it"
        );
    }

    /// An address the wrapper has already escaped is left alone rather than escaped twice, which
    /// would turn `%27` into `%2527` and serve nobody a picture.
    #[test]
    fn an_already_escaped_address_is_left_as_it_is() {
        let escaped = "https://yume.wiki/images/0/Urotsuki%27s_Room.png";
        assert_eq!(super::encode_uri(escaped), escaped);
    }
}
