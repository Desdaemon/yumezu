//! The tree of routes over what a player can walk, run outward from a world or inward to one.

use super::{Conditions, Demand, ESCAPE, Gate, Step};

/// The route from every world to the world a walk was seeded at: the origin for the canonical
/// tree, and the destination for a set of directions.
pub struct Routes {
    /// Per world, the world one step closer to the seed along its route. `None` for the seed
    /// itself and for anything with no route to it.
    pub parents: Vec<Option<usize>>,
    /// Per world, how many connections its route is long, and `None` where there is none.
    /// Measured here rather than read from the dump's own `depth`, so it agrees with the
    /// connections the walk was given.
    pub depth: Vec<Option<u32>>,
}

impl Routes {
    /// Per world, how many worlds' route home passes through it: how much of the game it is the
    /// way to. A reader sizing its nodes by this reads it through a logarithmic curve, so the
    /// origin -- which everything hangs off -- stays on the same scale as the rest.
    pub fn descendant_counts(&self) -> Vec<u32> {
        // Deepest first, so a world's descendants are all counted before it hands them up.
        let mut order: Vec<usize> = (0..self.parents.len()).collect();
        order.sort_unstable_by_key(|&world| std::cmp::Reverse(self.depth[world]));
        let mut descendants = vec![0; self.parents.len()];
        for &world in &order {
            if let Some(parent) = self.parents[world] {
                descendants[parent] += descendants[world] + 1;
            }
        }
        descendants
    }

    /// A world and every world that hangs off it, shallowest first so a reader walks it outward.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        // Shallowest first, so a world's parent has already been decided when it is reached. A
        // world the seed cannot reach sorts before every depth and has no parent to inherit from,
        // which keeps it out of every subtree but its own.
        let mut order: Vec<usize> = (0..self.parents.len()).collect();
        order.sort_unstable_by_key(|&world| self.depth[world]);
        let mut inside = vec![false; self.parents.len()];
        inside[root] = true;
        for &world in &order {
            if let Some(parent) = self.parents[world] {
                inside[world] |= inside[parent];
            }
        }
        order.retain(|&world| inside[world]);
        order
    }

    /// The way between two worlds, `from` first, and empty where there is none. Only meaningful on
    /// a tree seeded at the far end: see [`routes_toward`].
    pub fn walk_from(&self, from: usize) -> Vec<usize> {
        let mut walk = Vec::new();
        let mut step = self.depth[from].is_some().then_some(from);
        while let Some(world) = step {
            walk.push(world);
            step = self.parents[world];
        }
        walk
    }
}

/// Walks the route to every world a player could actually be expected to walk, outward from
/// `origin`.
///
/// Routes are ordered by the harshest [`Gate`] anywhere along them, then by length, then by how
/// many steps demand anything at all, so an unconditional route wins however long it is. That is
/// why the depth reported here is the higher, honest one: a locked or chance-gated shortcut no
/// longer makes a world look shallow.
///
/// A tree cannot hold that preference exactly -- a long unconditional route beats a short locked
/// one until a locked step is added to both -- so a world's route is the best one *through its
/// parent* rather than the best one there is.
pub fn routes_from(connections: &[Vec<Step>], origin: usize) -> Routes {
    routes_from_passing(connections, origin, Gate::Free)
}

/// The same walk for a player who can already get past everything up to and including `passable`:
/// those steps demand nothing, so the route through them falls to the shortest one. At
/// [`Gate::Revisit`] nothing is demanded anywhere and the walk counts connections alone.
pub fn routes_from_passing(connections: &[Vec<Step>], origin: usize, passable: Gate) -> Routes {
    // Standing at a world, the worlds it can be walked on to.
    walk(connections, origin, None, move |_, step| {
        let conditions = step.out.as_ref()?.gate;
        Some(Taken {
            world: step.world,
            gate: met(conditions, passable),
            onward: conditions.onward(),
        })
    })
}

/// The same walk run backwards: per world, the next world on the way to `destination` and how many
/// steps are left. One search answers both what the directions are and, from anywhere, which ways
/// on still reach the end of them.
pub fn routes_toward(connections: &[Vec<Step>], destination: usize) -> Routes {
    // Standing at a world, the worlds that can be walked on to it.
    // A step into an isolated section can only be the last of a walk, so it is only a way to the
    // world the search is seeded at -- everywhere else, walking on from it is what it does not
    // allow.
    walk(connections, destination, None, |at, step| {
        let conditions = step.back.as_ref()?.gate;
        (conditions.onward() || at == destination).then_some(Taken {
            world: step.world,
            gate: conditions,
            onward: true,
        })
    })
}

/// What taking a step comes to for the walk.
struct Taken {
    world: usize,
    /// What it demands, which is what the route it is part of is ordered by.
    gate: Gate,
    /// Whether the walk can go on from where it lands. See [`Gate::onward`].
    onward: bool,
}

/// `taking` is which of the connections of the world being stood at the walk may take and what
/// each comes to, which is the only difference between walking away from a world and walking
/// towards one -- and where a search is refused a step it has already used.
fn walk(
    connections: &[Vec<Step>],
    seed: usize,
    hub: Option<usize>,
    taking: impl Fn(usize, &Step) -> Option<Taken>,
) -> Routes {
    let mut routes = Routes {
        parents: vec![None; connections.len()],
        depth: vec![None; connections.len()],
    };
    if seed >= connections.len() {
        return routes;
    }

    // Dijkstra over (gate, trapped, depth, demands): a world settles on its parent's route plus
    // one step, so the parent chain and the depth cannot disagree. `demands` sits after the depth
    // and so cannot move it -- it only picks the least demanding of the equally short routes that
    // share one harshest condition. Reversed because `BinaryHeap` is a max-heap; the world and the
    // parent ride in the key so ties resolve the same way on every run.
    //
    // `trapped` sits above the depth so that a world reachable both ways settles on the arrival
    // that can be walked on from, however much longer it is. Settling it on the arrival into an
    // isolated section would leave every world past it reachable only by a route that walks out
    // of a section with no way out.
    let escape = hub.map(escape_step);
    let mut queue = std::collections::BinaryHeap::from([std::cmp::Reverse((
        Gate::Free,
        false,
        0,
        0,
        seed,
        seed,
    ))]);
    while let Some(std::cmp::Reverse((gate, trapped, depth, demands, world, parent))) = queue.pop()
    {
        if routes.depth[world].is_some() {
            continue;
        }
        routes.depth[world] = Some(depth);
        routes.parents[world] = (world != seed).then_some(parent);
        let bomb = escape.iter().filter(|_| Some(world) != hub);
        for step in connections[world].iter().chain(bomb) {
            let Some(taken) = taking(world, step) else {
                continue;
            };
            // An isolated section leads only to another one: the wiki writes the way on out of a
            // pocket as a pocket of its own, and everything else in the world it is a pocket of
            // is on the far side of a wall.
            if trapped && taken.onward {
                continue;
            }
            if routes.depth[taken.world].is_none() {
                queue.push(std::cmp::Reverse((
                    gate.max(taken.gate),
                    !taken.onward,
                    depth + 1,
                    demands + u32::from(taken.gate != Gate::Free),
                    taken.world,
                    world,
                )));
            }
        }
    }
    routes
}

/// What a step demands of a player who can already get past everything up to `passable`.
fn met(conditions: Gate, passable: Gate) -> Gate {
    match conditions <= passable {
        true => Gate::Free,
        false => conditions,
    }
}

fn escape_conditions() -> Conditions {
    Conditions::of([Demand {
        gate: Gate::Effect,
        detail: Some(ESCAPE.to_owned()),
    }])
}

/// One way only, and out of every world but the [`super::HUB`] itself.
fn escape_step(hub: usize) -> Step {
    Step {
        world: hub,
        out: Some(escape_conditions()),
        back: None,
    }
}

/// What stepping from one world to the next demands, the way home the game gives a player
/// everywhere included. `None` where there is no walking it.
pub fn step_conditions(
    connections: &[Vec<Step>],
    hub: Option<usize>,
    from: usize,
    to: usize,
) -> Option<Conditions> {
    let listed = connections[from]
        .iter()
        .find(|step| step.world == to)
        .and_then(|step| step.out.clone());
    listed.or_else(|| (hub == Some(to) && hub != Some(from)).then(escape_conditions))
}

/// Whether a way backs out through an exit point anywhere along it.
fn backs_out(connections: &[Vec<Step>], hub: Option<usize>, way: &[usize]) -> bool {
    way.windows(2).any(|pair| {
        step_conditions(connections, hub, pair[0], pair[1])
            .is_some_and(|step| step.gate == Gate::ExitPoint)
    })
}

/// The harshest thing a way demands anywhere along it, and how many of its steps demand anything
/// at all. Read in the order the way is walked, `from` first.
fn conditions_of(connections: &[Vec<Step>], hub: Option<usize>, way: &[usize]) -> (Gate, u32) {
    conditions_passing(connections, hub, way, Gate::Free)
}

fn conditions_passing(
    connections: &[Vec<Step>],
    hub: Option<usize>,
    way: &[usize],
    passable: Gate,
) -> (Gate, u32) {
    way.windows(2)
        .map(|pair| {
            step_conditions(connections, hub, pair[0], pair[1])
                .map_or(Gate::Free, |step| met(step.gate, passable))
        })
        .fold((Gate::Free, 0), |(harshest, demands), conditions| {
            (
                harshest.max(conditions),
                demands + u32::from(conditions != Gate::Free),
            )
        })
}

/// Up to `want` ways from `from` to `to`, `from` first, best first, no two the same and none
/// walking through a world twice.
///
/// Yen's: the best way there is, then the best way that leaves one already found at some world
/// without taking the step it took from there, and so on. `passable` orders them as it does in
/// [`routes_from_passing`], so it decides which ways are found at all once `want` cuts the list
/// off.
///
/// Best-first only as far as the walk beneath it is: see [`routes_from`] on why a tree cannot hold
/// that ordering exactly.
fn ways_between(
    connections: &[Vec<Step>],
    from: usize,
    to: usize,
    passable: Gate,
    want: usize,
    hub: Option<usize>,
) -> Vec<Vec<usize>> {
    let way = |spur: usize,
               shut: &[bool],
               refused: &std::collections::HashSet<(usize, usize)>|
     -> Option<Vec<usize>> {
        let routes = walk(connections, spur, hub, |at, step| {
            let conditions = step.out.as_ref()?.gate;
            let barred = shut[step.world] || refused.contains(&(at, step.world));
            (!barred).then_some(Taken {
                world: step.world,
                gate: met(conditions, passable),
                onward: conditions.onward(),
            })
        });
        routes.depth[to]?;
        let mut way = routes.walk_from(to);
        way.reverse();
        Some(way)
    };

    let open = vec![false; connections.len()];
    let Some(best) = way(from, &open, &Default::default()) else {
        return Vec::new();
    };
    let mut found = vec![best];
    let mut candidates: Vec<Vec<usize>> = Vec::new();
    while found.len() < want {
        let last = found[found.len() - 1].clone();
        for at in 0..last.len() - 1 {
            let root = &last[..=at];
            // Every step already taken out of this world by a way that reached it the same way,
            // so a candidate has to leave differently rather than rediscover one already found.
            let refused: std::collections::HashSet<(usize, usize)> = found
                .iter()
                .chain(&candidates)
                .filter(|way| way.len() > at + 1 && way[..=at] == *root)
                .map(|way| (way[at], way[at + 1]))
                .collect();
            let mut shut = open.clone();
            for &world in &root[..at] {
                shut[world] = true;
            }
            let Some(spur) = way(root[at], &shut, &refused) else {
                continue;
            };
            let whole: Vec<usize> = root[..at].iter().copied().chain(spur).collect();
            if !found.contains(&whole) && !candidates.contains(&whole) {
                candidates.push(whole);
            }
        }
        let best = candidates.iter().enumerate().min_by_key(|(_, way)| {
            let (conditions, demands) = conditions_passing(connections, hub, way, passable);
            (conditions, way.len(), demands)
        });
        let Some((best, _)) = best else {
            break;
        };
        found.push(candidates.swap_remove(best));
    }
    found
}

/// How much of the game a way may assume the player has already got past. Every level is run,
/// because the ordering is what a level changes: the way a player with nothing can walk and the
/// shortest one with every door open are each some level's own first answer, and neither is near
/// the top of the other's list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Passing {
    Nothing,
    /// Everything a player can meet where they stand: see [`Gate::in_place`].
    Conditions,
    Everything,
}

impl Passing {
    const ALL: [Self; 3] = [Self::Nothing, Self::Conditions, Self::Everything];

    /// The harshest gate this level walks through as though it demanded nothing.
    fn met(self) -> Gate {
        match self {
            Self::Nothing => Gate::Free,
            Self::Conditions => Gate::LockedCondition,
            Self::Everything => Gate::Revisit,
        }
    }
}

/// One way between two worlds, and what taking it comes to.
pub struct Way {
    /// `from` first, as [`ways`] was given them.
    pub walk: Vec<usize>,
    /// The harshest thing it demands anywhere along it, and how many of its steps demand
    /// anything at all: what one way is told apart from another by, the length aside.
    pub conditions: Gate,
    pub demands: u32,
    /// Whether it backs out through an exit point anywhere along it.
    pub backs_out: bool,
}

/// What a way costs a reader: whether it needs a door opened from its far side, then how many of
/// its steps demand anything at all.
///
/// [`Gate::in_place`] leads, because a way round demanding several things a player can meet where
/// they stand is still a way in and one far-side lock is not, however many fewer steps it takes.
fn price(conditions: Gate, demands: u32) -> (bool, u32) {
    (!conditions.in_place(), demands)
}

/// At most `want` ways from `from` to `to`, by what each [`price`] costs a reader and shorter with
/// every class: the trade the reader is being offered.
///
/// A class is only offered where it beats every way of a freer one, so a way that demands more
/// without saving a connection is no alternative. A class holds as many ways as it demands things,
/// a reader being likelier to fall short of a demanding class and so to want a second way of the
/// same price. Ways that back out through an exit point are tallied apart and held to one a class:
/// an exit point is a yes or no made on the spot, so several would crowd out the ways round
/// without offering a further choice.
///
/// What every [`Passing`] level finds, pooled and then read once for what a way actually demands
/// rather than for the level that found it. A few hundred whole-graph searches, so it is run where
/// a reader wants the ways rather than wherever they are read.
pub fn ways(
    connections: &[Vec<Step>],
    from: usize,
    to: usize,
    want: usize,
    hub: Option<usize>,
) -> Vec<Way> {
    let mut found: Vec<Vec<usize>> = Vec::new();
    for passing in Passing::ALL {
        for walk in ways_between(connections, from, to, passing.met(), want, hub) {
            if !found.contains(&walk) {
                found.push(walk);
            }
        }
    }
    // Cheapest first, and the shortest of each class ahead of the rest of it. The harshest gate
    // only separates two of a class that are the same length, so a reader offered one of them is
    // offered the gentler.
    found.sort_by_key(|walk| {
        let (conditions, demands) = conditions_of(connections, hub, walk);
        (price(conditions, demands), walk.len(), conditions)
    });

    let mut ways: Vec<Way> = Vec::new();
    for walk in found {
        if ways.len() == want {
            break;
        }
        let (conditions, demands) = conditions_of(connections, hub, &walk);
        let class = price(conditions, demands);
        let backs_out = backs_out(connections, hub, &walk);
        let kept = ways
            .iter()
            .filter(|way| price(way.conditions, way.demands) == class && way.backs_out == backs_out)
            .count();
        let room = kept
            < if backs_out {
                1
            } else {
                demands.max(1) as usize
            };
        let beats = ways
            .iter()
            .all(|way| price(way.conditions, way.demands) >= class || walk.len() < way.walk.len());
        if !room || !beats {
            continue;
        }
        ways.push(Way {
            walk,
            conditions,
            demands,
            backs_out,
        });
    }
    ways
}
