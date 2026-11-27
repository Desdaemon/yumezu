//! The dump's `depth` (steps from the start without conditions) and `minDepth` (with conditions).
//! Worlds out of reach are measured by allowing increasingly restrictive connections one class at
//! a time.

use crate::model::ConnType;

pub(crate) use yumezu_routing::START;

pub struct Node {
    pub title: String,
    /// A world the game no longer has. Routes may enter a removed world but never leave one, so a
    /// removed world cannot shorten a live world's distance.
    pub removed: bool,
    /// `(destination index, connection flags)` for every connection out of this world.
    pub outgoing: Vec<(usize, ConnType)>,
}

/// Conditions `depth` refuses: a connection with any of these is not unconditional.
fn refused_by_depth() -> ConnType {
    ConnType::NO_ENTRY
        | ConnType::LOCKED
        | ConnType::DEAD_END
        | ConnType::ISOLATED
        | ConnType::LOCKED_CONDITION
        | ConnType::EXIT_POINT
}

/// Conditions `minDepth` refuses: connections that lead somewhere other than the written
/// destination, as a way back or into an isolated pocket.
fn refused_by_min_depth() -> ConnType {
    ConnType::NO_ENTRY | ConnType::DEAD_END | ConnType::ISOLATED
}

/// `(depth, minDepth)` for every world, in input order.
pub fn depths_from_start(worlds: &[Node]) -> Vec<(i32, i32)> {
    let depths = steps_from_start(worlds, refused_by_depth());
    let min_depths = steps_from_start(worlds, refused_by_min_depth());
    depths.into_iter().zip(min_depths).collect()
}

/// Steps from [`START`] with `refused` connections skipped; while a world is out of reach, the next
/// class is allowed.
fn steps_from_start(worlds: &[Node], refused: ConnType) -> Vec<i32> {
    let mut steps: Vec<Option<i32>> = vec![None; worlds.len()];
    let mut via_removed: Vec<bool> = vec![false; worlds.len()];

    let Some(start) = worlds.iter().position(|world| world.title == START) else {
        tracing::warn!("no world called {START}: every distance will be a guess");
        return vec![1; worlds.len()];
    };
    steps[start] = Some(0);

    extend_steps(worlds, refused, &mut steps, &mut via_removed, false);

    let mut refused = refused;
    while !steps.iter().all(Option::is_some) {
        let Some(allowed) = allow_next_restriction(refused) else {
            break;
        };
        refused = allowed;
        extend_steps(worlds, refused, &mut steps, &mut via_removed, true);
    }

    // Worlds out of reach get 1: present in the game, so not the start.
    steps.into_iter().map(|step| step.unwrap_or(1)).collect()
}

/// `refused` minus the next class to allow, least restrictive first, in the reference
/// implementation's order; `None` once nothing is refused.
fn allow_next_restriction(refused: ConnType) -> Option<ConnType> {
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

/// Dijkstra from every measured world; with `keep_measured`, measured distances are final and only
/// unmeasured worlds are added. A heap, since later passes seed from every distance at once.
fn extend_steps(
    worlds: &[Node],
    refused: ConnType,
    steps: &mut [Option<i32>],
    via_removed: &mut [bool],
    keep_measured: bool,
) {
    let measured: Vec<bool> = match keep_measured {
        true => steps.iter().map(Option::is_some).collect(),
        false => vec![false; worlds.len()],
    };
    let mut queue: std::collections::BinaryHeap<std::cmp::Reverse<(i32, usize)>> = steps
        .iter()
        .enumerate()
        .filter_map(|(world, step)| Some(std::cmp::Reverse(((*step)?, world))))
        .collect();

    while let Some(std::cmp::Reverse((here, from))) = queue.pop() {
        // Stale heap entry.
        if steps[from] != Some(here) {
            continue;
        }
        let from_removed = via_removed[from];
        for &(to, flags) in &worlds[from].outgoing {
            if flags.intersects(refused) {
                continue;
            }
            let into_removed = worlds[to].removed;
            // A route through a removed world stays among removed worlds; a live route skips
            // inaccessible connections.
            if from_removed && !into_removed {
                continue;
            }
            if !from_removed && !into_removed && flags.contains(ConnType::INACCESSIBLE) {
                continue;
            }
            if measured[to] {
                continue;
            }
            if steps[to].is_none_or(|there| there > here + 1) {
                steps[to] = Some(here + 1);
                via_removed[to] = from_removed || into_removed;
                queue.push(std::cmp::Reverse((here + 1, to)));
            }
        }
    }
}
