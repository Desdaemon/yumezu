//! The tree of routes a player can traverse, outward from a world or inward to one.

use super::{Conditions, Demand, ESCAPE_EFFECT, Gate, Step};

/// Every world's route to or from one seed: the start for [`routes_from`], the destination for
/// [`routes_toward`].
pub struct Routes {
    /// Per world, the next world toward the seed; `None` for the seed and for worlds out of reach.
    pub parents: Vec<Option<usize>>,
    /// Per world, steps between the world and the seed; `None` for worlds out of reach.
    pub depth: Vec<Option<u32>>,
}

impl Routes {
    /// Per world, the number of worlds whose route to the seed passes through that world.
    pub fn descendant_counts(&self) -> Vec<u32> {
        // Deepest first, so descendants are counted before their parent.
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

    /// `root` and every world routed through `root`, shallowest first.
    pub fn subtree(&self, root: usize) -> Vec<usize> {
        // Shallowest first, so a parent is decided first; worlds out of reach lead, parentless.
        let mut order: Vec<usize> = (0..self.parents.len()).collect();
        order.sort_unstable_by_key(|&world| self.depth[world]);
        let mut in_subtree = vec![false; self.parents.len()];
        in_subtree[root] = true;
        for &world in &order {
            if let Some(parent) = self.parents[world] {
                in_subtree[world] |= in_subtree[parent];
            }
        }
        order.retain(|&world| in_subtree[world]);
        order
    }

    /// The worlds from `from` to the seed, `from` first; empty when `from` is out of reach.
    pub fn route_from(&self, from: usize) -> Vec<usize> {
        let mut route = Vec::new();
        let mut step = self.depth[from].is_some().then_some(from);
        while let Some(world) = step {
            route.push(world);
            step = self.parents[world];
        }
        route
    }
}

/// Routes outward from `seed`, ordered by the most restrictive gate along them, then length.
/// A tree holds one route per world: the best one through the world's parent.
pub fn routes_from(connections: &[Vec<Step>], seed: usize) -> Routes {
    routes_from_unlocked(connections, seed, Gate::Free)
}

/// [`routes_from`] for a player who has unlocked every gate up to `unlocked`.
/// Those gates demand nothing; at [`Gate::Revisit`] routes are shortest by steps.
pub fn routes_from_unlocked(connections: &[Vec<Step>], origin: usize, unlocked: Gate) -> Routes {
    route_tree(connections, origin, None, move |_, step| {
        RouteStep::outgoing(step, unlocked)
    })
}

/// Routes inward to `destination`: per world, the next world toward `destination` and the
/// steps left.
pub fn routes_toward(connections: &[Vec<Step>], destination: usize) -> Routes {
    // A step into an isolated section only ends a route, so only `destination` accepts one.
    route_tree(connections, destination, None, |at, step| {
        let gate = step.incoming.as_ref()?.gate;
        (gate.passable() || at == destination).then_some(RouteStep {
            world: step.world,
            gate,
            passable: true,
        })
    })
}

/// One step a route search may take.
struct RouteStep {
    world: usize,
    /// The step's gate; routes are ordered by the most restrictive gate along them.
    gate: Gate,
    /// Whether a route can continue past this step.
    passable: bool,
}

impl RouteStep {
    /// `step` taken outward past every gate up to `unlocked`; `None` with no exit.
    fn outgoing(step: &Step, unlocked: Gate) -> Option<Self> {
        let gate = step.outgoing.as_ref()?.gate;
        Some(Self {
            world: step.world,
            gate: unmet_gate(gate, unlocked),
            passable: gate.passable(),
        })
    }
}

/// Dijkstra from `seed`; `take_step` says which steps a route may take, and at what gate.
/// With a `hub`, every other world gains an escape step to the hub.
fn route_tree(
    connections: &[Vec<Step>],
    seed: usize,
    hub: Option<usize>,
    take_step: impl Fn(usize, &Step) -> Option<RouteStep>,
) -> Routes {
    let mut routes = Routes {
        parents: vec![None; connections.len()],
        depth: vec![None; connections.len()],
    };
    if seed >= connections.len() {
        return routes;
    }

    // Dijkstra over (gate, at_dead_end, depth, demands); world and parent break ties.
    // `at_dead_end` outranks depth, so a world settles on an arrival a route can continue from.
    let escape_to_hub = hub.map(escape_step);
    let mut queue = std::collections::BinaryHeap::from([std::cmp::Reverse((
        Gate::Free,
        false,
        0,
        0,
        seed,
        seed,
    ))]);
    while let Some(std::cmp::Reverse((gate, at_dead_end, depth, demands, world, parent))) =
        queue.pop()
    {
        if routes.depth[world].is_some() {
            continue;
        }
        routes.depth[world] = Some(depth);
        routes.parents[world] = (world != seed).then_some(parent);
        let escape_here = escape_to_hub.iter().filter(|_| Some(world) != hub);
        for step in connections[world].iter().chain(escape_here) {
            let Some(taken) = take_step(world, step) else {
                continue;
            };
            // The wiki writes a pocket's exit as another pocket.
            if at_dead_end && taken.passable {
                continue;
            }
            if routes.depth[taken.world].is_none() {
                queue.push(std::cmp::Reverse((
                    gate.max(taken.gate),
                    !taken.passable,
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

/// `gate`, or [`Gate::Free`] when `unlocked` covers `gate`.
fn unmet_gate(gate: Gate, unlocked: Gate) -> Gate {
    match gate <= unlocked {
        true => Gate::Free,
        false => gate,
    }
}

fn escape_conditions() -> Conditions {
    Conditions::from_demands([Demand {
        gate: Gate::Effect,
        detail: Some(ESCAPE_EFFECT.to_owned()),
    }])
}

/// The one-way escape step to the [`super::HUB`], added to every other world.
fn escape_step(hub: usize) -> Step {
    Step {
        world: hub,
        outgoing: Some(escape_conditions()),
        incoming: None,
    }
}

/// The step `from` lists toward `to`, if they are joined.
pub fn step_between(connections: &[Vec<Step>], from: usize, to: usize) -> Option<&Step> {
    connections[from].iter().find(|step| step.world == to)
}

/// Conditions of the step from `from` to `to`, the escape to `hub` included; `None` with no step.
pub fn step_conditions(
    connections: &[Vec<Step>],
    hub: Option<usize>,
    from: usize,
    to: usize,
) -> Option<Conditions> {
    let listed = step_between(connections, from, to).and_then(|step| step.outgoing.clone());
    listed.or_else(|| (hub == Some(to) && hub != Some(from)).then(escape_conditions))
}

/// Whether any step along `way` is a [`Gate::ExitPoint`].
fn through_exit_point(connections: &[Vec<Step>], hub: Option<usize>, way: &[usize]) -> bool {
    way.windows(2).any(|pair| {
        step_conditions(connections, hub, pair[0], pair[1])
            .is_some_and(|step| step.gate == Gate::ExitPoint)
    })
}

/// The most restrictive gate along `way` and the count of steps that demand anything.
fn demands_along(connections: &[Vec<Step>], hub: Option<usize>, way: &[usize]) -> (Gate, u32) {
    demands_along_unlocked(connections, hub, way, Gate::Free)
}

fn demands_along_unlocked(
    connections: &[Vec<Step>],
    hub: Option<usize>,
    way: &[usize],
    unlocked: Gate,
) -> (Gate, u32) {
    way.windows(2)
        .map(|pair| {
            step_conditions(connections, hub, pair[0], pair[1])
                .map_or(Gate::Free, |step| unmet_gate(step.gate, unlocked))
        })
        .fold((Gate::Free, 0), |(harshest, demands), conditions| {
            (
                harshest.max(conditions),
                demands + u32::from(conditions != Gate::Free),
            )
        })
}

/// Up to `max_ways` loopless ways from `from` to `to`, best first, by Yen's algorithm.
/// `unlocked` orders the ways as in [`routes_from_unlocked`] and so decides which are found.
fn ways_between(
    connections: &[Vec<Step>],
    from: usize,
    to: usize,
    unlocked: Gate,
    max_ways: usize,
    hub: Option<usize>,
) -> Vec<Vec<usize>> {
    let best_way = |spur: usize,
                    excluded: &[bool],
                    refused: &std::collections::HashSet<(usize, usize)>|
     -> Option<Vec<usize>> {
        let routes = route_tree(connections, spur, hub, |at, step| {
            if excluded[step.world] || refused.contains(&(at, step.world)) {
                return None;
            }
            RouteStep::outgoing(step, unlocked)
        });
        routes.depth[to]?;
        let mut way = routes.route_from(to);
        way.reverse();
        Some(way)
    };

    let none_excluded = vec![false; connections.len()];
    let Some(best) = best_way(from, &none_excluded, &Default::default()) else {
        return Vec::new();
    };
    let mut found = vec![best];
    let mut candidates: Vec<Vec<usize>> = Vec::new();
    while found.len() < max_ways {
        let latest = found[found.len() - 1].clone();
        for at in 0..latest.len() - 1 {
            let root = &latest[..=at];
            // Steps already taken from here by the same prefix, so a candidate must diverge.
            let refused: std::collections::HashSet<(usize, usize)> = found
                .iter()
                .chain(&candidates)
                .filter(|way| way.len() > at + 1 && way[..=at] == *root)
                .map(|way| (way[at], way[at + 1]))
                .collect();
            let mut excluded = none_excluded.clone();
            for &world in &root[..at] {
                excluded[world] = true;
            }
            let Some(spur) = best_way(root[at], &excluded, &refused) else {
                continue;
            };
            let candidate: Vec<usize> = root[..at].iter().copied().chain(spur).collect();
            if !found.contains(&candidate) && !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
        let best = candidates.iter().enumerate().min_by_key(|(_, way)| {
            let (conditions, demands) = demands_along_unlocked(connections, hub, way, unlocked);
            (conditions, way.len(), demands)
        });
        let Some((best, _)) = best else {
            break;
        };
        found.push(candidates.swap_remove(best));
    }
    found
}

/// The gates a way may assume a player has unlocked; [`ways`] searches at every level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UnlockLevel {
    Nothing,
    /// Every gate a player can meet in place.
    Conditions,
    Everything,
}

impl UnlockLevel {
    const ALL: [Self; 3] = [Self::Nothing, Self::Conditions, Self::Everything];

    /// The most restrictive gate this level treats as demanding nothing.
    fn most_restrictive_unlocked(self) -> Gate {
        match self {
            Self::Nothing => Gate::Free,
            Self::Conditions => Gate::LockedCondition,
            Self::Everything => Gate::Revisit,
        }
    }
}

/// One way between two worlds, with the way's demands.
pub struct Way {
    /// The worlds along the way, `from` first.
    pub worlds: Vec<usize>,
    /// The most restrictive gate along the way.
    pub gate: Gate,
    /// How many steps along the way demand anything.
    pub demanding_steps: u32,
    /// Whether any step along the way is a [`Gate::ExitPoint`].
    pub through_exit_point: bool,
}

/// A way's class: whether any gate needs the far side, then the count of demanding steps.
fn way_class(gate: Gate, demanding_steps: u32) -> (bool, u32) {
    (!gate.passable_without_unlocks(), demanding_steps)
}

/// At most `max_ways` ways from `from` to `to`; a harsher class must be shorter to be offered.
/// Per class: as many ways as demanding steps, at least one, plus one through an exit point.
pub fn ways(
    connections: &[Vec<Step>],
    from: usize,
    to: usize,
    max_ways: usize,
    hub: Option<usize>,
) -> Vec<Way> {
    let mut found: Vec<Vec<usize>> = Vec::new();
    for level in UnlockLevel::ALL {
        let unlocked = level.most_restrictive_unlocked();
        for way in ways_between(connections, from, to, unlocked, max_ways, hub) {
            if !found.contains(&way) {
                found.push(way);
            }
        }
    }
    // Cheapest class first, then shortest; the most restrictive gate breaks length ties.
    found.sort_by_key(|way| {
        let (gate, demanding_steps) = demands_along(connections, hub, way);
        (way_class(gate, demanding_steps), way.len(), gate)
    });

    let mut ways: Vec<Way> = Vec::new();
    for way in found {
        if ways.len() == max_ways {
            break;
        }
        let (gate, demanding_steps) = demands_along(connections, hub, &way);
        let class = way_class(gate, demanding_steps);
        let through_exit_point = through_exit_point(connections, hub, &way);
        let same_class = ways
            .iter()
            .filter(|earlier_way| {
                way_class(earlier_way.gate, earlier_way.demanding_steps) == class
                    && earlier_way.through_exit_point == through_exit_point
            })
            .count();
        let has_room = same_class
            < if through_exit_point {
                1
            } else {
                demanding_steps.max(1) as usize
            };
        let shorter_than_freer = ways.iter().all(|earlier_way| {
            way_class(earlier_way.gate, earlier_way.demanding_steps) >= class
                || way.len() < earlier_way.worlds.len()
        });
        if !has_room || !shorter_than_freer {
            continue;
        }
        ways.push(Way {
            worlds: way,
            gate,
            demanding_steps,
            through_exit_point,
        });
    }
    ways
}
