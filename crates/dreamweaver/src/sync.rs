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
    /// All of it, whatever the wiki says about itself. What a run does when it comes up and what
    /// `POST /update` asks for.
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

/// Fetches what `refresh` says is worth fetching and builds the dump.
///
/// `previous` is the last dump published, and it is consulted for three things only: the order
/// worlds are published in, what an operator has marked on them, and when the dump was last
/// rebuilt without asking. Everything else in the new dump comes from the wiki or from `fetched`,
/// which is the wiki as of the last time this asked.
pub async fn run(
    http: &reqwest::Client,
    previous: &Dump,
    refresh: Refresh,
    fetched: &mut Fetched,
) -> smw::Result<Dump> {
    // First and on its own: what the worlds are decides which pieces of the passage query there
    // are to ask for.
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
    let locations = in_published_order(locations, previous);
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
                name_jp: author.original_name.clone(),
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

/// The fetched worlds, reordered to keep the places they already hold in the published dump.
///
/// A world's published id is its index in the list, so the order is part of the interface: it is
/// what a client's own caches are keyed by, and what the thumbnail atlas is packed in. Worlds the
/// last dump had keep their relative order; worlds it did not have go on the end, in the order the
/// store listed them.
fn in_published_order(locations: Vec<smw::Location>, previous: &Dump) -> Vec<smw::Location> {
    let was: HashMap<&str, usize> = previous
        .worlds
        .iter()
        .map(|world| (world.title.as_str(), world.id))
        .collect();
    let mut locations = locations;
    locations.sort_by_key(|location| {
        was.get(location.title.as_str())
            .copied()
            .unwrap_or(usize::MAX)
    });
    locations
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
    let since = time::OffsetDateTime::parse(since, &time::format_description::well_known::Rfc3339)
        // A stamp that will not parse was not written by this program, and there is nothing to
        // date the question from. Reading the whole wiki is the safe answer to that.
        .ok()?;
    (now - since < HORIZON).then(|| iso(since - MARGIN))
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
