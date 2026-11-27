//! Scales in the worlds a rebuilt graph adds and carries over those both graphs share. See
//! [`GraphGraft`] and [`resume_from`].

use super::*;

/// How long one arriving world takes to grow from nothing to full size.
const ARRIVAL_SECONDS: f32 = 0.45;
/// The longest span the arrival stagger takes, however many worlds arrive; see [`GraphGraft::new`].
const ARRIVAL_WINDOW: f32 = 1.6;
/// The delay between successive arriving worlds, shortened to fit [`ARRIVAL_WINDOW`].
const ARRIVAL_STAGGER: f32 = 0.05;
/// How long the camera keeps framing the arrived worlds after the last finishes growing, while
/// they still push their neighbours around a settled layout.
const ARRIVAL_TRACKED_SECONDS: f32 = 4.0;

/// World positions by name, so the next graph starts from them instead of a fresh scatter. Keyed
/// by name because a frontier renumbers worlds from zero, so indices differ across rebuilds.
pub(super) type Standing = std::collections::HashMap<String, [f32; 3]>;

/// What the new graph carries on from the graph being replaced: the user's view of the graph, so a
/// rebuild does not look like a restart. See [`resume_from`] and [`GraphGraft`].
pub(super) struct Before {
    pub(super) standing: Standing,
    /// Worlds drawn as placeholders, by name: the new graph alone cannot tell a newly visited world
    /// from one visited all along. See [`GraftKind::Swapped`].
    pub(super) unvisited: std::collections::HashSet<String>,
    /// Kept across a refresh, which shows the same map with more worlds known.
    pub(super) layout: Layout,
}

/// How one world enters a rebuilt graph.
#[derive(Clone, Copy)]
enum GraftKind {
    /// Present before and unchanged, as nearly every world is: drawn at full size from the first
    /// frame.
    Unchanged,
    /// New to the graph. Grows in from nothing this many seconds after the build.
    Grafted(f32),
    /// A placeholder since visited. The placeholder shrinks away and the world grows back, so the
    /// swap happens at zero size. See [`GraphGraft::veiled`].
    Swapped(f32),
}

/// The worlds a rebuilt graph grafts on and how far each has grown, so a refresh animates in.
///
/// Empty for the first graph of a session and once the arrival ends, costing other frames nothing.
#[derive(Default)]
pub(super) struct GraphGraft {
    /// Per world, how the world joins the graph; `None` when nothing is arriving.
    kinds: Option<Vec<GraftKind>>,
    standing_seconds: f32,
    /// When the last world starts growing to full size.
    last: f32,
}

impl GraphGraft {
    /// `before` is the replaced graph; `None`, on a first build, means nothing arrives. `concealed`
    /// marks the current placeholders, compared with [`Before::unvisited`] to find newly visited
    /// worlds.
    ///
    /// Placeholder swaps go first, then new worlds follow by depth, so they spread outward.
    pub(super) fn new(
        names: &[&str],
        concealed: &[bool],
        depth: &[Option<u32>],
        before: Option<&Before>,
    ) -> Self {
        let Some(before) = before else {
            return Self::default();
        };
        let mut kinds = vec![GraftKind::Unchanged; names.len()];
        let mut arriving = Vec::new();
        let mut turning = Vec::new();
        for (world, &name) in names.iter().enumerate() {
            match before.standing.contains_key(name) {
                false => arriving.push(world),
                true if before.unvisited.contains(name) && !concealed[world] => turning.push(world),
                true => {}
            }
        }
        if arriving.is_empty() && turning.is_empty() {
            return Self::default();
        }
        for &world in &turning {
            kinds[world] = GraftKind::Swapped(0.0);
        }
        // New worlds start once the placeholders have shrunk away.
        let after = if turning.is_empty() {
            0.0
        } else {
            ARRIVAL_SECONDS
        };
        arriving.sort_by_key(|&world| depth[world].unwrap_or(u32::MAX));
        let stagger = ARRIVAL_STAGGER.min(ARRIVAL_WINDOW / arriving.len().max(1) as f32);
        for (nth, &world) in arriving.iter().enumerate() {
            kinds[world] = GraftKind::Grafted(after + nth as f32 * stagger);
        }
        Self {
            // A placeholder swap takes two of these; the first new world starts at the second.
            last: match arriving.is_empty() {
                true => ARRIVAL_SECONDS,
                false => after + (arriving.len() - 1) as f32 * stagger,
            },
            kinds: Some(kinds),
            standing_seconds: 0.0,
        }
    }

    /// The fraction of full size a world is drawn at: 0 before due, 1 once arrived, and 0 again at
    /// the moment of a placeholder swap.
    pub(super) fn grown(&self, world: usize) -> f32 {
        let Some(kinds) = &self.kinds else {
            return 1.0;
        };
        match kinds[world] {
            GraftKind::Unchanged => 1.0,
            GraftKind::Grafted(due) => eased((self.standing_seconds - due) / ARRIVAL_SECONDS),
            GraftKind::Swapped(due) => {
                let through = (self.standing_seconds - due) / ARRIVAL_SECONDS;
                match through < 1.0 {
                    true => 1.0 - eased(through),
                    false => eased(through - 1.0),
                }
            }
        }
    }

    /// Newly visited worlds whose placeholder has not yet shrunk away. The placeholder is drawn
    /// over the world's quad, so hiding the placeholder is the whole swap.
    pub(super) fn veiled(&self) -> impl Iterator<Item = usize> + '_ {
        self.kinds
            .iter()
            .flatten()
            .enumerate()
            .filter_map(|(world, kind)| {
                matches!(kind, GraftKind::Swapped(due) if self.standing_seconds < due + ARRIVAL_SECONDS)
                    .then_some(world)
            })
    }

    /// Whether a placeholder swap is under way, which needs the placeholder picture even in a graph
    /// with no unvisited world.
    pub(super) fn turning(&self) -> bool {
        self.kinds
            .iter()
            .flatten()
            .any(|kind| matches!(kind, GraftKind::Swapped(_)))
    }

    /// The arriving worlds, or `None` when nothing is arriving. Lasts past the growth while the
    /// camera frames them: see [`ARRIVAL_TRACKED_SECONDS`] and [`AppEntities::arrival_bounds`].
    pub(super) fn arriving(&self) -> Option<impl Iterator<Item = usize> + '_> {
        let kinds = self.kinds.as_ref()?;
        Some(
            kinds
                .iter()
                .enumerate()
                .filter(|(_, kind)| !matches!(kind, GraftKind::Unchanged))
                .map(|(world, _)| world),
        )
    }

    /// Answers whether a world is still growing, which forces a geometry
    /// rebuild on a still layout. The arrival ends [`ARRIVAL_TRACKED_SECONDS`] later.
    pub(super) fn update(&mut self, dt: f32) -> bool {
        if self.kinds.is_none() {
            return false;
        }
        self.standing_seconds += dt;
        let grown = self.last + ARRIVAL_SECONDS;
        if self.standing_seconds > grown + ARRIVAL_TRACKED_SECONDS {
            *self = Self::default();
            return false;
        }
        self.standing_seconds <= grown
    }
}

/// Smoothstep, clamped so a world is zero before due and full size once grown.
fn eased(through: f32) -> f32 {
    let through = through.clamp(0.0, 1.0);
    through * through * (3.0 - 2.0 * through)
}

/// Puts the worlds shared with the previous graph back where they were, at rest, and new worlds at
/// the mean of their placed neighbours; a world with none keeps the scatter.
pub(super) fn resume_from(data: &mut AppEntities, before: &Standing) {
    let standing: Vec<Option<[f32; 3]>> = data
        .titles
        .iter()
        .map(|title| before.get(&title.en).copied())
        .collect();
    // Seeded from the old positions, so new worlds enter beside their connections.
    let seeded: Vec<Option<[f32; 3]>> = (0..standing.len())
        .map(|world| {
            if standing[world].is_some() {
                return standing[world];
            }
            let near: Vec<[f32; 3]> = data.connections[world]
                .iter()
                .filter_map(|step| standing[step.world])
                .collect();
            if near.is_empty() {
                return None;
            }
            let mut at = [0.0; 3];
            for position in &near {
                for (at, value) in at.iter_mut().zip(position) {
                    *at += value / near.len() as f32;
                }
            }
            Some(at)
        })
        .collect();
    // Slot order is world order: one node per world, never removed.
    let mut world = 0;
    data.graph.visit_nodes_mut(|mut node| {
        if let Some(at) = seeded[world] {
            node.set_position(at);
            node.set_velocity([0.0; 3]);
        }
        world += 1;
    });
}

#[cfg(test)]
mod tests {
    #[cfg(target_family = "wasm")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    /// A graph the user was already viewing, with the `unvisited` worlds drawn as placeholders.
    fn before(standing: &[&str], unvisited: &[&str]) -> super::Before {
        super::Before {
            standing: standing
                .iter()
                .map(|name| ((*name).to_owned(), [0.0; 3]))
                .collect(),
            unvisited: unvisited.iter().map(|name| (*name).to_owned()).collect(),
            layout: super::Layout::default(),
        }
    }
    #[test]
    fn worlds_the_graph_before_did_not_have_arrive_one_after_another() {
        let before = before(&["Nexus"], &[]);
        let mut graft = super::GraphGraft::new(
            &["Nexus", "The Nexus Void", "Marijuana Goddess World"],
            &[false; 3],
            &[Some(0), Some(1), Some(2)],
            Some(&before),
        );
        assert_eq!(graft.grown(0), 1.0);
        assert_eq!(graft.grown(1), 0.0);
        assert!(graft.update(super::ARRIVAL_STAGGER * 0.5));
        assert!(graft.grown(1) > 0.0 && graft.grown(1) < 1.0);
        assert_eq!(graft.grown(2), 0.0);
        assert!(!graft.update(super::ARRIVAL_SECONDS + super::ARRIVAL_WINDOW));
        assert_eq!(graft.grown(2), 1.0);
    }

    #[test]
    fn a_world_that_has_become_known_shrinks_away_and_grows_back() {
        let before = before(&["Nexus", "Sugar Hole"], &["Sugar Hole"]);
        let mut graft = super::GraphGraft::new(
            &["Nexus", "Sugar Hole"],
            &[false, false],
            &[Some(0), Some(1)],
            Some(&before),
        );
        assert_eq!(graft.grown(1), 1.0);
        assert_eq!(graft.veiled().collect::<Vec<_>>(), vec![1]);
        graft.update(super::ARRIVAL_SECONDS * 0.5);
        assert!(graft.grown(1) > 0.0 && graft.grown(1) < 1.0);
        assert_eq!(graft.veiled().collect::<Vec<_>>(), vec![1]);
        graft.update(super::ARRIVAL_SECONDS);
        assert!(graft.grown(1) > 0.0 && graft.grown(1) < 1.0);
        assert!(graft.veiled().next().is_none());
        assert!(!graft.update(super::ARRIVAL_SECONDS));
        assert_eq!(graft.grown(1), 1.0);
    }

    #[test]
    fn a_world_still_unvisited_is_not_turned_over() {
        let before = before(&["Nexus", "Sugar Hole"], &["Sugar Hole"]);
        let graft = super::GraphGraft::new(
            &["Nexus", "Sugar Hole"],
            &[false, true],
            &[Some(0), Some(1)],
            Some(&before),
        );
        assert!(!graft.turning());
        assert!(graft.arriving().is_none());
    }

    #[test]
    fn what_arrived_is_still_framed_after_it_has_finished_growing() {
        let before = before(&["Nexus"], &[]);
        let mut graft = super::GraphGraft::new(
            &["Nexus", "Sugar Hole"],
            &[false; 2],
            &[Some(0), Some(1)],
            Some(&before),
        );
        assert!(!graft.update(super::ARRIVAL_SECONDS + 0.1));
        assert_eq!(graft.grown(1), 1.0);
        assert_eq!(graft.arriving().unwrap().collect::<Vec<_>>(), vec![1]);
        assert!(!graft.update(super::ARRIVAL_TRACKED_SECONDS));
        assert!(graft.arriving().is_none());
    }

    #[test]
    fn a_graph_with_nothing_new_in_it_animates_nothing() {
        for standing in [None, Some(before(&["Nexus"], &[]))] {
            let mut graft =
                super::GraphGraft::new(&["Nexus"], &[false], &[Some(0)], standing.as_ref());
            assert_eq!(graft.grown(0), 1.0);
            assert!(!graft.update(0.016));
        }
    }

    #[test]
    fn a_great_many_worlds_arriving_still_arrive_within_the_window() {
        let names: Vec<String> = (0..1500).map(|world| world.to_string()).collect();
        let graft = super::GraphGraft::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
            &vec![false; names.len()],
            &vec![Some(0); names.len()],
            Some(&before(&[], &[])),
        );
        assert!(graft.last <= super::ARRIVAL_WINDOW);
    }
}
