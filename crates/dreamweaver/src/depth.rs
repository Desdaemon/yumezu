//! How far each world is from the one the game starts in.
//!
//! Two numbers differing only in which passages they will walk. `depth` counts steps along
//! passages a player can simply take; `minDepth` takes any passage that exists at all, so it says
//! how deep the world sits at best. The reader draws the graph by the first and ranks by the second.
//!
//! Neither is a plain shortest path, because the graph is not connected under either rule: whole
//! branches hang off passages locked from both ends. A run that leaves worlds unreached gives up
//! one condition at a time, weakest first, and tries again from what it already knows.

use crate::model::ConnType;

/// The world the game starts in, and so the world every distance is measured from.
const START: &str = "Urotsuki's Room";

pub struct Node {
    pub title: String,
    /// A world the game no longer has. A route may walk into one but never back out, so a removed
    /// world cannot shorten a live world's distance.
    pub removed: bool,
    /// `(index of the world it leads to, what the passage is like)`.
    pub out: Vec<(usize, ConnType)>,
}

/// The conditions `depth` refuses: a passage that asks for any of them is not a step a player can
/// simply take.
fn walkable() -> ConnType {
    ConnType::NO_ENTRY
        | ConnType::LOCKED
        | ConnType::DEAD_END
        | ConnType::ISOLATED
        | ConnType::LOCKED_CONDITION
        | ConnType::EXIT_POINT
}

/// Only the conditions meaning the passage does not lead where it is written: a way back rather
/// than a way there, or a way into a pocket of the destination that connects to nothing else.
fn reachable() -> ConnType {
    ConnType::NO_ENTRY | ConnType::DEAD_END | ConnType::ISOLATED
}

/// `(depth, minDepth)` for every world, in the order they were given.
pub fn of(worlds: &[Node]) -> Vec<(i32, i32)> {
    let deep = distances(worlds, walkable());
    let shallow = distances(worlds, reachable());
    deep.into_iter().zip(shallow).collect()
}

/// Distances from [`START`] under one set of refused conditions, giving conditions up until
/// nothing more can be reached.
fn distances(worlds: &[Node], refused: ConnType) -> Vec<i32> {
    let mut depth: Vec<Option<i32>> = vec![None; worlds.len()];
    // How a world was reached decides what can be reached from it: a route inside the removed
    // part of the graph may not walk back out.
    let mut through_removed: Vec<bool> = vec![false; worlds.len()];

    let Some(start) = worlds.iter().position(|world| world.title == START) else {
        // Nothing to measure from; every world ends up at the fallback distance below.
        tracing::warn!("no world called {START}: every distance will be a guess");
        return vec![1; worlds.len()];
    };
    depth[start] = Some(0);

    // Everything reached here is measured under the conditions asked for.
    relax(worlds, refused, &mut depth, &mut through_removed, false);

    let mut refused = refused;
    while !depth.iter().all(Option::is_some) {
        let Some(weaker) = give_up(refused) else {
            // Every condition given up and worlds still unreached: they are genuinely not joined
            // to the rest of the graph.
            break;
        };
        refused = weaker;
        // A world already measured keeps the distance the stricter pass gave it: giving up a
        // condition reaches what could not be reached at all, it does not discount everything
        // else. A world four honest steps in does not become three because a locked door
        // somewhere would have been a shortcut.
        relax(worlds, refused, &mut depth, &mut through_removed, true);
    }

    // A world nothing leads to is put one step in rather than at the start: it exists, so it is
    // not where the player begins, and any further guess would be invented.
    depth.into_iter().map(|d| d.unwrap_or(1)).collect()
}

/// The next condition to stop refusing, weakest first.
///
/// The reference implementation's order, and one of confidence rather than difficulty: a
/// conditional passage is the most likely to be walkable in practice, a no-entry passage the least,
/// since walking one means going the way the wiki says you cannot.
fn give_up(refused: ConnType) -> Option<ConnType> {
    for condition in [
        ConnType::LOCKED_CONDITION,
        ConnType::LOCKED,
        ConnType::EXIT_POINT,
        ConnType::DEAD_END | ConnType::ISOLATED,
        ConnType::NO_ENTRY,
    ] {
        if refused.intersects(condition) {
            return Some(refused.difference(condition));
        }
    }
    None
}

/// Nearest first out of a heap rather than a breadth-first sweep: every pass after the first starts
/// from seeds sitting at every distance at once, and taking the nearest each time is what makes the
/// distance a world is first given the shortest one it has -- which matters because `keep` forbids
/// improving it afterwards.
///
/// `keep` leaves the worlds that already have a distance exactly as they are, and is how a pass that
/// has given up a condition reaches further without rewriting what a stricter pass decided.
fn relax(
    worlds: &[Node],
    refused: ConnType,
    depth: &mut [Option<i32>],
    through_removed: &mut [bool],
    keep: bool,
) {
    let settled: Vec<bool> = match keep {
        true => depth.iter().map(Option::is_some).collect(),
        false => vec![false; worlds.len()],
    };
    let mut queue: std::collections::BinaryHeap<std::cmp::Reverse<(i32, usize)>> = depth
        .iter()
        .enumerate()
        .filter_map(|(world, d)| Some(std::cmp::Reverse(((*d)?, world))))
        .collect();

    while let Some(std::cmp::Reverse((here, from))) = queue.pop() {
        // A world reached again by a shorter route is in the heap twice; the longer entry is
        // stale by the time it comes up.
        if depth[from] != Some(here) {
            continue;
        }
        let removed_route = through_removed[from];
        for &(to, flags) in &worlds[from].out {
            if flags.intersects(refused) {
                continue;
            }
            let into_removed = worlds[to].removed;
            // Once a route is in the removed part of the graph it stays there, and one that is
            // not may not cross a removed passage to reach a world that still exists.
            if removed_route && !into_removed {
                continue;
            }
            if !removed_route && !into_removed && flags.contains(ConnType::INACCESSIBLE) {
                continue;
            }
            if settled[to] {
                continue;
            }
            if depth[to].is_none_or(|there| there > here + 1) {
                depth[to] = Some(here + 1);
                through_removed[to] = removed_route || into_removed;
                queue.push(std::cmp::Reverse((here + 1, to)));
            }
        }
    }
}
