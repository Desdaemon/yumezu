//! `GET /getNextLocations`: the exit from a world toward the player's destination, for
//! YNOproject's game client, routed by [`yumezu_routing`].

use axum::extract::{Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::model::{ConnType, Dump};
use crate::store::Routing;
use yumezu_routing::World as _;

/// The CORS origin the reference implementation allows.
const ALLOWED_ORIGIN: &str = "https://ynoproject.net";

/// As many next locations as the reference implementation returns.
const MAX_NEXT_LOCATIONS: usize = 3;

#[derive(Deserialize)]
pub struct NextLocationsQuery {
    origin: Option<String>,
    dest: Option<String>,
}

/// One step out of the player's world, in the reference implementation's format.
#[derive(Serialize)]
struct NextLocation<'a> {
    title: &'a str,
    #[serde(rename = "titleJP")]
    title_jp: Option<&'a str>,
    /// The wiki's bitfield for the step out of the origin; see [`outgoing_connection`].
    #[serde(rename = "connType")]
    conn_type: u16,
    #[serde(rename = "typeParams")]
    type_params: std::collections::BTreeMap<u16, yumezu_routing::TypeParams>,
    /// Steps from the origin to the destination through this location, the first step included.
    depth: u32,
}

/// The reference implementation's error body, sent with status `200`.
#[derive(Serialize)]
struct InvalidRequest {
    error: &'static str,
    err_code: &'static str,
}

pub async fn get_next_locations(
    State(server): State<crate::Server>,
    Query(trip): Query<NextLocationsQuery>,
) -> Response {
    let snapshot = server.store.snapshot();
    let world_index = |title: &Option<String>| {
        title
            .as_deref()
            .and_then(|title| snapshot.routing.world_named(title))
    };
    let (Some(origin), Some(dest)) = (world_index(&trip.origin), world_index(&trip.dest)) else {
        return with_cors_header(axum::Json(InvalidRequest {
            error: "Invalid request",
            err_code: "INVALID_REQUEST",
        }));
    };

    with_cors_header(axum::Json(next_locations(
        &snapshot.dump,
        &snapshot.routing,
        origin,
        dest,
    )))
}

/// Every step out of `origin` that reaches `dest`, nearest first, at most [`MAX_NEXT_LOCATIONS`].
/// As in the reference implementation, routes via a secret world are kept only if every route is.
fn next_locations<'a>(
    dump: &'a Dump,
    routing: &Routing,
    origin: usize,
    dest: usize,
) -> Vec<NextLocation<'a>> {
    if origin == dest {
        return Vec::new();
    }
    let toward_dest = yumezu_routing::routes_toward(&routing.connections, dest);
    let mut candidates: Vec<(u32, bool, usize)> = routing.connections[origin]
        .iter()
        .filter(|step| step.outgoing.is_some())
        .filter_map(|step| {
            let steps_left = toward_dest.depth[step.world]?;
            let via_secret = toward_dest
                .route_from(step.world)
                .into_iter()
                .any(|world| dump.worlds[world].secret);
            Some((steps_left + 1, via_secret, step.world))
        })
        .collect();
    // Secret-free first, then shortest; the index breaks ties deterministically.
    candidates.sort_unstable_by_key(|&(depth, via_secret, world)| (via_secret, depth, world));
    if let Some(&(_, first_via_secret, _)) = candidates.first()
        && !first_via_secret
    {
        candidates.retain(|&(_, via_secret, _)| !via_secret);
    }

    candidates.truncate(MAX_NEXT_LOCATIONS);
    candidates
        .into_iter()
        .map(|(depth, _, world)| {
            let (conn_type, type_params) = outgoing_connection(dump, origin, world);
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

/// The connection out of `origin` toward `world`, as published in the dump.
/// Unlisted by `origin`: [`ConnType::LOCKED`] if the far side unlocks, otherwise unconditional.
fn outgoing_connection(
    dump: &Dump,
    origin: usize,
    world: usize,
) -> (
    u16,
    std::collections::BTreeMap<u16, yumezu_routing::TypeParams>,
) {
    if let Some(listed) = dump.worlds[origin].connection_to(world) {
        return (listed.bits, listed.type_params.clone());
    }
    let back = dump.worlds[world]
        .connection_to(origin)
        .map(|connection| connection.flags())
        .unwrap_or_default();
    let inferred = match back.contains(ConnType::UNLOCK) {
        true => ConnType::LOCKED,
        false => ConnType::empty(),
    };
    (inferred.bits(), Default::default())
}

fn with_cors_header(body: impl IntoResponse) -> Response {
    (
        [(
            header::ACCESS_CONTROL_ALLOW_ORIGIN,
            HeaderValue::from_static(ALLOWED_ORIGIN),
        )],
        body,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use crate::model::Dump;
    use crate::store::Routing;
    use yumezu_routing::World as _;

    fn dump() -> Dump {
        crate::model::dream_tree()
    }

    fn world_index(dump: &Dump, title: &str) -> usize {
        dump.worlds
            .iter()
            .position(|world| world.title == title)
            .unwrap_or_else(|| panic!("the dump has no {title}"))
    }

    #[test]
    fn every_next_location_is_a_step_a_player_can_take() {
        let dump = dump();
        let routing = Routing::of(&dump);
        let origin = world_index(&dump, yumezu_routing::START);
        let dest = world_index(&dump, "Marzipan Road");

        let ways = super::next_locations(&dump, &routing, origin, dest);
        assert!(!ways.is_empty(), "no way from the origin to Marzipan Road");
        assert!(ways.len() <= super::MAX_NEXT_LOCATIONS);
        assert!(
            ways.windows(2).all(|pair| pair[0].depth <= pair[1].depth),
            "the nearest next location is not offered first"
        );

        let toward = yumezu_routing::routes_toward(&routing.connections, dest);
        for way in &ways {
            let at = world_index(&dump, way.title);
            let step = yumezu_routing::step_between(&routing.connections, origin, at)
                .expect("a way on is one of the origin's own connections");
            assert!(
                step.outgoing.is_some(),
                "{} cannot be reached in one step",
                way.title
            );
            assert_eq!(way.depth, toward.depth[at].expect("it reaches") + 1);
        }
    }

    #[test]
    fn standing_at_the_destination_is_not_sent_anywhere() {
        let dump = dump();
        let routing = Routing::of(&dump);
        let nexus = world_index(&dump, "Nexus");
        assert!(super::next_locations(&dump, &routing, nexus, nexus).is_empty());
    }

    #[test]
    fn a_door_the_origin_does_not_list_is_still_described() {
        let dump = dump();
        let routing = Routing::of(&dump);
        let unlisted = (0..dump.worlds.len()).find_map(|from| {
            let to = routing.connections[from].iter().find(|step| {
                step.outgoing.is_some() && dump.worlds[from].connection_to(step.world).is_none()
            })?;
            Some((from, to.world))
        });
        let Some((from, to)) = unlisted else {
            return;
        };
        let (conn_type, params) = super::outgoing_connection(&dump, from, to);
        assert!(params.is_empty(), "words invented for an unlisted way");
        let back = dump.worlds[to]
            .connection_to(from)
            .expect("the far side lists what the origin does not");
        let locked = back.flags().contains(super::ConnType::UNLOCK);
        assert_eq!(
            conn_type,
            super::ConnType::LOCKED.bits() * u16::from(locked)
        );
    }
}
