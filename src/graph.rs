//! The graph as something to point at and read: what the simulation is told, what the
//! pointer is over, what a selection highlights, and what the camera should be shown.

use super::*;

/// Force per unit of distance pulling a grabbed node, and the graph attached, toward the cursor;
/// well under closing the gap in one frame, as momentum overshoots.
const GRAB_STIFFNESS: f32 = 0.5;
/// So a cursor flung across the window cannot throw a grabbed node clear of the layout.
const GRAB_FORCE_MAX: f32 = 2000.0;
/// Screen distance from a node, in physical pixels, within which a press hits the node; a node
/// drawn wider is hit anywhere inside the drawn edge.
const GRAB_TOLERANCE_PIXELS: f32 = 14.0;
/// How far the cursor may travel between press and release and still count as a click, in physical
/// pixels.
const GESTURE_SLOP_PIXELS: f32 = 6.0;
/// The world a session opens framed on, with the subtree: the game's hub. Spelled as on the
/// wiki's English pages.
const OPENING_ROOM: &str = "Nexus";

/// Index of [`OPENING_ROOM`], else the third world; `None` for a dump of three worlds or fewer.
pub(super) fn opening_room(worlds: &[world::World]) -> Option<usize> {
    worlds
        .iter()
        .position(|world| world.title == OPENING_ROOM)
        .or_else(|| (worlds.len() > 3).then_some(2)) // the third world is almost always Nexus
}

/// How many worlds the panel names out of a highlighted subtree.
const NOTABLE_WORLDS: usize = 10;
/// Connections a world needs to count as a "notable" descendant.
pub(super) const NOTABLE_HUB_CONNECTIONS: usize = 3;
/// How many ways between two worlds the panel offers at once.
const WAYS: usize = 5;

/// How many worlds the panel names as still having exits nobody has taken. See
/// [`AppEntities::untaken`].
const UNTAKEN_WORLDS: usize = 10;
const SEARCH_CANDIDATES: usize = 10;
/// The ways the graph can be cut down to a part worth looking at.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Highlight {
    /// The world's canonical route back to the origin. A plain click.
    Route(usize),
    /// Everything whose route home passes through the world. Picked from the right-click menu.
    Descendants(usize),
    /// Every world an author is credited with, indexed into [`AppEntities::authors`]. Picked by
    /// clicking a name, in the panel or in the catalog.
    Author(usize),
    /// What a release added, indexed into [`AppEntities::versions`]. Picked from the catalog.
    Version(usize),
    /// Every world exactly this many connections from the origin. Picked by the rocker in the
    /// bottom corner. See `Panel::rocker`.
    Layer(u32),
    /// One connection: the two worlds and the line between, picked in `Panel::ways_on`. Held
    /// as the world picked from, then the far end, so the panel stays on the world being read.
    Connection(usize, usize),
    /// At most [`UNTAKEN_WORLDS`] visited worlds with the most untaken exits; only with a
    /// frontier drawn.
    Untaken,
    /// The way from one world to another, held as origin then destination. The one highlight
    /// [`AppEntities::routes`] cannot answer, so the traversals are kept in [`AppEntities::ways`].
    Path(usize, usize),
}

impl Highlight {
    /// `None` for highlights of a set of worlds: no world to name, and no route home for the panel
    /// to traverse.
    pub(super) fn world(self) -> Option<usize> {
        match self {
            // A connection gives the world picked from, for the list to return to; a path, the end.
            Self::Route(world)
            | Self::Descendants(world)
            | Self::Connection(world, _)
            | Self::Path(_, world) => Some(world),
            Self::Author(_) | Self::Version(_) | Self::Layer(_) | Self::Untaken => None,
        }
    }
}
pub(super) struct Grab {
    node: DefaultNodeIdx,
    /// Distance from the camera to the node at the grab, so dragging slides the node across that
    /// plane.
    depth: f32,
    cursor: PhysicalPoint,
}

/// The left-button gesture in progress.
pub(super) enum Gesture {
    /// Pressed and undecided, with the node hit, so a release selects and a drag moves that node
    /// without picking again.
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
/// A connection's stored direction is the passable one, so a one-way connection's dashes march the
/// way the player can go. See [`AppEntities::update_dashes`].
pub(super) type Graph = ForceGraph<(), bool>;
/// Visited worlds with passable exits to unvisited ones, as `world::Dump::showing` builds the
/// frontier: most exits first, ties by world, at most [`UNTAKEN_WORLDS`]; empty with no frontier.
pub(super) fn untaken(connections: &[Vec<world::Step>], concealed: &[bool]) -> Vec<usize> {
    let mut ranked: Vec<(usize, usize)> = connections
        .iter()
        .enumerate()
        .filter(|(world, _)| !concealed[*world])
        .map(|(world, steps)| {
            let ways = steps
                .iter()
                .filter(|step| step.outgoing.is_some() && concealed[step.world])
                .count();
            (world, ways)
        })
        .filter(|(_, ways)| *ways > 0)
        .collect();
    ranked.sort_unstable_by_key(|&(world, ways)| (std::cmp::Reverse(ways), world));
    ranked.truncate(UNTAKEN_WORLDS);
    ranked.into_iter().map(|(world, _)| world).collect()
}

/// Indexed by world: whether each is one of `worlds`.
pub(super) fn world_mask(count: usize, worlds: impl IntoIterator<Item = usize>) -> Vec<bool> {
    let mut mask = vec![false; count];
    for world in worlds {
        mask[world] = true;
    }
    mask
}

pub(super) fn pixel_distance(a: PhysicalPoint, b: PhysicalPoint) -> f32 {
    (a.x - b.x).hypot(a.y - b.y)
}

impl AppEntities {
    pub(super) fn dimensions(&self) -> Dimensions {
        self.graph.parameters().dimensions
    }

    /// Whether every world but the Nexus is laid out on the 3D sky.
    pub(super) fn on_sky(&self) -> bool {
        self.arrangement == Arrangement::Orbital
    }

    /// Masses and leads together, as a hub that pushes harder needs room to push into; on the 3D
    /// sky both shrink with apparent size from the Nexus. See [`node_mass`] and [`edge_lead`].
    ///
    /// Nodes are visited in world order: one node per world was added and none removed.
    pub(super) fn set_masses_and_leads(&mut self) {
        let (radii, hub_repulsion) = (&self.node_radii, self.hub_repulsion);
        let descendants = &self.descendant_counts;
        let arrangement = self.arrangement;
        let radial = arrangement.radial();
        let branches = &self.branches;
        let held_apart =
            |from, to| !self.pull_across_branches && crosses_branches(branches, from, to);
        let depths = match arrangement {
            Arrangement::Tree | Arrangement::FlatTree => &self.start_depths,
            _ => &self.shell_depths,
        };
        let distances = &mut self.sky_distances;
        for (distance, &depth) in distances.iter_mut().zip(depths) {
            *distance = sky_distance(depth, arrangement);
        }
        // The layout weighs a world by how much farther than the first depth the world is drawn.
        let nearest = sky_distance(1.0, arrangement);
        let farther: Vec<f32> = distances
            .iter()
            .map(|distance| distance / nearest)
            .collect();
        let mut world = 0;
        self.graph.visit_nodes_mut(|mut node| {
            node.set_mass(node_mass(radii[world], hub_repulsion) / farther[world]);
            node.set_level(shell_level(depths[world], arrangement));
            world += 1;
        });
        self.graph.visit_edges_mut(|from, to, edge| {
            let (from, to) = (from.index(), to.index());
            (edge.reach, edge.stiffness) = edge_lead(
                edge.user_data,
                held_apart(from, to),
                radii[from].max(radii[to]),
                descendants[from].max(descendants[to]),
                hub_repulsion,
                radial,
                shell_gap(depths[from], arrangement).min(shell_gap(depths[to], arrangement))
                    / (farther[from] * farther[to]).sqrt(),
            );
        });
    }

    /// Resolves the left button into a click, a node drag or an orbit, and settles the hover. Runs
    /// before [`OrbitControl`], which reads any left drag left unhandled.
    pub(super) fn track_gesture(&mut self, camera: &Camera, events: &mut [Event], pinching: bool) {
        for event in events.iter_mut() {
            match event {
                // Swallowed, so the orbit control ignores a pinch's left button.
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
                    self.menu = None;
                    // Even over empty space, the press may be a click clearing the selection.
                    self.gesture = Some(Gesture::Held {
                        hit: self.pick(camera, *position),
                        origin: *position,
                    });
                    *handled = true;
                }
                // Button down only, so a release lost to a focus change cannot make a hover drag.
                Event::MouseMotion {
                    button: Some(MouseButton::Left),
                    position,
                    handled,
                    ..
                } if !*handled => {
                    let awarded = match &mut self.gesture {
                        Some(Gesture::Held { hit, origin, .. }) => {
                            if pixel_distance(*position, *origin) <= GESTURE_SLOP_PIXELS {
                                // Within the slop: swallowed so a click on a node does not orbit.
                                *handled = true;
                                None
                            } else {
                                Some(match hit.take() {
                                    Some(mut grab) => {
                                        grab.cursor = *position;
                                        *handled = true;
                                        Gesture::Moving(grab)
                                    }
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
                // Left unhandled for the pan; any press closes the menu, which a pan would strand.
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
                    if let Some(origin) = self.right_press.take()
                        && pixel_distance(*position, origin) <= GESTURE_SLOP_PIXELS
                    {
                        self.menu = self.pick(camera, *position).map(|grab| ContextMenu {
                            world: grab.node.index(),
                            at: *position,
                        });
                    }
                }
                // Left unhandled; a position the panel claimed counts as no pointer.
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
                    // An undecided gesture at release is a click.
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
        // Every frame: the layout and the keyboard movement move what is under a still cursor.
        self.hover = match self.gesture {
            None => self
                .cursor
                .and_then(|cursor| self.pick(camera, cursor))
                .map(|grab| grab.node.index()),
            Some(_) => None,
        };
    }

    /// The nearest node within tolerance of the cursor on screen, tested at node positions as the
    /// plates are one instanced mesh.
    fn pick(&self, camera: &Camera, cursor: PhysicalPoint) -> Option<Grab> {
        let origin = camera.position_at_pixel(cursor);
        let direction = camera.view_direction_at_pixel(cursor);
        let across = camera.right_direction().normalize();
        let mut nearest: Option<Grab> = None;
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            let position = world_pos(node.position(), self.sky_distances[world]);
            let depth = (position - origin).dot(direction);
            if depth <= 0.0 || nearest.as_ref().is_some_and(|near| near.depth <= depth) {
                return;
            }
            let pixel = camera.pixel_at_position(position);
            let tolerance = GRAB_TOLERANCE_PIXELS
                .max(0.5 * drawn_width(camera, across, self.node_radii[world], position, pixel));
            if pixel_distance(pixel, cursor) < tolerance {
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
        let distance = self.sky_distances[grab.node.index()];
        let offset = sim_pos(target, distance) - Vec3::from(position);
        let force = offset * GRAB_STIFFNESS;
        let force = force * (GRAB_FORCE_MAX / force.magnitude().max(GRAB_FORCE_MAX));
        self.graph.apply_force(grab.node, force.into());
    }

    /// The selected world and every step back to the origin, or the chosen way reversed when
    /// directions are highlighted: arrival first either way. Empty with nothing selected.
    pub(super) fn route(&self) -> Vec<usize> {
        if let Some(Highlight::Path(..)) = self.selected {
            let arrival_first = |way: &world::Way| way.worlds.iter().rev().copied().collect();
            return self.ways.get(self.way).map_or_else(Vec::new, arrival_first);
        }
        let mut route = Vec::new();
        let mut step = self.selected.and_then(Highlight::world);
        while let Some(node) = step {
            route.push(node);
            step = self.routes.parents[node];
        }
        route
    }

    pub(super) fn highlighted(&self) -> Vec<usize> {
        match self.selected {
            None => Vec::new(),
            // The chain and the children meet only at that world, so `repaint` colours them apart.
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
            Some(Highlight::Path(from, to)) => match self.route() {
                route if route.is_empty() => vec![from, to],
                route => route,
            },
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

    /// At most [`NOTABLE_WORLDS`] junctions and dead ends of the highlighted subtree, taken in
    /// turns: junctions by most connections, dead ends by depth. Empty for any other highlight.
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

    /// Everything the selection highlights, except a route not chosen to be framed whole, which
    /// frames the picked world alone. See [`Self::frame_route`].
    fn framed(&self) -> Vec<usize> {
        match self.selected {
            Some(Highlight::Route(world)) if !self.frame_route => vec![world],
            // Read only while framing, so clearing a selection keeps the view.
            None => self.opened_on(),
            // The chain home; directions fall through to be framed whole.
            Some(Highlight::Route(_)) => self.route(),
            _ => self.highlighted(),
        }
    }

    /// Every world the opening view frames: whatever the opening world leads to.
    fn opened_on(&self) -> Vec<usize> {
        self.opening
            .map_or_else(Vec::new, |root| self.routes.subtree(root))
    }

    /// The sphere the opening view frames, whatever is selected. See [`Self::opened_on`].
    pub(super) fn opening_bounds(&self) -> Option<Bounds> {
        self.bounds_of(&world_mask(self.titles.len(), self.opened_on()))
    }

    /// `None` when nothing is selected and there is no world to open on. Centred on the bounding
    /// box, so a route is framed around the span; radii count, so thumbnails stay whole.
    pub(super) fn framing_bounds(&self) -> Option<Bounds> {
        let highlighted = self.framed();
        if highlighted.is_empty() {
            return None;
        }
        self.bounds_of(&world_mask(self.titles.len(), highlighted))
    }

    /// The sphere every world sits in, for `profile` to keep the whole graph on screen. See
    /// [`super::profile::pan_aside`].
    #[cfg(feature = "profile")]
    pub(super) fn whole_bounds(&self) -> Option<Bounds> {
        self.bounds_of(&vec![true; self.titles.len()])
    }

    /// The sphere of the worlds a refresh added, which the user refreshed to see; `None` once the
    /// arrival hands the camera back. Neighbours are left out, as one far edge spans the graph.
    pub(super) fn arrival_bounds(&self) -> Option<Bounds> {
        let arriving = self.graft.arriving()?;
        self.bounds_of(&world_mask(self.titles.len(), arriving))
    }

    /// `None` in a dump with no Nexus. See [`opening_room`].
    pub(super) fn nexus_bounds(&self) -> Option<Bounds> {
        let nexus = self.opening?;
        self.bounds_of(&world_mask(self.titles.len(), [nexus]))
    }

    pub(super) fn world_at(&self, world: usize) -> Option<Vec3> {
        let mut wanted = vec![false; self.titles.len()];
        *wanted.get_mut(world)? = true;
        Some(self.bounds_of(&wanted)?.center)
    }

    /// The sphere holding a set of worlds, pictures and all. See [`Self::framing_bounds`].
    fn bounds_of(&self, wanted: &[bool]) -> Option<Bounds> {
        let mut positions = Vec::new();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            if wanted[world] {
                let position = world_pos(node.position(), self.sky_distances[world]);
                positions.push((position, self.node_radii[world]));
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

    /// At most [`SEARCH_CANDIDATES`], ranked as [`world::ranked_matches`] ranks them; empty for an
    /// empty needle.
    pub(super) fn search(&self, needle: &str) -> Vec<usize> {
        if needle.trim().is_empty() {
            return Vec::new();
        }
        let mut worlds = world::ranked_matches(self.titles.iter().map(world::Title::names), needle);
        worlds.truncate(SEARCH_CANDIDATES);
        worlds
    }

    /// The world with this name, in either language and any case. Unvisited worlds are nameless and
    /// so not found, as in [`AppEntities::search`].
    pub(super) fn world_named(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        self.titles
            .iter()
            .position(|title| title.names().any(|have| have.eq_ignore_ascii_case(name)))
    }

    pub(super) fn select(&mut self, selected: Option<Highlight>) {
        if selected != self.selected {
            self.selected = selected;
            self.ways = match selected {
                Some(Highlight::Path(from, to)) => {
                    world::ways(&self.connections, from, to, WAYS, self.hub)
                }
                _ => Vec::new(),
            };
            self.way = 0;
            // Clearing a selection keeps the view.
            self.framing = selected.is_some();
            self.repaint();
        }
    }

    /// Picks which of [`Self::ways`] is drawn and read out, and frames again, as another way spans
    /// other worlds.
    pub(super) fn take_way(&mut self, way: usize) {
        if way != self.way && way < self.ways.len() {
            self.way = way;
            self.framing = true;
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
        Before {
            unvisited: self
                .unvisited
                .iter()
                .map(|&world| self.titles[world].en.clone())
                .collect(),
            standing,
            layout: Layout {
                arrangement: self.arrangement,
                hub_repulsion: self.hub_repulsion,
                pull_across_branches: self.pull_across_branches,
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

    #[test]
    fn only_passable_exits_are_counted_as_untaken() {
        let step = |world, out: bool| world::Step {
            world,
            outgoing: out.then(world::Conditions::free),
            incoming: Some(world::Conditions::free()),
        };
        // 0 visited: passable, back-only, passable unknowns, and visited 4.
        let connections = vec![
            vec![step(1, true), step(2, false), step(3, true), step(4, true)],
            vec![step(0, true)],
            vec![step(0, true)],
            vec![step(0, true)],
            vec![step(0, true)],
        ];
        let concealed = [false, true, true, true, false];
        assert_eq!(super::untaken(&connections, &concealed), vec![0]);
    }

    #[test]
    fn untaken_worlds_are_ranked_most_first_and_cut_to_the_ten_named() {
        let out = |world| world::Step {
            world,
            outgoing: Some(world::Conditions::free()),
            incoming: Some(world::Conditions::free()),
        };
        // World n joins n + 1 unknowns, numbered above the visited.
        let visited = 12;
        let mut connections: Vec<Vec<world::Step>> = (0..visited * 2).map(|_| Vec::new()).collect();
        let mut concealed = vec![false; connections.len()];
        for world in 0..visited {
            for nth in 0..=world {
                let far = visited + nth;
                concealed[far] = true;
                connections[world].push(out(far));
                connections[far].push(out(world));
            }
        }
        let ranked = super::untaken(&connections, &concealed);
        assert_eq!(ranked, vec![11, 10, 9, 8, 7, 6, 5, 4, 3, 2]);
    }

    #[test]
    fn a_graph_with_no_frontier_has_nothing_untaken() {
        let both = |world| world::Step {
            world,
            outgoing: Some(world::Conditions::free()),
            incoming: Some(world::Conditions::free()),
        };
        let connections = vec![vec![both(1)], vec![both(0)]];
        assert!(super::untaken(&connections, &[false, false]).is_empty());
    }
}
