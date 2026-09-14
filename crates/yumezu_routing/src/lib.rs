//! What a player can walk, and where that gets them.
//!
//! The wiki writes a connection as a bitfield of independent conditions, and a route is only as
//! walkable as its strictest one. That becomes a [`Gate`] per direction and a tree of routes over
//! them, here rather than in either program, so the one publishing the dump and the one drawing it
//! cannot disagree about what a connection means.
//!
//! Saying it is the reader's own business: a [`Gate`] carries the wiki's own words where it wrote
//! any, and nothing here names a condition in anyone's language.

#[doc(hidden)]
pub mod fixture;
mod routes;

pub use routes::{Routes, Way, routes_from, routes_from_passing, routes_toward, step_asks, ways};

use serde::{Deserialize, Serialize};

/// As the wiki's English pages spell it.
pub const START: &str = "Urotsuki's Room";

bitflags::bitflags! {
    /// Independent flags rather than a kind: a connection can be locked behind a condition *and*
    /// seasonal *and* one-way. The numbering is the wiki explorer's own and cannot be renumbered
    /// -- it is what the dump publishes and what a reader reads back.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct ConnType: u16 {
        /// Walkable from the world that lists it, never back.
        const ONE_WAY = 1 << 0;
        /// Walkable only back to the world that lists it, never from it.
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
        /// Open at random.
        const CHANCE = 1 << 7;
        /// Open only once something else has happened, which the wiki writes out as a sentence.
        const LOCKED_CONDITION = 1 << 8;
        const SHORTCUT = 1 << 9;
        /// Where a shortcut comes out, walked backwards into the shortcut.
        const EXIT_POINT = 1 << 10;
        const SEASONAL = 1 << 11;
        /// Documented but no longer walkable.
        const INACCESSIBLE = 1 << 12;
        const TRACKED = 1 << 13;
    }
}

/// One connection as the dump publishes it, from the side of the world that lists it.
#[derive(Clone, Serialize, Deserialize)]
pub struct Connection {
    #[serde(rename = "targetId")]
    pub target_id: usize,
    /// The wiki's own bitfield, as the bare number the dump carries. [`Connection::conditions`]
    /// types it.
    #[serde(rename = "type")]
    pub flags: u16,
    /// The wiki's own words for a demand, keyed by the flag making it. Ordered, so two dumps of
    /// the same wiki compare equal as text.
    #[serde(rename = "typeParams", default)]
    pub type_params: std::collections::BTreeMap<u16, TypeParams>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TypeParams {
    /// Optional only to be read back: the dump has always written words here.
    pub params: Option<String>,
    /// Only ever the four seasons -- every other condition the wiki writes in English alone.
    #[serde(rename = "paramsJP")]
    pub params_jp: Option<String>,
}

impl Connection {
    pub fn conditions(&self) -> ConnType {
        ConnType::from_bits_truncate(self.flags)
    }

    fn ask(&self) -> Ask {
        let gate = Gate::of(self.conditions());
        Ask {
            gate,
            detail: gate
                .worded()
                .and_then(|flag| self.type_params.get(&flag.bits()))
                .and_then(|words| words.params.clone())
                .filter(|words| !words.is_empty()),
        }
    }
}

/// What the walk needs of a world.
///
/// A trait rather than a type of this crate's own, so neither program has to hold its worlds twice.
pub trait World {
    /// The English one, which is what a condition naming a world names it by.
    fn title(&self) -> &str;
    fn connections(&self) -> &[Connection];
}

/// The world the Eyeball Bomb effect returns a player to, from anywhere in the game.
pub const HUB: &str = "Nexus";
/// In the wiki's own English, which is the only language it writes an effect's name in.
pub const ESCAPE: &str = "Eyeball Bomb";

/// The world every other one has a way back to, which no world lists as a connection -- every
/// world would have to. A walk given it takes it as a step out of anywhere, so [`ways`] finds the
/// routes a player who owns the effect would actually take. `None` for a dump without the [`HUB`].
pub fn hub_world(worlds: &[impl World]) -> Option<usize> {
    worlds.iter().position(|world| world.title() == HUB)
}

/// Where the game starts, and so where every canonical route ends.
///
/// By name rather than by position. The dump usually lists it first, but only because it was the
/// first world the reference implementation's database ever held: a dump built from nothing lists
/// the worlds alphabetically and starts at `3D Structures Path`, and seeding the walk there leaves
/// nearly every world unreachable -- one flat layer at no depth. The first world if the dump has no
/// such title, which is not worth failing over.
pub fn origin_world(worlds: &[impl World]) -> usize {
    worlds
        .iter()
        .position(|world| world.title() == START)
        .unwrap_or(0)
}

/// The condition on its own is what a route is ordered by; the words are what a reader is told.
#[derive(Clone)]
pub struct Ask {
    pub gate: Gate,
    /// The wiki's own words for the condition, `None` where it wrote none and always for a
    /// direction that is inferred rather than listed. See [`walkable_steps`].
    pub detail: Option<String>,
}

impl Ask {
    /// Prose is all there is to go on: no flag separates the condition on a shortcut back in from
    /// one a first-time visitor could meet. `destination` is that world's English title, the only
    /// one the wiki writes these sentences in.
    fn first_visit(mut self, destination: &str) -> Self {
        let names_destination = self
            .detail
            .as_deref()
            .is_some_and(|words| words.to_lowercase().contains(&destination.to_lowercase()));
        if self.gate == Gate::LockedCondition && names_destination {
            self.gate = Gate::Revisit;
        }
        self
    }

    pub fn free() -> Self {
        Self {
            gate: Gate::Free,
            detail: None,
        }
    }
}

/// Ordered by how readily a route accepts it: [`Gate::Free`] demands nothing, and the rest follow
/// in the order the wiki's own path finder falls back through them -- conditional before locked,
/// and a shortcut's exit only once both of those are already allowed.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Gate {
    Free,
    Effect,
    Chance,
    Seasonal,
    LockedCondition,
    /// Unlocked from the opposite entrance.
    Locked,
    /// Where a shortcut comes out, walked backwards into the shortcut.
    ExitPoint,
    /// Leads to an isolated section of the world at the far end.
    DeadEnd,
    /// The far side of a [`Gate::DeadEnd`]: the way back is reachable only from that isolated
    /// section.
    Isolated,
    /// A condition naming the world it leads to, which only a player already there can meet.
    /// Harsher than anything the wiki flags, since it is no way in -- but still a step, so a world
    /// it is the only way to is reached rather than lost.
    Revisit,
}

impl Gate {
    /// Harshest wins: several flags are demands to be met together, so the route is only as free
    /// as its strictest one. Listed strictest first: the enum's order, backwards.
    fn of(flags: ConnType) -> Gate {
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
        .find(|(flag, _)| flags.contains(*flag))
        .map_or(Gate::Free, |(_, gate)| gate)
    }

    /// Whether a walk can go on from a step of this kind. A dead end lands a player in a part of
    /// the world with no way out of it but back, so a route can end on one and never pass through
    /// it. Not a condition: no effect and no unlock opens a way onward that is not there.
    pub fn onward(self) -> bool {
        !matches!(self, Gate::DeadEnd | Gate::Isolated)
    }

    /// The flag whose words the dump writes for this condition, for the four it writes any for.
    pub fn worded(self) -> Option<ConnType> {
        match self {
            Gate::Effect => Some(ConnType::EFFECT),
            Gate::Chance => Some(ConnType::CHANCE),
            Gate::Seasonal => Some(ConnType::SEASONAL),
            Gate::LockedCondition | Gate::Revisit => Some(ConnType::LOCKED_CONDITION),
            Gate::Free | Gate::Locked | Gate::ExitPoint | Gate::DeadEnd | Gate::Isolated => None,
        }
    }
}

/// One connection of a world, from that world's side. A direction with no gate is a direction
/// there is no way to walk, which is what makes a connection one-way.
pub struct Step {
    pub world: usize,
    /// `None` where there is no way there.
    pub out: Option<Ask>,
    /// `None` where there is no way back.
    pub back: Option<Ask>,
}

impl Step {
    /// Drawn as marching dashes, and the one thing a line has to know about itself.
    pub fn one_way(&self) -> bool {
        self.out.is_some() != self.back.is_some()
    }
}

/// Per world, every world it is joined to, each with what the connection asks in either direction.
///
/// One entry per connection rather than one per listing: a connection is nearly always listed by
/// both worlds it joins and is still one connection. A world's own listings come first, in the
/// dump's order, then the connections only the far side lists.
///
/// Both the lines a reader draws and the routes walked here come from this, so a line drawn
/// one-way is one-way on exactly the steps a route is denied.
pub fn connections(worlds: &[impl World]) -> Vec<Vec<Step>> {
    let gates: std::collections::HashMap<_, _> = walkable_steps(worlds)
        .into_iter()
        .enumerate()
        .flat_map(|(from, steps)| steps.into_iter().map(move |(to, ask)| ((from, to), ask)))
        .collect();

    let mut joined: Vec<Vec<usize>> = vec![Vec::new(); worlds.len()];
    for (from, world) in worlds.iter().enumerate() {
        for connection in world.connections() {
            let to = connection.target_id;
            // A world connected to itself is no way anywhere.
            if to != from && to < worlds.len() && !joined[from].contains(&to) {
                joined[from].push(to);
            }
        }
    }
    // The same connections again from the far side, for the world that did not list them itself.
    for (from, world) in worlds.iter().enumerate() {
        for connection in world.connections() {
            let to = connection.target_id;
            if to != from && to < worlds.len() && !joined[to].contains(&from) {
                joined[to].push(from);
            }
        }
    }

    joined
        .into_iter()
        .enumerate()
        .map(|(from, joined)| {
            joined
                .into_iter()
                .map(|to| Step {
                    world: to,
                    out: gates.get(&(from, to)).cloned(),
                    back: gates.get(&(to, from)).cloned(),
                })
                .collect()
        })
        .collect()
}

/// Every step a player can take, as a directed adjacency list carrying what each demands.
///
/// Directed and one-sided, unlike [`connections`]: what a world leads to, rather than what it is
/// joined to.
///
/// A connection is nearly always listed by both worlds it joins, and those two listings are the two
/// directions. Where only one side lists it, the other is inferred the way the wiki's own path
/// finder infers it: [`ConnType::ONE_WAY`] means there is no way back, [`ConnType::UNLOCK`] means
/// the way back is [`Gate::Locked`].
pub fn walkable_steps(worlds: &[impl World]) -> Vec<Vec<(usize, Ask)>> {
    let listed: std::collections::HashSet<_> = worlds
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
            let (to, flags) = (connection.target_id, connection.conditions());
            if to == from || to >= worlds.len() {
                continue;
            }
            if !flags.contains(ConnType::NO_ENTRY) {
                steps[from].push((to, connection.ask().first_visit(worlds[to].title())));
            }
            if !listed.contains(&(to, from)) && !flags.contains(ConnType::ONE_WAY) {
                let gate = match flags.contains(ConnType::UNLOCK) {
                    true => Gate::Locked,
                    false => Gate::Free,
                };
                // No words: the wiki wrote none for a direction it did not list at all.
                steps[to].push((from, Ask { gate, detail: None }));
            }
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::{Ask, ConnType, Connection, Gate, Step, World, connections, routes_toward};

    struct Place {
        title: String,
        connections: Vec<Connection>,
    }

    impl World for Place {
        fn title(&self) -> &str {
            &self.title
        }
        fn connections(&self) -> &[Connection] {
            &self.connections
        }
    }

    /// A line of worlds, each listing the next, every connection alike.
    fn chain(titles: &[&str], flags: ConnType) -> Vec<Place> {
        titles
            .iter()
            .enumerate()
            .map(|(at, title)| Place {
                title: (*title).to_owned(),
                connections: (at + 1 < titles.len())
                    .then(|| Connection {
                        target_id: at + 1,
                        flags: flags.bits(),
                        type_params: Default::default(),
                    })
                    .into_iter()
                    .collect(),
            })
            .collect()
    }

    // The mildest demand would understate what the player has to have done.
    #[test]
    fn a_connection_is_named_by_its_harshest_demand() {
        assert_eq!(Gate::of(ConnType::empty()), Gate::Free);
        assert_eq!(Gate::of(ConnType::ONE_WAY | ConnType::NO_ENTRY), Gate::Free);
        assert_eq!(Gate::of(ConnType::EFFECT), Gate::Effect);
        assert_eq!(
            Gate::of(ConnType::CHANCE | ConnType::LOCKED_CONDITION),
            Gate::LockedCondition
        );
        assert_eq!(
            Gate::of(ConnType::LOCKED_CONDITION | ConnType::EXIT_POINT),
            Gate::ExitPoint
        );
    }

    // `dreamweaver`'s `give_up` is the reference for this order.
    #[test]
    fn a_route_gives_conditions_up_in_the_references_order() {
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

    // A condition a player can only meet by already standing where it leads is no way in, and the
    // wiki writes no flag for it -- only the sentence.
    #[test]
    fn a_condition_naming_where_it_leads_is_not_a_way_in() {
        let ask = |gate, detail: &str| Ask {
            gate,
            detail: Some(detail.to_owned()),
        };
        assert_eq!(
            ask(
                Gate::LockedCondition,
                "If Fluorescent Halls has been visited before"
            )
            .first_visit("Fluorescent Halls")
            .gate,
            Gate::Revisit
        );
        assert_eq!(
            ask(Gate::LockedCondition, "View Ending #1 at least once")
                .first_visit("Oil Puddle World B")
                .gate,
            Gate::LockedCondition
        );
        assert_eq!(
            ask(Gate::Effect, "Chainsaw the Tree of Life in Blood Cell Sea")
                .first_visit("Blood Cell Sea")
                .gate,
            Gate::Effect
        );
    }

    #[test]
    fn a_connection_listed_once_is_walkable_both_ways() {
        let joined = connections(&chain(&["Nexus", "Sofa Room"], ConnType::empty()));
        let step = &joined[1][0];
        assert_eq!(step.world, 0);
        assert!(step.out.is_some(), "no way back out of the far side");
        assert!(!step.one_way());
    }

    #[test]
    fn a_walk_towards_a_world_counts_the_steps_left() {
        let joined = connections(&chain(
            &["Nexus", "Sofa Room", "Far Room"],
            ConnType::ONE_WAY,
        ));
        let toward = routes_toward(&joined, 2);
        assert_eq!(toward.depth[2], Some(0));
        assert_eq!(toward.depth[0], Some(2));
        assert_eq!(toward.walk_from(0), [0, 1, 2]);
        assert_eq!(routes_toward(&joined, 0).depth[2], None);
    }

    #[test]
    fn a_walk_never_steps_a_way_that_cannot_be_walked() {
        let joined: Vec<Vec<Step>> = connections(&chain(
            &["Nexus", "Sofa Room", "Far Room"],
            ConnType::NO_ENTRY,
        ));
        // Every listed step is one-way *into* the world listing it: nothing leads onward.
        assert_eq!(routes_toward(&joined, 2).depth[0], None);
    }
}
