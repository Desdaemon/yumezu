//! The graph as something to point at and ask about: what the simulation is told, what the
//! pointer is over, what a selection lights, and what the camera should be shown.

use super::*;

/// Force per unit of distance between a grabbed node and the cursor.
///
/// The cursor pulls the node rather than placing it, so the graph attached to it travels along.
/// Momentum makes the response second order, so the gain has to stay well under what would close
/// the gap in a single frame.
const GRAB_STIFFNESS: f32 = 0.5;
/// So a cursor flung across the window cannot throw a grabbed node clear of the layout.
const GRAB_FORCE_MAX: f32 = 2000.0;
/// How far from a node a press may land and still count as over it, in physical pixels, for a node
/// drawn smaller than that.
///
/// On screen rather than in the world: a world-space radius is a cylinder through the whole depth
/// of the layout, and in a cloud this dense it catches a node under nearly every press.
///
/// A press on a node drawn wider than this has to land inside the node's own drawn edge, or a
/// close enough zoom would leave a plate that fills the window pickable only near its centre.
const GRAB_TOLERANCE_PIXELS: f32 = 14.0;
/// How far the cursor may travel between press and release and still count as a click, in physical
/// pixels.
const GESTURE_SLOP_PIXELS: f32 = 6.0;
/// The world a run opens framed on, with everything behind it: the game's hub, which nearly the
/// whole graph hangs off. As the wiki's English pages spell it.
const OPENING_ROOM: &str = "Nexus";

/// Which world that is, or `None` in a dump that has no such title -- a frontier drawing a player
/// who has not been there, which opens on the camera's own pose rather than on some other tree.
pub(super) fn opening_room(worlds: &[world::World]) -> Option<usize> {
    worlds
        .iter()
        .position(|world| world.title == OPENING_ROOM)
        .or_else(|| (worlds.len() > 3).then_some(2)) // or default to the third world, which is almost always Nexus
}

/// How many worlds the panel names out of a highlighted subtree.
const NOTABLE_WORLDS: usize = 10;
/// Connections a world needs to count as a "notable" descendant.
pub(super) const NOTABLE_HUB_CONNECTIONS: usize = 3;
/// How many worlds the panel names as still having ways out nobody has walked. See
/// [`AppEntities::untaken`].
const UNTAKEN_WORLDS: usize = 10;
/// How many worlds the search box offers at once.
const SEARCH_CANDIDATES: usize = 10;
/// The ways the graph can be cut down to a part worth looking at.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Highlight {
    /// The world's canonical route back to the origin. A plain click.
    Route(usize),
    /// Everything whose route home passes through the world. Asked for from the right-click menu.
    Descendants(usize),
    /// Every world an author is credited with, indexed into [`AppEntities::authors`]. Asked for by
    /// clicking a name, in the panel or in the catalog.
    Author(usize),
    /// What a release added, indexed into [`AppEntities::versions`]. Asked for from the catalog.
    Version(usize),
    /// Every world exactly this many connections from the origin. Asked for by the rocker in the
    /// bottom corner. See `Panel::rocker`.
    Layer(u32),
    /// The two worlds one connection joins and the line between them, and nothing else. Asked for
    /// from the ways on a world offers: see [`Panel::ways_on`].
    ///
    /// Held as the world it was picked from and then the world at the far end. Not a direction --
    /// which ways round it can be walked is read off the connection itself -- but the order is
    /// what keeps the panel on the world the reader is reading.
    Connection(usize, usize),
    /// At most [`UNTAKEN_WORLDS`] of the worlds the player has stood in with the most unwalked
    /// ways out. Asked for from the panel with nothing selected, and only in a run drawing a
    /// frontier: the whole game has no unwalked way to count.
    Untaken,
}

impl Highlight {
    /// `None` for the highlights about a set of worlds rather than a place in the graph: no world
    /// to name, and no route home for the panel to walk.
    pub(super) fn world(self) -> Option<usize> {
        match self {
            // A connection answers with the world it was picked from -- the one whose ways on the
            // panel is listing -- so clicking through them leaves the reader where they started.
            Self::Route(world) | Self::Descendants(world) | Self::Connection(world, _) => {
                Some(world)
            }
            Self::Author(_) | Self::Version(_) | Self::Layer(_) | Self::Untaken => None,
        }
    }
}
pub(super) struct Grab {
    node: DefaultNodeIdx,
    /// Distance from the camera to the node when it was grabbed, so dragging slides the node
    /// across the plane it was picked on rather than pulling it toward the camera.
    depth: f32,
    /// Cursor position in physical pixels.
    cursor: PhysicalPoint,
}

/// The left-button gesture in progress.
pub(super) enum Gesture {
    /// Pressed, and still ambiguous. Carries what the press landed on, so a release can select it
    /// and a drag can start moving it without picking again.
    Held {
        hit: Option<Grab>,
        /// Where the press landed, in physical pixels, to measure travel against.
        origin: PhysicalPoint,
    },
    Moving(Grab),
    /// Awarded to the camera: the motion is left unhandled for [`OrbitControl`].
    Orbiting,
}
/// The worlds and the connections between them, laid out by the simulation.
///
/// A connection carries whether a player can only walk it one way, which is the one thing about it
/// a frame has to know. Its stored direction is the walkable one, so a one-way connection's dashes
/// march the way the player can go. See [`AppEntities::march_dashes`].
pub(super) type Graph = ForceGraph<(), bool>;
/// The worlds a player still has somewhere to go from.
///
/// A way out counts only where the player could actually take it: the far end has to be somewhere
/// they have not been, and the connection walkable in that direction -- the same reading
/// `world::Dump::showing` builds the frontier by, so the list cannot disagree with the graph.
///
/// Ranked most first, ties by world so the list is the same every time it is built, and cut to
/// [`UNTAKEN_WORLDS`]. The counts order the list and are then dropped: a number beside each would
/// invite the reader to weigh two of them against each other, which is a question they did not ask.
///
/// Empty exactly when the graph has no frontier, which is what tells the panel whether to offer
/// the list at all -- read off the graph rather than the account that asked for it, the same way
/// the placeholders are.
pub(super) fn untaken(connections: &[Vec<world::Step>], unknown: &[bool]) -> Vec<usize> {
    let mut ranked: Vec<(usize, usize)> = connections
        .iter()
        .enumerate()
        .filter(|(world, _)| !unknown[*world])
        .map(|(world, steps)| {
            let ways = steps
                .iter()
                .filter(|step| step.out.is_some() && unknown[step.world])
                .count();
            (world, ways)
        })
        .filter(|(_, ways)| *ways > 0)
        .collect();
    ranked.sort_unstable_by_key(|&(world, ways)| (std::cmp::Reverse(ways), world));
    ranked.truncate(UNTAKEN_WORLDS);
    ranked.into_iter().map(|(world, _)| world).collect()
}

impl AppEntities {
    /// Masses and reaches together because the knob means one thing: a hub that pushes its
    /// neighbours harder needs the room to push them into. See [`node_mass`] and [`edge_reach`].
    ///
    /// The visit runs in slot order, which is world order: the nodes were added one per world and
    /// none is ever removed.
    pub(super) fn apply_hub_push(&mut self) {
        let (radii, hub_repulsion) = (&self.node_radii, self.hub_repulsion);
        let mut world = 0;
        self.graph.visit_nodes_mut(|mut node| {
            node.set_mass(node_mass(radii[world], hub_repulsion));
            world += 1;
        });
        self.graph.visit_edges_mut(|from, to, edge| {
            // The edge was given the one-way flag to carry, so the reach is rewritten from the
            // same answer it was first built from.
            edge.reach = edge_reach(
                edge.user_data,
                radii[from.index()].max(radii[to.index()]),
                hub_repulsion,
            );
        });
    }

    /// Resolves the left button between clicking a node, dragging a node and orbiting, marking as
    /// handled only the events the winner actually uses.
    ///
    /// Runs before [`OrbitControl`], which loses this contest: it reads any unhandled left-drag
    /// motion, so the decision has to be made, and the undecided motion swallowed, upstream of it.
    ///
    /// Also where the hover is settled, off the same events: it is the reading of the pointer no
    /// button is contesting.
    pub(super) fn track_gesture(&mut self, camera: &Camera, events: &mut [Event], pinching: bool) {
        for event in events.iter_mut() {
            match event {
                // While more than one finger is down the left button belongs to the pinch, so
                // neither its travel nor its release is a gesture of its own. Swallowed rather
                // than skipped, so the orbit control downstream does not read them either.
                Event::MousePress {
                    button: MouseButton::Left,
                    handled,
                    ..
                }
                | Event::MouseMotion {
                    button: Some(MouseButton::Left),
                    handled,
                    ..
                }
                | Event::MouseRelease {
                    button: MouseButton::Left,
                    handled,
                    ..
                } if pinching => *handled = true,
                Event::MousePress {
                    button: MouseButton::Left,
                    position,
                    handled,
                    ..
                } if !*handled => {
                    // A press anywhere the menu did not take is a press past it.
                    self.menu = None;
                    // Nominate, do not award: the press is ambiguous even over empty space, where
                    // it may still turn out to be the click that clears the selection.
                    self.gesture = Some(Gesture::Held {
                        hit: self.pick(camera, *position),
                        origin: *position,
                    });
                    *handled = true;
                }
                // Only while the button is actually down: a release lost to a focus change would
                // otherwise leave a nomination standing for the next hover to award.
                Event::MouseMotion {
                    button: Some(MouseButton::Left),
                    position,
                    handled,
                    ..
                } if !*handled => {
                    let awarded = match &mut self.gesture {
                        Some(Gesture::Held { hit, origin, .. }) => {
                            if (position.x - origin.x).hypot(position.y - origin.y)
                                <= GESTURE_SLOP_PIXELS
                            {
                                // Still within the slop, so this may yet be a click. Swallowed,
                                // or the camera would orbit under every click on a node.
                                *handled = true;
                                None
                            } else {
                                Some(match hit.take() {
                                    Some(mut grab) => {
                                        grab.cursor = *position;
                                        *handled = true;
                                        Gesture::Moving(grab)
                                    }
                                    // Nothing under the press, so the drag belongs to the camera.
                                    None => Gesture::Orbiting,
                                })
                            }
                        }
                        Some(Gesture::Moving(grab)) => {
                            grab.cursor = *position;
                            *handled = true;
                            None
                        }
                        Some(Gesture::Orbiting) | None => None,
                    };
                    if let Some(awarded) = awarded {
                        self.gesture = Some(awarded);
                    }
                }
                // The right button is contested too -- it pans the camera -- so it is nominated
                // the same way the left one is, and left unhandled for the pan to claim its
                // motion. Any press closes the open menu, a pan carrying the graph out from
                // under it.
                Event::MousePress {
                    button: MouseButton::Right,
                    position,
                    handled,
                    ..
                } if !*handled => {
                    self.menu = None;
                    self.right_press = Some(*position);
                }
                Event::MouseRelease {
                    button: MouseButton::Right,
                    position,
                    ..
                } => {
                    // A press that never travelled is a click, which on a world opens its menu.
                    if let Some(origin) = self.right_press.take()
                        && (position.x - origin.x).hypot(position.y - origin.y)
                            <= GESTURE_SLOP_PIXELS
                    {
                        self.menu = self.pick(camera, *position).map(|grab| ContextMenu {
                            world: grab.node.index(),
                            at: *position,
                        });
                    }
                }
                // Motion with nothing pressed is nobody's gesture, so it is read rather than
                // taken. A position the panel already claimed counts as no pointer at all.
                Event::MouseMotion {
                    button: None,
                    position,
                    handled,
                    ..
                } => self.cursor = (!*handled).then_some(*position),
                Event::MouseLeave => self.cursor = None,
                Event::MouseRelease {
                    button: MouseButton::Left,
                    ..
                } => {
                    // A gesture still undecided at release never travelled: it is a click, which
                    // selects what it landed on, or clears the selection over empty space.
                    let clicked = match &self.gesture {
                        Some(Gesture::Held { hit, .. }) => Some(hit.as_ref().map(|grab| grab.node)),
                        _ => None,
                    };
                    self.gesture = None;
                    if let Some(node) = clicked {
                        self.select(node.map(|node| Highlight::Route(node.index())));
                    }
                }
                _ => (),
            }
        }
        // Retested every frame rather than only on motion: the layout is usually still moving and
        // the camera can be flown with the keys, so what is under a still cursor changes anyway.
        // One walk of the nodes, a fraction of the walk [`AppEntities::magnified`] already makes
        // per frame. Nothing hovers mid-gesture: the pointer is dragging a node or turning the
        // camera.
        self.hover = match self.gesture {
            None => self
                .cursor
                .and_then(|cursor| self.pick(camera, cursor))
                .map(|grab| grab.node.index()),
            Some(_) => None,
        };
    }

    /// Against the node positions rather than the drawn geometry: the plates are one instanced
    /// mesh, so there is nothing per node to intersect. The test is how far the node lands from
    /// the cursor on screen, which is the only distance the person clicking can see.
    fn pick(&self, camera: &Camera, cursor: PhysicalPoint) -> Option<Grab> {
        let origin = camera.position_at_pixel(cursor);
        let direction = camera.view_direction_at_pixel(cursor);
        let across = camera.right_direction().normalize();
        let mut nearest: Option<Grab> = None;
        self.graph.visit_nodes(|node| {
            let position = world_pos(node.position());
            let depth = (position - origin).dot(direction);
            // Behind the camera, or further off than something already found.
            if depth <= 0.0 || nearest.as_ref().is_some_and(|near| near.depth <= depth) {
                return;
            }
            let pixel = camera.pixel_at_position(position);
            let tolerance = GRAB_TOLERANCE_PIXELS.max(
                0.5 * drawn_width(
                    camera,
                    across,
                    self.node_radii[node.index().index()],
                    position,
                    pixel,
                ),
            );
            if (pixel.x - cursor.x).hypot(pixel.y - cursor.y) < tolerance {
                nearest = Some(Grab {
                    node: node.index(),
                    depth,
                    cursor,
                });
            }
        });
        nearest
    }

    /// Pulling a node also keeps the simulation awake.
    pub(super) fn pull_grabbed_node(&mut self, camera: &Camera) {
        let Some(Gesture::Moving(grab)) = &self.gesture else {
            return;
        };
        let Some(position) = self.graph.node(grab.node).map(|node| node.position()) else {
            return;
        };
        let target = camera.position_at_pixel(grab.cursor)
            + camera.view_direction_at_pixel(grab.cursor) * grab.depth;
        let offset = sim_pos(target) - sim_pos(world_pos(position));
        let force = offset * GRAB_STIFFNESS;
        let force = force * (GRAB_FORCE_MAX / force.magnitude().max(GRAB_FORCE_MAX));
        self.graph.apply_force(grab.node, force.into());
    }

    /// The selected node and every step from it back to the origin world, selection first. Empty
    /// with nothing selected, whatever the selection lights: the route is what the panel walks.
    pub(super) fn route(&self) -> Vec<usize> {
        let mut route = Vec::new();
        let mut step = self.selected.and_then(Highlight::world);
        while let Some(node) = step {
            route.push(node);
            step = self.routes.parents[node];
        }
        route
    }

    /// Empty with nothing selected.
    pub(super) fn highlighted(&self) -> Vec<usize> {
        match self.selected {
            None => Vec::new(),
            // One step on rather than the whole subtree, which is `Highlight::Descendants`. The
            // chain and the children meet at that world and nowhere else, so `repaint` can colour
            // them apart.
            Some(Highlight::Route(world)) => {
                let mut lit = self.route();
                lit.extend(
                    self.routes
                        .parents
                        .iter()
                        .enumerate()
                        .filter(|(_, parent)| **parent == Some(world))
                        .map(|(child, _)| child),
                );
                lit
            }
            Some(Highlight::Descendants(world)) => self.routes.subtree(world),
            Some(Highlight::Author(author)) => self.authors[author].worlds.clone(),
            Some(Highlight::Version(version)) => self.versions[version].worlds.clone(),
            Some(Highlight::Layer(depth)) => self.layer(depth),
            Some(Highlight::Connection(at, far)) => vec![at, far],
            Some(Highlight::Untaken) => self.untaken.clone(),
        }
    }

    /// In world order. The worlds the origin cannot reach have no depth, and so sit on no layer.
    fn layer(&self, depth: u32) -> Vec<usize> {
        self.routes
            .depth
            .iter()
            .enumerate()
            .filter(|(_, at)| **at == Some(depth))
            .map(|(world, _)| world)
            .collect()
    }

    /// At most [`NOTABLE_WORLDS`], and empty unless a subtree is what is lit.
    ///
    /// Two kinds are worth a name, for opposite reasons: a junction, where the subtree opens out,
    /// and a dead end, where it stops. Neither measure ranks the other, so the two are ranked
    /// apart -- junctions by how many ways they offer, dead ends by how far out they are -- and
    /// then taken in turns, which keeps the list from filling with junctions before it names a
    /// single place the subtree ends.
    pub(super) fn notable(&self) -> Vec<usize> {
        let Some(Highlight::Descendants(root)) = self.selected else {
            return Vec::new();
        };
        let subtree = self.routes.subtree(root);
        let mut junctions: Vec<_> = subtree
            .iter()
            .copied()
            .filter(|&world| world != root && self.degrees[world] >= NOTABLE_HUB_CONNECTIONS)
            .collect();
        junctions.sort_unstable_by_key(|&world| (std::cmp::Reverse(self.degrees[world]), world));
        let mut dead_ends: Vec<_> = subtree
            .iter()
            .copied()
            .filter(|&world| world != root && self.degrees[world] <= 1)
            .collect();
        dead_ends
            .sort_unstable_by_key(|&world| (std::cmp::Reverse(self.routes.depth[world]), world));

        let mut notable = Vec::with_capacity(NOTABLE_WORLDS);
        let (mut junctions, mut dead_ends) = (junctions.into_iter(), dead_ends.into_iter());
        // Whichever kind runs out first, the other goes on filling the list alone.
        while notable.len() < NOTABLE_WORLDS {
            let taken = notable.len();
            notable.extend(junctions.next());
            if notable.len() < NOTABLE_WORLDS {
                notable.extend(dead_ends.next());
            }
            if notable.len() == taken {
                break;
            }
        }
        notable
    }

    /// Everything the selection lights, except a route the reader has not asked to see whole:
    /// that is framed on the world at its end, which is what was picked and what the panel is
    /// reading. See [`Self::frame_route`].
    fn framed(&self) -> Vec<usize> {
        match self.selected {
            Some(Highlight::Route(world)) if !self.frame_route => vec![world],
            // What a run opens framed on. Read only while framing, so clearing a selection later
            // leaves the person looking at whatever they were looking at.
            None => self
                .opening
                .map_or_else(Vec::new, |root| self.routes.subtree(root)),
            // The chain home, not everything a route lights: the button asks for the way there,
            // not for however much of the game hangs off its end.
            Some(Highlight::Route(_)) => self.route(),
            _ => self.highlighted(),
        }
    }

    /// `None` for a run with nothing selected and no world to open on.
    ///
    /// Centred on the middle of the bounding box rather than on the average, so a route that piles
    /// up near the origin and reaches out with a few steps is still framed around what it spans
    /// instead of around where most of it sits.
    ///
    /// The reach counts each world's own radius, so the sphere holds the thumbnails rather than
    /// the points they hang on: that is what a lone world is framed by, and what keeps a hub on
    /// the rim of a group whole instead of clipped by the window edge.
    pub(super) fn framing_bounds(&self) -> Option<Bounds> {
        let highlighted = self.framed();
        if highlighted.is_empty() {
            return None;
        }
        let mut on_route = vec![false; self.titles.len()];
        for node in highlighted {
            on_route[node] = true;
        }
        self.bounds_of(&on_route)
    }

    /// What a refresh turned up is what a person pressed refresh to see, and it may be nowhere
    /// near the part of the map they were looking at. `None` once the arrival is over, which is
    /// what hands the camera back.
    ///
    /// The arriving worlds only, not their neighbours: a new world may join back to somewhere
    /// nowhere near the rest of it, and one edge like that drags the sphere across the whole graph.
    pub(super) fn arrival_bounds(&self) -> Option<Bounds> {
        let arriving = self.arrivals.arriving()?;
        let mut wanted = vec![false; self.titles.len()];
        for world in arriving {
            wanted[world] = true;
        }
        self.bounds_of(&wanted)
    }

    pub(super) fn world_at(&self, world: usize) -> Option<Vec3> {
        let mut wanted = vec![false; self.titles.len()];
        *wanted.get_mut(world)? = true;
        Some(self.bounds_of(&wanted)?.center)
    }

    /// The sphere holding a set of worlds, pictures and all. See [`Self::framing_bounds`].
    fn bounds_of(&self, wanted: &[bool]) -> Option<Bounds> {
        // Paired with the radius, both ends of the reach below needing the two together.
        let mut positions = Vec::new();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            if wanted[world] {
                positions.push((world_pos(node.position()), self.node_radii[world]));
            }
        });

        if positions.is_empty() {
            return None;
        }
        let low = positions
            .iter()
            .map(|&(position, radius)| position - vec3(radius, radius, radius))
            .reduce(|a, b| vec3(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z)))
            .unwrap();
        let high = positions
            .iter()
            .map(|&(position, radius)| position + vec3(radius, radius, radius))
            .reduce(|a, b| vec3(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z)))
            .unwrap();
        let center = (low + high) * 0.5;
        let radius = positions
            .iter()
            .map(|&(position, radius)| (position - center).magnitude() + radius)
            .fold(0.0, f32::max);
        Some(Bounds { center, radius })
    }

    /// At most [`SEARCH_CANDIDATES`], ranked by where the match falls and then by how much title
    /// is left over: a world whose name starts with what was typed comes before one that merely
    /// contains it, and an exact name before the longer names that extend it.
    ///
    /// Empty for an empty needle -- ten arbitrary worlds are noise rather than suggestions.
    pub(super) fn search(&self, needle: &str) -> Vec<usize> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<_> = self
            .titles
            .iter()
            .enumerate()
            .filter_map(|(world, title)| {
                let (at, length) = title.find(&needle)?;
                Some((at, length, world))
            })
            .collect();
        hits.sort_unstable();
        hits.truncate(SEARCH_CANDIDATES);
        hits.into_iter().map(|(_, _, world)| world).collect()
    }

    /// The world a name names, by either of its names and in whatever case. A world the player has
    /// not been to is named nothing and so cannot be found here, as in [`AppEntities::search`].
    pub(super) fn world_named(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.titles
            .iter()
            .position(|title| title.names().any(|have| have.eq_ignore_ascii_case(name)))
    }

    pub(super) fn select(&mut self, selected: Option<Highlight>) {
        if selected != self.selected {
            self.selected = selected;
            // Clearing a selection leaves the person looking at whatever they were looking at.
            self.framing = selected.is_some();
            self.repaint();
        }
    }

    /// What the graph that replaces this one should carry on from. See [`Before`].
    pub(super) fn before(&self) -> Before {
        let mut standing = Standing::with_capacity(self.titles.len());
        self.graph.visit_nodes(|node| {
            standing.insert(
                self.titles[node.index().index()].en.clone(),
                node.position(),
            );
        });
        let parameters = self.graph.parameters();
        Before {
            unvisited: self
                .unvisited
                .iter()
                .map(|&world| self.titles[world].en.clone())
                .collect(),
            standing,
            layout: Layout {
                dimensions: parameters.dimensions,
                layered: parameters.dag_level_distance.is_some(),
                hub_repulsion: self.hub_repulsion,
                link_reach: self.link_reach,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_family = "wasm")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::world;

    // The frontier is built outward along walkable steps, so counting a connection the player could
    // only come back through would offer a world an exit it has not got.
    #[test]
    fn only_ways_out_a_player_could_walk_are_counted_as_untaken() {
        let step = |world, out: bool| world::Step {
            world,
            out: out.then(world::Ask::free),
            back: Some(world::Ask::free()),
        };
        // 0 visited, joined to three unknowns -- one it can walk to, one it can only come back
        // through, one it can walk to again -- and to a fourth world it has been to.
        let connections = vec![
            vec![step(1, true), step(2, false), step(3, true), step(4, true)],
            vec![step(0, true)],
            vec![step(0, true)],
            vec![step(0, true)],
            vec![step(0, true)],
        ];
        let unknown = [false, true, true, true, false];
        assert_eq!(super::untaken(&connections, &unknown), vec![0]);
    }

    #[test]
    fn untaken_worlds_are_ranked_most_first_and_cut_to_the_ten_named() {
        let out = |world| world::Step {
            world,
            out: Some(world::Ask::free()),
            back: Some(world::Ask::free()),
        };
        // Twelve visited worlds, each joined to a different number of unknowns: world 0 to one,
        // world 11 to twelve. The unknowns are numbered above all of them.
        let visited = 12;
        let mut connections: Vec<Vec<world::Step>> = (0..visited * 2).map(|_| Vec::new()).collect();
        let mut unknown = vec![false; connections.len()];
        for world in 0..visited {
            for nth in 0..=world {
                let far = visited + nth;
                unknown[far] = true;
                connections[world].push(out(far));
                connections[far].push(out(world));
            }
        }
        // World 11 has twelve ways out; world 2, with three, is the last to make the cut.
        let ranked = super::untaken(&connections, &unknown);
        assert_eq!(ranked, vec![11, 10, 9, 8, 7, 6, 5, 4, 3, 2]);
    }

    // What keeps the panel from offering a list that would say the same thing about every world.
    #[test]
    fn a_graph_with_no_frontier_has_nothing_untaken() {
        let both = |world| world::Step {
            world,
            out: Some(world::Ask::free()),
            back: Some(world::Ask::free()),
        };
        let connections = vec![vec![both(1)], vec![both(0)]];
        assert!(super::untaken(&connections, &[false, false]).is_empty());
    }
}
