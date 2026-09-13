//! `GET /getNextLocations` -- which way out of here leads to where the player is trying to get.
//!
//! The one question about the dump that handing over the dump does not answer: the caller is
//! YNOproject's game client, which knows where a player is standing and has no graph to walk. The
//! walk is [`yumezu_routing`]'s, the same one the app walks, so a door offered here is a door that
//! app draws a line through.

use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::model::{ConnType, Dump};
use crate::store::Routing;

/// The one site that asks, as the reference implementation pins it.
const ASKED_BY: &str = "https://ynoproject.net";

/// As many ways on as the reference implementation offers.
const CANDIDATES: usize = 3;

#[derive(Deserialize)]
pub struct Trip {
    origin: Option<String>,
    dest: Option<String>,
}

/// One way out of where the player is, as the reference implementation answers it.
#[derive(Serialize)]
struct NextLocation<'a> {
    title: &'a str,
    #[serde(rename = "titleJP")]
    title_jp: Option<&'a str>,
    /// The wiki's bitfield for the connection out of the origin, inferred where only the far side
    /// lists it: see [`leaving`].
    #[serde(rename = "connType")]
    conn_type: u16,
    #[serde(rename = "typeParams")]
    type_params: std::collections::BTreeMap<u16, yumezu_routing::TypeParams>,
    /// Connections from the origin to the destination this way, this door counted.
    depth: u32,
}

/// The reference implementation's own error, and its `200` with it.
#[derive(Serialize)]
struct Invalid {
    error: &'static str,
    err_code: &'static str,
}

pub async fn get_next_locations(
    State(server): State<crate::Server>,
    Query(trip): Query<Trip>,
) -> Response {
    let snapshot = server.store.snapshot();
    let named = |title: &Option<String>| {
        title
            .as_deref()
            .and_then(|title| snapshot.routing.world_named(title))
    };
    let (Some(origin), Some(dest)) = (named(&trip.origin), named(&trip.dest)) else {
        return answer(axum::Json(Invalid {
            error: "Invalid request",
            err_code: "INVALID_REQUEST",
        }));
    };

    answer(axum::Json(ways_on(
        &snapshot.dump,
        &snapshot.routing,
        origin,
        dest,
    )))
}

/// Every way out of `origin` that still reaches `dest`, nearest first and at most [`CANDIDATES`].
///
/// A route through a world the wiki keeps secret is held back unless there is no other, which is
/// the reference implementation's rule.
fn ways_on<'a>(
    dump: &'a Dump,
    routing: &Routing,
    origin: usize,
    dest: usize,
) -> Vec<NextLocation<'a>> {
    // Every way on from here would be a way back here.
    if origin == dest {
        return Vec::new();
    }
    let toward = yumezu_routing::routes_toward(&routing.connections, dest);
    let mut ways: Vec<(u32, bool, usize)> = routing.connections[origin]
        .iter()
        .filter(|step| step.out.is_some())
        .filter_map(|step| {
            let left = toward.depth[step.world]?;
            let secret = toward
                .walk_from(step.world)
                .into_iter()
                .any(|world| dump.worlds[world].secret);
            Some((left + 1, secret, step.world))
        })
        .collect();
    // Secret-free first, then shortest; the world's own index breaks ties, so two identical
    // questions are answered identically.
    ways.sort_unstable_by_key(|&(depth, secret, world)| (secret, depth, world));
    if let Some(&(_, through_a_secret, _)) = ways.first()
        && !through_a_secret
    {
        ways.retain(|&(_, secret, _)| !secret);
    }

    ways.truncate(CANDIDATES);
    ways.into_iter()
        .map(|(depth, _, world)| {
            let (conn_type, type_params) = leaving(dump, origin, world);
            NextLocation {
                title: &dump.worlds[world].title,
                title_jp: dump.worlds[world].title_jp.as_deref(),
                conn_type,
                type_params,
                depth,
            }
        })
        .collect()
}

/// The connection out of `origin` towards `world`, as the dump publishes it.
///
/// The wiki writes a connection once, so the origin's own page need not list it. Where it does not,
/// the way out is what the far side's listing implies: locked where that side says it unlocks
/// something, unconditional otherwise, and no words either way.
fn leaving(
    dump: &Dump,
    origin: usize,
    world: usize,
) -> (
    u16,
    std::collections::BTreeMap<u16, yumezu_routing::TypeParams>,
) {
    if let Some(listed) = dump.worlds[origin]
        .connections
        .iter()
        .find(|connection| connection.target_id == world)
    {
        return (listed.flags, listed.type_params.clone());
    }
    let back = dump.worlds[world]
        .connections
        .iter()
        .find(|connection| connection.target_id == origin)
        .map(|connection| connection.conditions())
        .unwrap_or_default();
    let inferred = match back.contains(ConnType::UNLOCK) {
        true => ConnType::LOCKED,
        false => ConnType::empty(),
    };
    (inferred.bits(), Default::default())
}

fn answer(body: impl IntoResponse) -> Response {
    (
        [(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static(ASKED_BY),
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use crate::model::Dump;
    use crate::store::Routing;

    fn dump() -> Option<Dump> {
        let json = crate::model::published()?;
        Some(serde_json::from_str(&json).expect("the dump this program writes"))
    }

    fn world(dump: &Dump, title: &str) -> usize {
        dump.worlds
            .iter()
            .position(|world| world.title == title)
            .unwrap_or_else(|| panic!("the dump has no {title}"))
    }

    #[test]
    fn every_way_on_offered_is_a_way_a_player_could_take() {
        let Some(dump) = dump() else { return };
        let routing = Routing::of(&dump);
        let origin = world(&dump, yumezu_routing::START);
        // Far enough that the answer is a route rather than a neighbour.
        let dest = world(&dump, "Sugar Road");

        let ways = super::ways_on(&dump, &routing, origin, dest);
        assert!(!ways.is_empty(), "no way from the origin to Sugar Road");
        assert!(ways.len() <= super::CANDIDATES);
        assert!(
            ways.windows(2).all(|pair| pair[0].depth <= pair[1].depth),
            "the nearest way on is not offered first"
        );

        let toward = yumezu_routing::routes_toward(&routing.connections, dest);
        for way in &ways {
            let at = world(&dump, way.title);
            let step = routing.connections[origin]
                .iter()
                .find(|step| step.world == at)
                .expect("a way on is one of the origin's own connections");
            assert!(step.out.is_some(), "{} cannot be walked to", way.title);
            assert_eq!(way.depth, toward.depth[at].expect("it reaches") + 1);
        }
    }

    #[test]
    fn standing_at_the_destination_is_not_sent_anywhere() {
        let Some(dump) = dump() else { return };
        let routing = Routing::of(&dump);
        let nexus = world(&dump, "Nexus");
        assert!(super::ways_on(&dump, &routing, nexus, nexus).is_empty());
    }

    #[test]
    fn a_door_the_origin_does_not_list_is_still_described() {
        let Some(dump) = dump() else { return };
        let routing = Routing::of(&dump);
        let unlisted = (0..dump.worlds.len()).find_map(|from| {
            let to = routing.connections[from].iter().find(|step| {
                step.out.is_some()
                    && !dump.worlds[from]
                        .connections
                        .iter()
                        .any(|listed| listed.target_id == step.world)
            })?;
            Some((from, to.world))
        });
        let Some((from, to)) = unlisted else {
            return; // A dump listing every connection from both sides has nothing to infer.
        };
        let (conn_type, params) = super::leaving(&dump, from, to);
        assert!(params.is_empty(), "words invented for an unlisted way");
        let back = dump.worlds[to]
            .connections
            .iter()
            .find(|connection| connection.target_id == from)
            .expect("the far side lists what the origin does not");
        let locked = back.conditions().contains(super::ConnType::UNLOCK);
        assert_eq!(
            conn_type,
            super::ConnType::LOCKED.bits() * u16::from(locked)
        );
    }
}
