//! What a player can traverse: each connection as a [`Gate`] per direction, and route trees over
//! them; shared, so dreamweaver and the app read a connection the same way.

#[doc(hidden)]
pub mod fixture;
mod routes;

pub use routes::{
    Routes, Way, routes_from, routes_from_unlocked, routes_toward, step_between, step_conditions,
    ways,
};

use serde::{Deserialize, Serialize};

/// The start world's title, in the wiki's English spelling.
pub const START: &str = "Urotsuki's Room";

bitflags::bitflags! {
    /// Independent flags: a connection can be locked, seasonal and one-way at once.
    /// The numbering is the wiki explorer's, published in the dump, and must not change.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct ConnType: u16 {
        /// Traversable from the listing world only, never back.
        const ONE_WAY = 1 << 0;
        /// Traversable only into the listing world, never out.
        const NO_ENTRY = 1 << 1;
        /// This side opens a connection the far side reports as [`ConnType::LOCKED`].
        const UNLOCK = 1 << 2;
        const LOCKED = 1 << 3;
        /// Leads to a part of the destination with no way onward.
        const DEAD_END = 1 << 4;
        /// The far side of a dead end: reachable only from that isolated part.
        const ISOLATED = 1 << 5;
        /// Open only to a player wearing particular effects.
        const EFFECT = 1 << 6;
        const CHANCE = 1 << 7;
        /// Open only once a condition is met; the wiki states the condition as a sentence.
        const LOCKED_CONDITION = 1 << 8;
        const SHORTCUT = 1 << 9;
        /// A shortcut's exit, traversed backwards into the shortcut.
        const EXIT_POINT = 1 << 10;
        const SEASONAL = 1 << 11;
        /// Documented but no longer traversable.
        const INACCESSIBLE = 1 << 12;
        const TRACKED = 1 << 13;
    }
}

/// One connection as published in the dump, from the listing world's side.
#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    #[serde(rename = "targetId")]
    pub target_id: usize,
    /// The wiki's bitfield as a bare number; [`Connection::flags`] gives the typed form.
    #[serde(rename = "type")]
    pub bits: u16,
    /// The wiki's words for each demand, by flag; ordered, so two dumps of one wiki match as text.
    #[serde(rename = "typeParams", default)]
    pub type_params: std::collections::BTreeMap<u16, TypeParams>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TypeParams {
    /// Always written in a published dump; optional for deserialization only.
    pub params: Option<String>,
    /// Seasons only; the wiki writes every other condition in English alone.
    #[serde(rename = "paramsJP")]
    pub params_jp: Option<String>,
}

impl Connection {
    pub fn flags(&self) -> ConnType {
        ConnType::from_bits_truncate(self.bits)
    }

    fn conditions(&self) -> Conditions {
        Conditions::from_demands(Gate::most_restrictive_first(self.flags()).map(|gate| {
            Demand {
                gate,
                detail: gate
                    .as_conn_type()
                    .and_then(|flag| self.type_params.get(&flag.bits()))
                    .and_then(|words| words.params.clone())
                    .filter(|words| !words.is_empty()),
            }
        }))
    }
}

/// The fields routing reads from a world, implemented by each program's own world type.
pub trait World {
    /// The English title, which conditions use to name a world.
    fn title(&self) -> &str;
    fn connections(&self) -> &[Connection];

    /// The connection this world lists to `target`, if any.
    fn connection_to(&self, target: usize) -> Option<&Connection> {
        self.connections()
            .iter()
            .find(|connection| connection.target_id == target)
    }
}

/// The world the Eyeball Bomb effect returns a player to, from anywhere in the game.
pub const HUB: &str = "Nexus";
/// The effect that returns a player to the [`HUB`], by the wiki's English name.
pub const ESCAPE_EFFECT: &str = "Eyeball Bomb";

/// The [`HUB`]'s index, or `None` for a dump without the hub.
/// No world lists the escape to the hub, so [`ways`] adds an escape step out of every world.
pub fn hub_world(worlds: &[impl World]) -> Option<usize> {
    worlds.iter().position(|world| world.title() == HUB)
}

/// The start world's index, found by title; the first world when no world has the [`START`] title.
pub fn origin(worlds: &[impl World]) -> usize {
    worlds
        .iter()
        .position(|world| world.title() == START)
        .unwrap_or(0)
}

/// One condition of a connection: the gate and the wiki's words.
#[derive(Clone)]
pub struct Demand {
    pub gate: Gate,
    /// `None` where the wiki wrote no words, which includes every inferred direction.
    pub detail: Option<String>,
}

/// Every demand of a connection, most restrictive first.
/// Routes are ordered by [`Conditions::gate`], the most restrictive demand, alone.
#[derive(Clone)]
pub struct Conditions {
    pub gate: Gate,
    pub demands: Vec<Demand>,
}

impl Conditions {
    pub(crate) fn from_demands(demands: impl IntoIterator<Item = Demand>) -> Self {
        let demands: Vec<_> = demands.into_iter().collect();
        Self {
            gate: demands.first().map_or(Gate::Free, |demand| demand.gate),
            demands,
        }
    }

    /// A single demand without the wiki's words; no demand at all for [`Gate::Free`].
    pub fn from_simple(gate: Gate) -> Self {
        Self::from_demands((gate != Gate::Free).then_some(Demand { gate, detail: None }))
    }

    pub fn free() -> Self {
        Self::from_simple(Gate::Free)
    }

    /// The wiki's words for the most restrictive demand, the one [`Conditions::gate`] names.
    pub fn most_restrictive_detail(&self) -> Option<&str> {
        self.demands
            .first()
            .and_then(|demand| demand.detail.as_deref())
    }

    /// A [`Gate::LockedCondition`] leading the demands becomes [`Gate::Revisit`] when the words
    /// name `destination`; no flag marks a revisit, so the English title is matched.
    fn with_revisit(mut self, destination: &str) -> Self {
        let names_destination = self
            .most_restrictive_detail()
            .is_some_and(|words| words.to_lowercase().contains(&destination.to_lowercase()));
        if self.gate == Gate::LockedCondition && names_destination {
            self.gate = Gate::Revisit;
            self.demands[0].gate = Gate::Revisit;
        }
        self
    }
}

/// Least restrictive first, in the reference implementation's order; `Free` demands nothing.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Gate {
    Free,
    Effect,
    Chance,
    Seasonal,
    LockedCondition,
    /// Unlocked from the opposite entrance.
    Locked,
    /// A shortcut's exit, traversed backwards into the shortcut.
    ExitPoint,
    /// Leads to an isolated section of the world at the far end.
    DeadEnd,
    /// The far side of a [`Gate::DeadEnd`], traversable only from the isolated section.
    Isolated,
    /// A condition naming the destination, which only a player already there can meet.
    /// The most restrictive gate; a world with only this way in is still reached.
    Revisit,
}

impl Gate {
    /// Every gate in `flags`, most restrictive first.
    pub fn most_restrictive_first(flags: ConnType) -> impl Iterator<Item = Gate> {
        [
            (ConnType::ISOLATED, Gate::Isolated),
            (ConnType::DEAD_END, Gate::DeadEnd),
            (ConnType::EXIT_POINT, Gate::ExitPoint),
            (ConnType::LOCKED, Gate::Locked),
            (ConnType::LOCKED_CONDITION, Gate::LockedCondition),
            (ConnType::SEASONAL, Gate::Seasonal),
            (ConnType::CHANCE, Gate::Chance),
            (ConnType::EFFECT, Gate::Effect),
        ]
        .into_iter()
        .filter(move |(flag, _)| flags.contains(*flag))
        .map(|(_, gate)| gate)
    }

    /// Whether a player can meet this gate in place: [`Gate::LockedCondition`] or less.
    pub fn passable_without_unlocks(self) -> bool {
        self <= Gate::LockedCondition
    }

    /// Whether a route can continue past a step of this kind; dead-end and isolated steps end one.
    pub fn passable(self) -> bool {
        !matches!(self, Gate::DeadEnd | Gate::Isolated)
    }

    /// The flag the dump keys this gate's words by; `None` for gates without words.
    pub fn as_conn_type(self) -> Option<ConnType> {
        match self {
            Gate::Effect => Some(ConnType::EFFECT),
            Gate::Chance => Some(ConnType::CHANCE),
            Gate::Seasonal => Some(ConnType::SEASONAL),
            Gate::LockedCondition | Gate::Revisit => Some(ConnType::LOCKED_CONDITION),
            Gate::Free | Gate::Locked | Gate::ExitPoint | Gate::DeadEnd | Gate::Isolated => None,
        }
    }
}

/// One connection of a world, from that world's side.
pub struct Step {
    pub world: usize,
    /// Conditions toward [`Step::world`]; `None` with no way there.
    pub outgoing: Option<Conditions>,
    /// Conditions from [`Step::world`] back; `None` with no way back.
    pub incoming: Option<Conditions>,
}

impl Step {
    /// Whether exactly one direction is traversable; drawn as marching dashes.
    pub fn one_way(&self) -> bool {
        self.outgoing.is_some() != self.incoming.is_some()
    }
}

/// Per world, one [`Step`] per connected world: own listings first in dump order, then
/// connections only the far side lists.
pub fn connections(worlds: &[impl World]) -> Vec<Vec<Step>> {
    let conditions_by_pair: std::collections::HashMap<_, _> = directed_steps(worlds)
        .into_iter()
        .enumerate()
        .flat_map(|(from, steps)| {
            steps
                .into_iter()
                .map(move |(to, conditions)| ((from, to), conditions))
        })
        .collect();

    let mut neighbours: Vec<Vec<usize>> = vec![Vec::new(); worlds.len()];
    for (from, world) in worlds.iter().enumerate() {
        for connection in world.connections() {
            let to = connection.target_id;
            if to != from && to < worlds.len() && !neighbours[from].contains(&to) {
                neighbours[from].push(to);
            }
        }
    }
    // The same connections from the far side, for worlds that did not list them.
    for (from, world) in worlds.iter().enumerate() {
        for connection in world.connections() {
            let to = connection.target_id;
            if to != from && to < worlds.len() && !neighbours[to].contains(&from) {
                neighbours[to].push(from);
            }
        }
    }

    neighbours
        .into_iter()
        .enumerate()
        .map(|(from, neighbours)| {
            neighbours
                .into_iter()
                .map(|to| Step {
                    world: to,
                    outgoing: conditions_by_pair.get(&(from, to)).cloned(),
                    incoming: conditions_by_pair.get(&(to, from)).cloned(),
                })
                .collect()
        })
        .collect()
}

/// Per world, every step out with the step's conditions; unlisted directions are inferred.
/// [`ConnType::ONE_WAY`] infers no way back; [`ConnType::UNLOCK`] infers a locked way back.
pub fn directed_steps(worlds: &[impl World]) -> Vec<Vec<(usize, Conditions)>> {
    let listed_pairs: std::collections::HashSet<_> = worlds
        .iter()
        .enumerate()
        .flat_map(|(from, world)| {
            world
                .connections()
                .iter()
                .map(move |connection| (from, connection.target_id))
        })
        .collect();

    let mut steps = vec![Vec::new(); worlds.len()];
    for (from, world) in worlds.iter().enumerate() {
        for connection in world.connections() {
            let (to, flags) = (connection.target_id, connection.flags());
            if to == from || to >= worlds.len() {
                continue;
            }
            if !flags.contains(ConnType::NO_ENTRY) {
                steps[from].push((to, connection.conditions().with_revisit(worlds[to].title())));
            }
            if !listed_pairs.contains(&(to, from)) && !flags.contains(ConnType::ONE_WAY) {
                // The wiki wrote no words for an unlisted direction.
                let back = Conditions::from_simple(match flags.contains(ConnType::UNLOCK) {
                    true => Gate::Locked,
                    false => Gate::Free,
                });
                steps[to].push((from, back));
            }
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::{
        Conditions, ConnType, Connection, Demand, Gate, Step, TypeParams, connections,
        routes_toward, ways,
    };

    use crate::fixture::{Place, chain, connection};

    #[test]
    fn a_connection_lists_every_demand_harshest_first() {
        let gates = |flags| Gate::most_restrictive_first(flags).collect::<Vec<_>>();
        assert_eq!(gates(ConnType::empty()), []);
        assert_eq!(gates(ConnType::ONE_WAY | ConnType::NO_ENTRY), []);
        assert_eq!(gates(ConnType::EFFECT), [Gate::Effect]);
        assert_eq!(
            gates(ConnType::CHANCE | ConnType::LOCKED_CONDITION),
            [Gate::LockedCondition, Gate::Chance]
        );
        assert_eq!(
            gates(ConnType::LOCKED | ConnType::EFFECT),
            [Gate::Locked, Gate::Effect]
        );
        assert_eq!(
            gates(ConnType::LOCKED_CONDITION | ConnType::EXIT_POINT),
            [Gate::ExitPoint, Gate::LockedCondition]
        );
    }

    // The dump writes words per flag, so a locked effect gate has words only for the effect.
    #[test]
    fn a_connection_keeps_the_words_of_every_demand() {
        let connection = Connection {
            target_id: 0,
            bits: (ConnType::LOCKED | ConnType::EFFECT).bits(),
            type_params: [(
                ConnType::EFFECT.bits(),
                TypeParams {
                    params: Some("Twintails".to_owned()),
                    params_jp: None,
                },
            )]
            .into_iter()
            .collect(),
        };

        let conditions = connection.conditions();

        assert_eq!(conditions.gate, Gate::Locked);
        assert_eq!(
            conditions
                .demands
                .iter()
                .map(|demand| (demand.gate, demand.detail.as_deref()))
                .collect::<Vec<_>>(),
            [(Gate::Locked, None), (Gate::Effect, Some("Twintails"))]
        );
    }

    // `dreamweaver`'s `allow_next_restriction` allows conditions in this order.
    #[test]
    fn a_route_allows_conditions_in_the_references_order() {
        let order = [
            Gate::Free,
            Gate::Effect,
            Gate::Chance,
            Gate::Seasonal,
            Gate::LockedCondition,
            Gate::Locked,
            Gate::ExitPoint,
            Gate::DeadEnd,
            Gate::Isolated,
        ];
        assert!(order.is_sorted());
    }

    // The wiki marks a revisit with a sentence alone.
    #[test]
    fn a_condition_naming_where_it_leads_is_not_a_way_in() {
        let with_words = |gate, detail: &str| {
            Conditions::from_demands([Demand {
                gate,
                detail: Some(detail.to_owned()),
            }])
        };
        assert_eq!(
            with_words(
                Gate::LockedCondition,
                "If Fluorescent Halls has been visited before"
            )
            .with_revisit("Fluorescent Halls")
            .gate,
            Gate::Revisit
        );
        assert_eq!(
            with_words(Gate::LockedCondition, "View Ending #1 at least once")
                .with_revisit("Oil Puddle World B")
                .gate,
            Gate::LockedCondition
        );
        assert_eq!(
            with_words(Gate::Effect, "Chainsaw the Tree of Life in Blood Cell Sea")
                .with_revisit("Blood Cell Sea")
                .gate,
            Gate::Effect
        );
    }

    #[test]
    fn a_way_through_a_far_side_lock_never_replaces_one_a_visitor_can_take() {
        let place = |title, connections| Place { title, connections };
        let without_words = |to, flags| connection(to, flags, None);
        let worlds = [
            place(
                "Nexus",
                vec![
                    without_words(1, ConnType::empty()),
                    without_words(4, ConnType::empty()),
                ],
            ),
            place(
                "Lantern Causeway",
                vec![without_words(2, ConnType::empty())],
            ),
            place("Mural Vestibule", vec![without_words(3, ConnType::EFFECT)]),
            place("Ember Terrace", vec![without_words(4, ConnType::UNLOCK)]),
            place("Chalk Observatory", vec![]),
        ];
        let joined = connections(&worlds);

        let ways = ways(&joined, 0, 3, 5, Some(0));

        assert_eq!(
            ways.iter()
                .map(|way| way.worlds.clone())
                .collect::<Vec<_>>(),
            [vec![0, 1, 2, 3], vec![0, 4, 3]]
        );
        assert_eq!(ways[0].gate, Gate::Effect);
        assert_eq!(ways[1].gate, Gate::Locked);
    }

    #[test]
    fn a_connection_listed_once_is_traversable_both_ways() {
        let joined = connections(&chain(&["Nexus", "Sofa Room"], ConnType::empty()));
        let step = &joined[1][0];
        assert_eq!(step.world, 0);
        assert!(step.outgoing.is_some(), "no way back out of the far side");
        assert!(!step.one_way());
    }

    #[test]
    fn a_route_toward_a_world_counts_the_steps_left() {
        let joined = connections(&chain(
            &["Nexus", "Sofa Room", "Far Room"],
            ConnType::ONE_WAY,
        ));
        let toward = routes_toward(&joined, 2);
        assert_eq!(toward.depth[2], Some(0));
        assert_eq!(toward.depth[0], Some(2));
        assert_eq!(toward.route_from(0), [0, 1, 2]);
        assert_eq!(routes_toward(&joined, 0).depth[2], None);
    }

    #[test]
    fn a_route_never_takes_a_refused_step() {
        let joined: Vec<Vec<Step>> = connections(&chain(
            &["Nexus", "Sofa Room", "Far Room"],
            ConnType::NO_ENTRY,
        ));
        assert_eq!(routes_toward(&joined, 2).depth[0], None);
    }
}
