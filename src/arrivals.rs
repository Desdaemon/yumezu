//! Growing in the worlds a rebuilt graph has that the graph before it had not, and carrying over
//! the ones it shares with it. See [`Arrivals`] and [`resume_from`].

use super::*;

/// How long one world takes to grow from nothing to its own size when it arrives.
const ARRIVAL_SECONDS: f32 = 0.45;
/// The longest a whole arrival takes, however many worlds are in it: two worlds should not take as
/// long as four hundred, and four hundred should not be watched for a minute. See [`Arrivals::new`].
const ARRIVAL_WINDOW: f32 = 1.6;
/// How far apart worlds arrive while [`ARRIVAL_WINDOW`] still allows it, which is what makes an
/// arrival a drip rather than a single pop.
const ARRIVAL_STAGGER: f32 = 0.05;
/// How long the camera goes on following what arrived after the last of it finished growing.
///
/// Worlds dropped into a settled layout go on pushing their neighbours around, so a camera handed
/// back the moment the last one reached full size would leave what it was showing to drift out of
/// frame. A pan or an orbit takes it back at any point.
const ARRIVAL_TRACKED_SECONDS: f32 = 4.0;

/// Where the worlds of a graph were standing, so the one built after it can start from there
/// instead of from a fresh scatter.
///
/// By name because that is the one thing two graphs of the same game share: a frontier is a part
/// of the dump renumbered from zero, so a world's index means nothing across a rebuild.
pub(super) type Standing = std::collections::HashMap<String, [f32; 3]>;

/// As much of the graph a new one is replacing as the new one should carry on from.
///
/// A rebuild is not a new run: the person is still looking at the same map and has only asked what
/// else is on it. Everything here was theirs rather than the dump's, and losing any of it would
/// read as the app having restarted. See [`resume_from`] and [`Arrivals`].
pub(super) struct Before {
    pub(super) standing: Standing,
    /// Which worlds were drawn as placeholders, by the same name. What a world has become is not
    /// readable from the new graph alone: a world with a picture may have had one all along. See
    /// [`Coming::Known`].
    pub(super) unvisited: std::collections::HashSet<String>,
    /// A refresh is the same map with more of it known, not a reason to put someone back in a view
    /// they left.
    pub(super) layout: Layout,
}

/// What one world is doing while a graph settles in around the worlds that were already there.
#[derive(Clone, Copy)]
enum Coming {
    /// Standing there and unchanged, which is nearly every world: drawn at its own size from the
    /// first frame, with nothing to wait for and nothing to animate.
    Already,
    /// Not in the graph before at all. Grows in from nothing this many seconds after it was built.
    New(f32),
    /// Was a placeholder and is a world now: the player has been somewhere they had only been
    /// shown the edge of. Shrinks away as the placeholder and grows back as itself, so the swap
    /// happens when there is nothing on screen to swap. See [`Arrivals::veiled`].
    Known(f32),
}

/// The worlds coming into a graph that was already standing, and how far in each of them is.
///
/// A refresh does not redraw the same graph: it builds another one, out of a dump cut back by an
/// account that has been somewhere new since. Left alone that lands as a jump -- a hundred worlds
/// where a moment ago there were none, and a placeholder that is suddenly a photograph.
///
/// `None` for the first graph of a run, where everything is simply there. That also keeps the cost
/// off every other frame: a graph with no arrivals has no vector, no clock, and nothing to ask.
#[derive(Default)]
pub(super) struct Arrivals {
    /// Per world, what it is doing. `None` where nothing is doing anything.
    coming: Option<Vec<Coming>>,
    /// How long the graph has been standing.
    clock: f32,
    /// When the last arrival finishes, which is when there stops being anything to wait for.
    last: f32,
}

impl Arrivals {
    /// `before` is the graph this one replaces, and `None` -- a first build -- means nothing is
    /// arriving. `unknown` says which worlds are placeholders now, which against
    /// [`Before::unvisited`] tells a world that has become known from one that was always either.
    ///
    /// The turning over goes first and the new worlds follow, because that is the order it
    /// happened in: the player walked into a world, and what lies past it is what that opened.
    /// Arrivals are ordered by depth, so they spread outward from what the player already had
    /// rather than speckling the graph, and spaced to fit [`ARRIVAL_WINDOW`] however many there
    /// are.
    pub(super) fn new(
        names: &[&str],
        unknown: &[bool],
        depth: &[Option<u32>],
        before: Option<&Before>,
    ) -> Self {
        let Some(before) = before else {
            return Self::default();
        };
        let mut coming = vec![Coming::Already; names.len()];
        let mut arriving = Vec::new();
        let mut turning = Vec::new();
        for (world, &name) in names.iter().enumerate() {
            match before.standing.contains_key(name) {
                false => arriving.push(world),
                // Both halves have to have changed: a world still a placeholder has not become
                // anything, and one that never was has nothing to turn over.
                true if before.unvisited.contains(name) && !unknown[world] => turning.push(world),
                true => {}
            }
        }
        if arriving.is_empty() && turning.is_empty() {
            return Self::default();
        }
        for &world in &turning {
            coming[world] = Coming::Known(0.0);
        }
        // Behind the turning over, so a new world comes out from under a placeholder that has
        // already gone rather than through one still standing.
        let after = if turning.is_empty() {
            0.0
        } else {
            ARRIVAL_SECONDS
        };
        arriving.sort_by_key(|&world| depth[world].unwrap_or(u32::MAX));
        let stagger = ARRIVAL_STAGGER.min(ARRIVAL_WINDOW / arriving.len().max(1) as f32);
        for (nth, &world) in arriving.iter().enumerate() {
            coming[world] = Coming::New(after + nth as f32 * stagger);
        }
        Self {
            // When the last arrival begins its final `ARRIVAL_SECONDS`. A turning over is two of
            // those, one each way, so it begins its second at `after` -- where the first new world
            // starts too, every one after that being later still.
            last: match arriving.is_empty() {
                true => ARRIVAL_SECONDS,
                false => after + (arriving.len() - 1) as f32 * stagger,
            },
            coming: Some(coming),
            clock: 0.0,
        }
    }

    /// How much of its own size a world is drawn at: nothing before it is due, all of it once it
    /// has arrived, and nothing again at the moment a world that has become known turns over.
    pub(super) fn grown(&self, world: usize) -> f32 {
        let Some(coming) = &self.coming else {
            return 1.0;
        };
        match coming[world] {
            Coming::Already => 1.0,
            Coming::New(due) => eased((self.clock - due) / ARRIVAL_SECONDS),
            Coming::Known(due) => {
                let through = (self.clock - due) / ARRIVAL_SECONDS;
                match through < 1.0 {
                    // Away as what it was, and back as what it is.
                    true => 1.0 - eased(through),
                    false => eased(through - 1.0),
                }
            }
        }
    }

    /// The worlds that have become known and not yet shrunk away.
    ///
    /// The placeholder is drawn over a world's own quad rather than instead of it, so this is all
    /// the swap takes: underneath, the picture has been the world's own all along, and it comes
    /// out from under the placeholder at the size where neither can be seen.
    pub(super) fn veiled(&self) -> impl Iterator<Item = usize> + '_ {
        self.coming
            .iter()
            .flatten()
            .enumerate()
            .filter_map(|(world, coming)| {
                matches!(coming, Coming::Known(due) if self.clock < due + ARRIVAL_SECONDS)
                    .then_some(world)
            })
    }

    /// What says the graph needs the placeholder picture at all even where nothing in it is
    /// unvisited.
    pub(super) fn turning(&self) -> bool {
        self.coming
            .iter()
            .flatten()
            .any(|coming| matches!(coming, Coming::Known(_)))
    }

    /// Which worlds moved, or `None` for a graph with nothing arriving into it.
    ///
    /// Answers past the end of the growing, for as long as the camera is still following them:
    /// see [`ARRIVAL_TRACKED_SECONDS`] and [`AppEntities::arrival_bounds`], which is what frames
    /// them.
    pub(super) fn arriving(&self) -> Option<impl Iterator<Item = usize> + '_> {
        let coming = self.coming.as_ref()?;
        Some(
            coming
                .iter()
                .enumerate()
                .filter(|(_, coming)| !matches!(coming, Coming::Already))
                .map(|(world, _)| world),
        )
    }

    /// Carries the arrival forward, answering whether anything is still moving -- which is what
    /// says the geometry has to be built again on a frame the layout did not move in.
    ///
    /// The camera goes on following for a while after that answer turns false, which is why this
    /// is not also what ends the arrival: see [`ARRIVAL_TRACKED_SECONDS`].
    pub(super) fn tick(&mut self, dt: f32) -> bool {
        if self.coming.is_none() {
            return false;
        }
        self.clock += dt;
        let grown = self.last + ARRIVAL_SECONDS;
        if self.clock > grown + ARRIVAL_TRACKED_SECONDS {
            // Dropped rather than left at rest: from here on this graph is one like any other,
            // and asking it about arrivals costs nothing again.
            *self = Self::default();
            return false;
        }
        self.clock <= grown
    }
}

/// Eased at both ends so a world neither snaps into existence nor overshoots the size it will
/// keep. Clamped, so a world is nothing before it is due and itself ever after.
fn eased(through: f32) -> f32 {
    let through = through.clamp(0.0, 1.0);
    through * through * (3.0 - 2.0 * through)
}

/// Puts the worlds this graph shares with the one before it back where they were standing, and
/// starts the new ones off among the neighbours they attach to.
///
/// Without this a refresh would reshuffle the whole graph to say that a handful of worlds had been
/// added, and the arrival would be lost in the churn. A returning world is stopped dead as well as
/// replaced, its velocity belonging to a layout that no longer exists.
///
/// A world with nothing to stand near keeps the scatter: nothing in the graph says where it
/// belongs until the layout has been stepped.
pub(super) fn resume_from(data: &mut AppEntities, before: &Standing) {
    let standing: Vec<Option<[f32; 3]>> = data
        .titles
        .iter()
        .map(|title| before.get(&title.en).copied())
        .collect();
    // New worlds are placed among the ones already there, so each comes in out of what it connects
    // to rather than from the far edge of the spawn volume. Read off the old positions rather than
    // the placements being made, so no world is seeded from another seed.
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
    // In slot order, which is world order: one node was added per world and none is ever removed.
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

    /// A graph the person was already looking at, with `unvisited` of its worlds wearing the
    /// placeholder.
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
        let mut arrivals = super::Arrivals::new(
            &["Nexus", "The Nexus Void", "Marijuana Goddess World"],
            &[false; 3],
            &[Some(0), Some(1), Some(2)],
            Some(&before),
        );
        // Already standing there, so it is drawn at its own size from the first frame.
        assert_eq!(arrivals.grown(0), 1.0);
        assert_eq!(arrivals.grown(1), 0.0);
        // Far enough in for the first new one to be part way and the second not yet due.
        assert!(arrivals.tick(super::ARRIVAL_STAGGER * 0.5));
        assert!(arrivals.grown(1) > 0.0 && arrivals.grown(1) < 1.0);
        assert_eq!(arrivals.grown(2), 0.0);
        // Past the last of them, which is where they stop being redrawn every frame.
        assert!(!arrivals.tick(super::ARRIVAL_SECONDS + super::ARRIVAL_WINDOW));
        assert_eq!(arrivals.grown(2), 1.0);
    }

    #[test]
    fn a_world_that_has_become_known_shrinks_away_and_grows_back() {
        let before = before(&["Nexus", "Sugar Hole"], &["Sugar Hole"]);
        let mut arrivals = super::Arrivals::new(
            &["Nexus", "Sugar Hole"],
            // No longer a placeholder, which is the whole of what has changed about it.
            &[false, false],
            &[Some(0), Some(1)],
            Some(&before),
        );
        // Sugar Hole starts at its full size, having already been standing there.
        assert_eq!(arrivals.grown(1), 1.0);
        assert_eq!(arrivals.veiled().collect::<Vec<_>>(), vec![1]);
        // Half way out: smaller, and still wearing the placeholder it is leaving.
        arrivals.tick(super::ARRIVAL_SECONDS * 0.5);
        assert!(arrivals.grown(1) > 0.0 && arrivals.grown(1) < 1.0);
        assert_eq!(arrivals.veiled().collect::<Vec<_>>(), vec![1]);
        // Half way back: the placeholder is gone, and what is growing is the world itself.
        arrivals.tick(super::ARRIVAL_SECONDS);
        assert!(arrivals.grown(1) > 0.0 && arrivals.grown(1) < 1.0);
        assert!(arrivals.veiled().next().is_none());
        // And it ends at the size it started, having become something else on the way.
        assert!(!arrivals.tick(super::ARRIVAL_SECONDS));
        assert_eq!(arrivals.grown(1), 1.0);
    }

    #[test]
    fn a_world_still_unvisited_is_not_turned_over() {
        let before = before(&["Nexus", "Sugar Hole"], &["Sugar Hole"]);
        let arrivals = super::Arrivals::new(
            &["Nexus", "Sugar Hole"],
            &[false, true],
            &[Some(0), Some(1)],
            Some(&before),
        );
        assert!(!arrivals.turning());
        assert!(arrivals.arriving().is_none());
    }

    // The layout is still settling around what arrived after the last of it stops growing.
    #[test]
    fn what_arrived_is_still_framed_after_it_has_finished_growing() {
        let before = before(&["Nexus"], &[]);
        let mut arrivals = super::Arrivals::new(
            &["Nexus", "Sugar Hole"],
            &[false; 2],
            &[Some(0), Some(1)],
            Some(&before),
        );
        // Past the growing, which is what stops the geometry being rebuilt every frame.
        assert!(!arrivals.tick(super::ARRIVAL_SECONDS + 0.1));
        assert_eq!(arrivals.grown(1), 1.0);
        // Still something to frame, though, and still the world that arrived.
        assert_eq!(arrivals.arriving().unwrap().collect::<Vec<_>>(), vec![1]);
        // And past the following, which is where an arrival stops costing anything at all.
        assert!(!arrivals.tick(super::ARRIVAL_TRACKED_SECONDS));
        assert!(arrivals.arriving().is_none());
    }

    #[test]
    fn a_graph_with_nothing_new_in_it_animates_nothing() {
        for standing in [None, Some(before(&["Nexus"], &[]))] {
            let mut arrivals =
                super::Arrivals::new(&["Nexus"], &[false], &[Some(0)], standing.as_ref());
            assert_eq!(arrivals.grown(0), 1.0);
            assert!(!arrivals.tick(0.016));
        }
    }

    #[test]
    fn a_great_many_worlds_arriving_still_arrive_within_the_window() {
        let names: Vec<String> = (0..1500).map(|world| world.to_string()).collect();
        let arrivals = super::Arrivals::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
            &vec![false; names.len()],
            &vec![Some(0); names.len()],
            Some(&before(&[], &[])),
        );
        assert!(arrivals.last <= super::ARRIVAL_WINDOW);
    }
}
