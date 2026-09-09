//! The graph as something drawn: the worlds, their connections, the dashes that march along the
//! one-way ones, and the backdrop behind all of it. See [`entities`] and [`AppEntities`].

use super::*;

/// How thick a connection is drawn.
const EDGE_RADIUS: f32 = 0.05;
/// Sides of the tube a connection is drawn as. Tens of thousands of them come out a pixel wide, so
/// the cost is triangle setup rather than fill and the sides are very nearly the whole of it.
const EDGE_SIDES: u32 = 3;
/// A connection a player can only walk one way is drawn as marching dashes. A fixed count rather
/// than a fixed dash length, so it is settled when the graph is built and never moves with the
/// layout -- the dashes stretch with the connection instead.
const EDGE_DASHES: usize = 7;
/// How much of its own slot a dash fills. Short, because a dash is only seen to move against the
/// gap it leaves behind.
const EDGE_DASH_FILL: f32 = 0.25;
/// Wider than a solid line because it is so much shorter: a dash as thin as the line it stands in
/// for reads as a worn-away line rather than a mark travelling along one.
const EDGE_DASH_WIDTH: f32 = 1.6;
/// Slots a second: at `1.0` a dash takes a second to reach where the dash ahead of it started. The
/// marching carries the direction on its own, so nothing about a still frame points anywhere.
const EDGE_DASH_SPEED: f32 = 0.8;
/// sRGB, which is what both the clear and [`panorama_texture`]'s glows over it want.
pub(super) const BACKGROUND_COLOR: [f32; 3] = [0.03, 0.03, 0.05];
/// Side of the backdrop's repeating tile in texels, and the spacing of the lattice of glows inside
/// it. Four glows lie across the tile, each with its own brightness, so the lattice does not read
/// as one cell stamped over and over. Its size on screen is [`PANORAMA_TILE_PIXELS`].
const PANORAMA_TILE_TEXELS: usize = 256;
const PANORAMA_CELL_TEXELS: usize = 64;
/// How far a glow reaches, in texels, and how sharply it fades over that reach. Most of a cell, so
/// neighbouring glows almost meet.
const PANORAMA_GLOW_RADIUS_TEXELS: f32 = 52.0;
const PANORAMA_GLOW_FALLOFF: f32 = 2.2;
/// Which norm measures the distance to a glow's centre. Between the diamond a norm of 1 draws and
/// the circle a norm of 2 draws, which is the rounded square of the reference backdrop.
const PANORAMA_GLOW_NORM: f32 = 1.5;
/// Blue and dim on purpose: it has to stay visible past a graph whose own edges are only a little
/// brighter, without competing with them.
const PANORAMA_GLOW_COLOR: [f32; 3] = [3.0 / 255.0, 6.0 / 255.0, 42.0 / 255.0];
/// Peak brightness per cell, as a fraction of [`PANORAMA_GLOW_COLOR`]. Irregular, and belonging to
/// the tile rather than the screen, so the pattern repeats seamlessly.
const PANORAMA_CELL_PEAKS: [[f32; 4]; 4] = [
    [0.55, 0.30, 0.85, 0.40],
    [0.35, 1.00, 0.45, 0.65],
    [0.90, 0.50, 0.70, 0.30],
    [0.40, 0.75, 0.35, 0.95],
];
/// Side of the tile on screen, in logical pixels: independent of the texel size, so the backdrop
/// keeps its scale on a high-density display rather than shrinking to half of it.
const PANORAMA_TILE_PIXELS: f32 = 260.0;
/// How far the backdrop slides, in tiles, per unit of the view direction's horizontal or vertical
/// component.
///
/// Small on purpose: a panorama sits far enough away that a turn toward it barely shifts it, and
/// that faint drift is what makes the graph read as turning in front of a scene rather than a
/// decal.
const PANORAMA_PARALLAX: f32 = 0.12;
/// Brightness of the connection into a world with nothing behind it, against the one into the world
/// the whole game is behind. The connections carry the depth ramp, so this is the range the ramp is
/// drawn over rather than a color of its own. See [`edge_colors`].
const EDGE_LEAF_BRIGHTNESS: f32 = 0.6;
/// How far the connection colors move toward even apparent brightness: at 0 the ramp keeps its own
/// stops, at 1 every stop takes the brightness of the dimmest.
///
/// The ramp is picked for hue, and hue drags brightness with it -- most of what the eye reads as
/// brightness is the green channel, so the green stop in the middle looks glaring beside the
/// magenta at the end even though both are nominally full. Levelled far enough that no stop shouts
/// over the others, and not so far that the ramp flattens into four shades of one brightness.
const EDGE_LUMINANCE_EVENNESS: f32 = 0.5;
/// Warm and bright, for the edges of the route back to the origin world.
const ROUTE_COLOR: Srgba = Srgba::new(255, 242, 194, 255);
/// Brightness left to everything off the highlighted route. Low enough to set the surrounding
/// graph behind the route, high enough that it stays readable as context.
const DIMMED_BRIGHTNESS: f32 = 0.25;
/// How much of its own picture is added back over a world's node while a list row points at it.
///
/// Added rather than multiplied, because a lit node is already painted white and there is nothing
/// left to multiply by. High enough to pick the node out of a crowd, low enough that the
/// screenshot is still a screenshot.
const POINTED_BRIGHTNESS: f32 = 0.85;
/// Deliberately off the distance ramp: the worlds the origin cannot reach are at no distance at
/// all rather than at a large one, and reading them as the far end of the ramp would be a lie.
const UNREACHED_COLOR: Srgba = Srgba::new(110, 110, 120, 255);
/// How long the panel counts frames over before saying how many there were, in milliseconds. The
/// One one-way connection's dashes, less where along the connection each of them has marched to.
///
/// The marching moves a dash along its own slot and changes nothing else about it, so a settled
/// layout can keep the orientation and the size and write only the position. That matters because
/// this is the one piece of geometry rebuilt on every frame however still the graph is -- the
/// marching is the whole point of the dashes -- and there are [`EDGE_DASHES`] of them per
/// connection, half again as many instances as there are solid lines in the whole graph.
pub(super) struct DashRun {
    /// Where the run starts, clear of the picture drawn on the world at that end.
    origin: Vec3,
    /// From there to where the dash one slot ahead starts. A dash stands at `origin + travel * at`
    /// for its own `at` in `0.0..1.0`.
    travel: Vec3,
    /// The dash's orientation and size: everything about its transformation except the position,
    /// which is written into the fourth column. Scaled to nothing for a connection whose ends are
    /// closer together than their own pictures, there being no run to draw in.
    basis: Mat4,
}
/// A free function rather than a method because it reads the dump the app holds and hands back
/// what the app is about to hold beside it: two borrows of one [`App`] that do not overlap in fact
/// and cannot both be taken in a method.
pub(super) fn entities(
    dump: &world::Dump,
    before: Option<&Before>,
    ctx: &WindowedContext,
) -> AppEntities {
    let rng = Rng(0x5eed_1337);

    let worlds = &dump.worlds;
    // Depth computed once for both the colors and, in layered mode, the layer each world is
    // pinned to, so the two agree.
    let routes = world::canonical_routes(worlds);
    let deepest = routes.depth.iter().flatten().copied().max().unwrap_or(0);
    let furthest = deepest.max(1) as f32;
    // Sized by how much of the graph hangs off each world. The logarithm grows the size with the
    // order of magnitude of what a world leads to rather than with the count, so a deep world with
    // a handful behind it lands within reach of a shallow one with a hundred.
    let descendants = routes.descendant_counts();
    let node_radii: Vec<_> = descendants
        .iter()
        .map(|&descendants| {
            let growth = (1.0 + descendants as f32).ln() / (1.0 + NODE_HUB_DESCENDANTS).ln();
            NODE_LEAF_RADIUS + (NODE_HUB_RADIUS - NODE_LEAF_RADIUS) * growth
        })
        .collect();

    // How the graph before this one was being read, or how the last run was left. Either way the
    // person is put back where they were.
    let layout = before.map_or_else(Layout::remembered, |before| before.layout);
    let Layout {
        dimensions,
        hub_repulsion,
        link_reach,
        ..
    } = layout;
    let Solve {
        dag_level_distance,
        dag_level_slack,
        force_charge,
        link_distance_max,
    } = layout.parameters();
    let mut graph = Graph::new(SimulationParameters {
        dimensions,
        dag_level_distance,
        dag_level_slack,
        force_charge,
        link_distance_max,
        settle_after: Some(SETTLE_AFTER),
        ..Default::default()
    });
    // Positions come from `scatter` below, so the initial layout and a restart agree.
    let nodes: Vec<_> = routes
        .depth
        .iter()
        .enumerate()
        .map(|(world, depth)| {
            graph.add_node(NodeData {
                // Worlds the origin cannot reach have no depth to share a layer with, so they get
                // one of their own past the deepest that does.
                level: depth.map_or(furthest + 1.0, |depth| depth as f32),
                mass: node_mass(node_radii[world], hub_repulsion),
                ..Default::default()
            })
        })
        .collect();
    let connections = world::connections(worlds);
    // Counted off the same connections the edges are built from, so the panel calls a world a
    // junction on exactly the lines it draws for it.
    let mut degrees = vec![0; worlds.len()];
    // What fixes how many dash instances there are. See [`EDGE_DASHES`].
    let mut dashed = 0;
    for (from, steps) in connections.iter().enumerate() {
        for step in steps {
            // Once per connection rather than per world it joins: two springs on the same two
            // nodes would pull them together twice as hard.
            if step.world <= from {
                continue;
            }
            let to = step.world;
            // Stored from the end a player can leave, so the dashes march the way they can walk.
            // Walkable both ways or neither keeps the dump's own order: nothing drawn on such a
            // connection means anything to a direction.
            let (from, to) = if step.out.is_none() && step.back.is_some() {
                (to, from)
            } else {
                (from, to)
            };
            dashed += usize::from(step.one_way());
            graph.add_edge(
                nodes[from],
                nodes[to],
                EdgeData {
                    reach: edge_reach(
                        step.one_way(),
                        node_radii[from].max(node_radii[to]),
                        hub_repulsion,
                    ),
                    user_data: step.one_way(),
                },
            );
            degrees[from] += 1;
            degrees[to] += 1;
        }
    }

    // Not drawn on the worlds themselves, which carry pictures: the connections wear the depth.
    let depth_colors: Vec<_> = routes
        .depth
        .iter()
        .map(|depth| match depth {
            Some(depth) => distance_color(*depth as f32 / furthest),
            None => UNREACHED_COLOR,
        })
        .collect();
    let edge_colors = edge_colors(&graph, &routes, &descendants, &depth_colors);
    // The worlds the placeholder is drawn on. Read off the same thing the atlas cells are, so the
    // two cannot disagree.
    let unvisited: Vec<usize> = worlds
        .iter()
        .enumerate()
        .filter_map(|(world, it)| it.cell().is_none().then_some(world))
        .collect();
    let titles: Vec<world::Title> = worlds.iter().map(world::World::titles).collect();
    let maps: Vec<Vec<world::Map>> = worlds.iter().map(world::World::maps).collect();
    // Built once here: both group every world, which is not work a frame can afford.
    let (authors, author_of) = dump.authors();
    let versions = dump.versions();
    // White, so a picture reaches the screen as itself. Only a selection ever changes them.
    let thumbnail_instances = Instances {
        transformations: vec![Mat4::identity(); worlds.len()],
        colors: Some(vec![Srgba::WHITE; worlds.len()]),
        ..Default::default()
    };
    // One instance per solid line, and [`EDGE_DASHES`] per one-way one. The colors are left to
    // the first `repaint` below, where the same fan-out has to live anyway.
    let solid = edge_colors.len() - dashed;
    let edge_instances = Instances {
        transformations: vec![Mat4::identity(); solid],
        colors: Some(vec![Srgba::WHITE; solid]),
        ..Default::default()
    };
    let dash_instances = Instances {
        transformations: vec![Mat4::identity(); dashed * EDGE_DASHES],
        colors: Some(vec![Srgba::WHITE; dashed * EDGE_DASHES]),
        ..Default::default()
    };
    let backdrop = ColorMaterial {
        color: Srgba::WHITE,
        texture: Some(Texture2DRef::from_cpu_texture(ctx, &panorama_tile())),
        // Colour only and no depth test: the quad stands in for the cleared background, so it
        // must neither occlude the graph nor be occluded by the cleared depth buffer.
        render_states: RenderStates {
            write_mask: WriteMask::COLOR,
            depth_test: DepthTest::Always,
            ..Default::default()
        },
        is_transparent: false,
    };
    // Quads rather than cubes, turned to face the camera every frame: a picture has to be seen
    // square on, and a cube's uv coordinates unwrap its six faces across the image rather than
    // giving each face the whole of it. No texture yet -- the atlas is still on its way in.
    let thumbnails = Gm::new(
        InstancedMesh::new(ctx, &thumbnail_instances, &CpuMesh::square()),
        ColorMaterial::default(),
    );
    // The same quad again, added to what is on the screen rather than covering it: a lit node is
    // drawn white, so no multiplier is left that would brighten it. It samples the same atlas.
    let glow = Gm::new(
        Mesh::new(ctx, &CpuMesh::square()),
        ColorMaterial {
            color: scaled(Srgba::WHITE, POINTED_BRIGHTNESS),
            texture: None,
            render_states: RenderStates {
                // Depth read but not written: the quad stands in front of the node it brightens,
                // and a node genuinely in front of that one still hides it.
                write_mask: WriteMask::COLOR,
                // Equal passes because at these distances the lift toward the camera is smaller
                // than one step of the depth buffer. The quad is never *behind* the node it
                // copies, so the two depths quantise to either less or the same, and a strict
                // test drops the glow on exactly the frames the rounding went the other way.
                depth_test: DepthTest::LessOrEqual,
                blend: Blend::ADD,
                ..Default::default()
            },
            is_transparent: true,
        },
    );
    let edges = Gm::new(
        InstancedMesh::new(ctx, &edge_instances, &CpuMesh::cylinder(EDGE_SIDES)),
        ColorMaterial::default(),
    );
    // The same cylinder as a solid line, placed the same way, only shorter.
    let dashes = Gm::new(
        InstancedMesh::new(ctx, &dash_instances, &CpuMesh::cylinder(EDGE_SIDES)),
        ColorMaterial::default(),
    );

    // Before the graph takes the names over.
    let mut unknown = vec![false; worlds.len()];
    for &world in &unvisited {
        unknown[world] = true;
    }
    let untaken = untaken(&connections, &unknown);
    let arrivals = Arrivals::new(
        &titles
            .iter()
            .map(|title| title.en.as_str())
            .collect::<Vec<_>>(),
        &unknown,
        &routes.depth,
        before,
    );
    let arriving = arrivals.arriving().is_some();

    let mut data = AppEntities {
        graph,
        gesture: None,
        rng,
        routes,
        titles,
        maps,
        authors,
        author_of,
        versions,
        selected: None,
        deepest,
        right_press: None,
        menu: None,
        cursor: None,
        hover: None,
        pointed: None,
        // A graph with worlds coming into it moves the camera onto them. A pan or an orbit takes
        // it back at any point.
        framing: arriving,
        frame_route: false,
        edge_colors,
        node_radii,
        hub_repulsion,
        link_reach,
        descendants,
        connections,
        untaken,
        degrees,
        backdrop,
        thumbnails,
        glow,
        edges,
        dashes,
        thumbnail_instances,
        edge_instances,
        dash_instances,
        dash_runs: Vec::new(),
        dash_runs_stale: true,
        recolored: true,
        glowing: None,
        atlas: Some(thumbnails::load()),
        sheet: None,
        cells: worlds.iter().map(world::World::cell).collect(),
        packed: dump.packed,
        unvisited: unvisited.clone(),
        // White until the first `place_unvisited`, the same fan-out the thumbnails' colors take.
        unvisited_quads: Instances {
            transformations: vec![Mat4::identity(); unvisited.len()],
            colors: Some(vec![Srgba::WHITE; unvisited.len()]),
            ..Default::default()
        },
        detail: detail::Detail::new(
            worlds.iter().map(|world| world.image.clone()).collect(),
            // Or one on its way out from under a placeholder, which needs the same picture for as
            // long as it takes to shrink away.
            !unvisited.is_empty() || arrivals.turning(),
        ),
        dash_phase: 0.0,
        // Rebuilt on the first frame either way, a fresh layout not having settled.
        billboard: Mat4::identity(),
        arrivals,
    };
    // The instance colors have been sized above but not written.
    data.repaint();
    scatter(&mut data);
    // Over the top of the scatter, so a graph replacing another carries on from where that one
    // stood instead of restarting.
    if let Some(before) = before {
        resume_from(&mut data, &before.standing);
    }
    data
}
impl AppEntities {
    /// Rewrites the instance colors for the current selection: the steps between the lit worlds
    /// are lit brighter still, and everything else is dimmed.
    ///
    /// Uploads on its own rather than waiting for [`Self::rebuild_instances`], which a settled
    /// graph never reaches.
    pub(super) fn repaint(&mut self) {
        self.recolored = true;
        let mut on_route = vec![false; self.titles.len()];
        for node in self.highlighted() {
            on_route[node] = true;
        }
        let lit = self.selected.is_some();

        // White leaves a picture as itself, so a world is only ever painted to push it back behind
        // what a selection lights.
        let colors = self.thumbnail_instances.colors.as_mut().unwrap();
        for (color, &on_route) in colors.iter_mut().zip(&on_route) {
            *color = if !lit || on_route {
                Srgba::WHITE
            } else {
                dim(Srgba::WHITE)
            };
        }

        let (routes, on_route) = (&self.routes.parents, &on_route);
        let (solid, dashed, base) = (
            self.edge_instances.colors.as_mut().unwrap(),
            self.dash_instances.colors.as_mut().unwrap(),
            &self.edge_colors,
        );
        // The same order, and the same split between the two, that [`Self::rebuild_instances`] and
        // [`Self::march_dashes`] write the transformations in.
        let (mut edge, mut line, mut dash) = (0, 0, 0);
        self.graph.visit_edges(|a, b, data| {
            let (a, b) = (a.index().index(), b.index().index());
            // Whether this edge is a canonical step: from one of its ends to that end's parent.
            let step = routes[a] == Some(b) || routes[b] == Some(a);
            let color = match self.selected {
                None => base[edge],
                // The one line it is about, and not even the step home from either of its ends,
                // which is a different way through the graph than the one the reader asked for.
                Some(Highlight::Connection(at, far)) => {
                    if (a, b) == (at, far) || (a, b) == (far, at) {
                        ROUTE_COLOR
                    } else {
                        dim(base[edge])
                    }
                }
                // A layer is a shell rather than a walk, so what is worth seeing across it is
                // where it is stitched to itself, not the step each world takes home. Both ends
                // being lit is the whole test, and such an edge is never a canonical step -- a
                // parent is always exactly one depth in -- so it keeps its own distance color and
                // only escapes the dimming.
                Some(Highlight::Layer(_)) if on_route[a] && on_route[b] => base[edge],
                // Both ends being lit is not enough: it also has to be the step from one of them
                // to that end's parent, or a shortcut between two distant points of a route would
                // light up as if the walk went through it.
                Some(_) if step && on_route[a] && on_route[b] => ROUTE_COLOR,
                Some(_) => dim(base[edge]),
            };
            // Every dash of a one-way connection takes the same color, so it reads as the one
            // line it stands for.
            if data.user_data {
                dashed[dash..dash + EDGE_DASHES].fill(color);
                dash += EDGE_DASHES;
            } else {
                solid[line] = color;
                line += 1;
            }
            edge += 1;
        });

        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.edges.set_instances(&self.edge_instances);
        self.dashes.set_instances(&self.dash_instances);
    }

    fn edge_radius(&self) -> f32 {
        EDGE_RADIUS
    }

    pub(super) fn rebuild_instances(&mut self, camera: &Camera) {
        let billboard = billboard(camera);
        let radius = self.edge_radius();
        let thumbnails = &mut self.thumbnail_instances.transformations;
        thumbnails.clear();
        let arrivals = &self.arrivals;
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            // A world that has not arrived is drawn at none of its size, which is the whole of how
            // an arrival looks: everything hung off the node quads is copied from these.
            let radius = self.node_radii[world] * arrivals.grown(world);
            // Wider than tall, in the shape of the pictures: a square node would either crop a
            // third off every screenshot or stretch them all.
            thumbnails.push(
                Mat4::from_translation(world_pos(node.position()))
                    * billboard
                    * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0),
            );
        });
        let edges = &mut self.edge_instances.transformations;
        edges.clear();
        self.graph.visit_edges(|a, b, data| {
            // The one-way connections belong to [`Self::march_dashes`], which rebuilds them every
            // frame rather than only the frames the layout moves in.
            if data.user_data {
                return;
            }
            let (from, to) = (world_pos(a.position()), world_pos(b.position()));
            let dir = to - from;
            // As thin as the later of the two worlds it joins is small, so a line is not drawn to
            // somewhere that is not there yet.
            let radius = radius
                * arrivals
                    .grown(a.index().index())
                    .min(arrivals.grown(b.index().index()));
            edges.push(
                Mat4::from_translation(from)
                    * rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), dir.normalize())
                    * Mat4::from_nonuniform_scale(dir.magnitude(), radius, radius),
            );
        });
        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.edges.set_instances(&self.edge_instances);
        self.billboard = billboard;
    }

    /// Stands the placeholder over every world the player has not been to.
    ///
    /// Taken from the nodes' own quads rather than worked out again, so a placeholder is exactly
    /// where and how big the world it stands for is, and dims with it. Lifted toward the camera as
    /// a magnified picture is, two coplanar quads otherwise leaving the depth test to pick between
    /// them per pixel; composed onto the node's own transformation, translations commuting.
    ///
    /// Every frame, like the magnified pictures: what it copies is rebuilt whenever the layout
    /// moves, the camera turns, or a selection repaints.
    pub(super) fn place_unvisited(&mut self, context: &Context, camera: &Camera) {
        if self.unvisited.is_empty() && !self.arrivals.turning() {
            return;
        }
        let forward = camera.view_direction();
        let nodes = &self.thumbnail_instances;
        let painted = nodes.colors.as_ref();
        // The worlds with no picture of their own, and the ones still shrinking out from under
        // the placeholder.
        let wearing = self.unvisited.iter().copied().chain(self.arrivals.veiled());
        let (transformations, colors) = (
            &mut self.unvisited_quads.transformations,
            self.unvisited_quads.colors.as_mut().unwrap(),
        );
        transformations.clear();
        colors.clear();
        for world in wearing {
            let Some(node) = nodes.transformations.get(world) else {
                continue;
            };
            transformations.push(
                Mat4::from_translation(-forward * (self.node_radii[world] * detail::LIFT)) * node,
            );
            colors.push(painted.map_or(Srgba::WHITE, |painted| painted[world]));
        }
        self.detail.place_unvisited(context, &self.unvisited_quads);
    }

    /// Every frame, unlike the rest of the geometry: the marching is the whole point of the
    /// dashes, and a settled layout is exactly when it has to carry on regardless.
    /// Works out where each connection's dashes run, which is everything about them except how
    /// far along they have marched. See [`DashRun`].
    fn lay_dash_runs(&mut self) {
        let radius = self.edge_radius() * EDGE_DASH_WIDTH;
        let radii = &self.node_radii;
        let arrivals = &self.arrivals;
        let runs = &mut self.dash_runs;
        runs.clear();
        self.graph.visit_edges(|a, b, data| {
            if !data.user_data {
                return;
            }
            // Thin with whichever end is later, as the plain lines are.
            let radius = radius
                * arrivals
                    .grown(a.index().index())
                    .min(arrivals.grown(b.index().index()));
            let (from, to) = (world_pos(a.position()), world_pos(b.position()));
            let dir = to - from;
            let along = rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), dir.normalize());
            // The run stops at the pictures rather than running under them: a dash reaching a
            // world's own place would stand inside its quad and show through it. Held off by the
            // half-width, the furthest a quad ever reaches from the world it draws, so a dash
            // clears it whichever way the billboard has been turned. See
            // [`Self::rebuild_instances`], which draws the quads that wide.
            let clear = |node: usize| radii[node] * arrivals.grown(node) * thumbnails::ASPECT;
            let start = clear(a.index().index());
            let span = dir.magnitude() - start - clear(b.index().index());
            // Two worlds closer together than their own pictures leave nothing to draw a run in.
            // Collapsed rather than skipped: the count of dashes is fixed when the graph is built
            // and the colors are written against it.
            if span <= 0.0 {
                runs.push(DashRun {
                    origin: vec3(0.0, 0.0, 0.0),
                    travel: vec3(0.0, 0.0, 0.0),
                    basis: Mat4::from_scale(0.0),
                });
                return;
            }
            // Each dash fills the front of its own slot, the gap behind it being what makes the
            // run read as dashed and what a dash marching off the far end comes back into at the
            // near one. The run is one dash shorter than the span, so the leading dash ends where
            // the span does rather than reaching into the picture.
            let run = span / (1.0 + EDGE_DASH_FILL / EDGE_DASHES as f32);
            let length = run * EDGE_DASH_FILL / EDGE_DASHES as f32;
            let unit = dir / dir.magnitude();
            runs.push(DashRun {
                origin: from + unit * start,
                travel: unit * run,
                basis: along * Mat4::from_nonuniform_scale(length, radius, radius),
            });
        });
        self.dash_runs_stale = false;
    }

    pub(super) fn march_dashes(&mut self, dt: f32) {
        // Kept inside a single slot: past the end of one, every dash stands where the dash ahead
        // of it stood, so the phase can simply start over.
        self.dash_phase = (self.dash_phase + dt * EDGE_DASH_SPEED).fract();
        if self.dash_instances.transformations.is_empty() {
            return;
        }
        if self.dash_runs_stale {
            self.lay_dash_runs();
        }
        let phase = self.dash_phase;
        let (slots, _) = self
            .dash_instances
            .transformations
            .as_chunks_mut::<EDGE_DASHES>();
        for (run, dashes) in self.dash_runs.iter().zip(slots) {
            for (dash, out) in dashes.iter_mut().enumerate() {
                let at = ((dash as f32 + phase) / EDGE_DASHES as f32).fract();
                let at = run.origin + run.travel * at;
                // Composing the translation by hand: `basis` carries no translation of its own, so
                // the product is exactly `basis` with the position written into the last column.
                *out = run.basis;
                out.w = at.extend(1.0);
            }
        }
        self.dashes.set_instances(&self.dash_instances);
    }

    /// Points the thumbnail quads at their own cells of the atlas once it has arrived, and hands
    /// egui a copy for the catalog. Nothing is drawn until then, or ever if it cannot be had.
    pub(super) fn receive_atlas(&mut self, context: &Context, egui: &egui::Context) {
        let Some(loaded) = self.atlas.as_ref().and_then(fetch::Pending::take) else {
            return;
        };
        self.atlas = None;
        // Both failures are logged where they are found, and both leave the graph drawn as it was
        // before thumbnails existed.
        let Some(atlas) = loaded else { return };
        self.sheet = thumbnails::Sheet::new(egui, self.packed, &atlas);
        let Some(cells) = thumbnails::cells(self.packed, &self.cells, &atlas) else {
            return;
        };
        self.thumbnail_instances.texture_transformations = Some(cells);
        self.thumbnails.set_instances(&self.thumbnail_instances);
        // One upload, sampled by both: a [`Texture2DRef`] is a handle over a shared texture, so
        // the clone costs a Mat3 and an atomic rather than a second copy of the atlas.
        let atlas = Texture2DRef::from_cpu_texture(context, &atlas);
        self.glow.material.texture = Some(atlas.clone());
        // Last, being what [`Self::drawn_thumbnails`] reads to decide the quads are drawable.
        self.thumbnails.material.texture = Some(atlas);
    }

    /// `None` before there is an atlas to sample: an untextured [`ColorMaterial`] would paint its
    /// base color flat over every world.
    pub(super) fn drawn_thumbnails(&self) -> Option<&dyn Object> {
        self.thumbnails
            .material
            .texture
            .is_some()
            .then_some(&self.thumbnails as &dyn Object)
    }

    /// Stands the glow quad over the node a list row is pointing at, on that world's cell.
    ///
    /// The node's own quad is taken rather than worked out again: it is already turned to face the
    /// camera and already the right size, and standing the glow anywhere else would brighten a
    /// different shape than the one on screen. Only the lift toward the camera is added, so the
    /// two are not coplanar -- twice [`detail::LIFT`], because a magnified world already sits one
    /// lift in front and the glow is over that too.
    ///
    /// A lift proportional to a node is small next to how far away the graph is read from, so it
    /// is not on its own enough to clear the depth buffer's rounding. What makes that harmless is
    /// the `depth_test` the quad is drawn under.
    pub(super) fn aim_glow(&mut self, camera: &Camera) {
        let Some(world) = self.pointed else { return };
        let Some(cells) = self.thumbnail_instances.texture_transformations.as_ref() else {
            return;
        };
        let Some(quad) = self.thumbnail_instances.transformations.get(world) else {
            return;
        };
        let lift = camera.view_direction() * (self.node_radii[world] * detail::LIFT * 2.0);
        let quad = Mat4::from_translation(-lift) * quad;
        self.glow.set_transformation(quad);
        if let Some(texture) = self.glow.material.texture.as_mut() {
            texture.transformation = cells[world];
        }
    }

    /// `None` without a world to brighten or an atlas to brighten it out of.
    pub(super) fn drawn_glow(&self) -> Option<&dyn Object> {
        let ready = self.pointed.is_some() && self.glow.material.texture.is_some();
        ready.then_some(&self.glow as &dyn Object)
    }

    /// Every world the view is asking more of than the atlas holds, widest on screen first.
    ///
    /// What [`detail`] is driven from, and the reason it needs no view of its own: a world is here
    /// because its node is on screen and drawn wider than [`detail::SWITCH_PIXELS`], and it
    /// carries the quad it would have been drawn on so the full picture lands over its thumbnail.
    ///
    /// A world in the middle of becoming itself is asked for at the size it is about to be rather
    /// than the size it is now, so the picture is fetched while the placeholder is still shrinking
    /// away and what grows back is the world's own from the first frame, rather than a thumbnail
    /// that pops sharp part way up.
    pub(super) fn magnified(&self, camera: &Camera, viewport: Viewport) -> Vec<detail::Magnified> {
        let billboard = billboard(camera);
        let forward = camera.view_direction();
        let colors = self.thumbnail_instances.colors.as_ref();
        let across = camera.right_direction().normalize();
        let mut magnified = Vec::new();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            let position = world_pos(node.position());
            // Behind the camera a projection says nothing useful, and off the edge of the window
            // there is nothing to sharpen.
            if (position - camera.position()).dot(forward) <= 0.0 {
                return;
            }
            let center = camera.pixel_at_position(position);
            if center.x < 0.0
                || center.y < 0.0
                || center.x > viewport.width as f32
                || center.y > viewport.height as f32
            {
                return;
            }
            let full = self.node_radii[world];
            let grown = self.arrivals.grown(world);
            // Ranked and admitted at the size it is settling at, drawn at the size it is now.
            let width = drawn_width(camera, across, full, position, center);
            if width < detail::SWITCH_PIXELS {
                return;
            }
            let radius = full * grown;
            magnified.push(detail::Magnified {
                world,
                width,
                transformation: Mat4::from_translation(
                    position - forward * (radius * detail::LIFT),
                ) * billboard
                    * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0),
                color: colors.map_or(Srgba::WHITE, |colors| colors[world]),
            });
        });
        magnified.sort_by(|a, b| b.width.total_cmp(&a.width));
        magnified
    }
}
/// A lattice of soft blue glows on the background color, in the style of the game's own panoramas.
///
/// Held in linear color, which is what the shader's own sRGB encoding expects on the way out, and
/// at half precision because eight bits of linear is not enough for glows this dim: the darkest
/// steps land far enough apart once encoded to band, and dithering them only trades the banding
/// for grain across the whole flat backdrop.
fn panorama_tile() -> CpuTexture {
    let cells = PANORAMA_TILE_TEXELS / PANORAMA_CELL_TEXELS;
    let mut data = Vec::with_capacity(PANORAMA_TILE_TEXELS * PANORAMA_TILE_TEXELS);
    for y in 0..PANORAMA_TILE_TEXELS {
        for x in 0..PANORAMA_TILE_TEXELS {
            // Every cell whose glow could reach here, the tile's own neighbours included, so the
            // glows straddling an edge match up when the tile repeats.
            let mut glow: f32 = 0.0;
            for cell_y in -1..=1 {
                for cell_x in -1..=1 {
                    let center = |along: usize, cell: i32| {
                        (along as i32 / PANORAMA_CELL_TEXELS as i32 + cell)
                            * PANORAMA_CELL_TEXELS as i32
                            + PANORAMA_CELL_TEXELS as i32 / 2
                    };
                    let (center_x, center_y) = (center(x, cell_x), center(y, cell_y));
                    let axis = |at: usize, center: i32| (at as f32 + 0.5 - center as f32).abs();
                    let distance = (axis(x, center_x).powf(PANORAMA_GLOW_NORM)
                        + axis(y, center_y).powf(PANORAMA_GLOW_NORM))
                    .powf(1.0 / PANORAMA_GLOW_NORM);
                    if distance >= PANORAMA_GLOW_RADIUS_TEXELS {
                        continue;
                    }
                    let cell = |center: i32| {
                        (center.div_euclid(PANORAMA_CELL_TEXELS as i32)).rem_euclid(cells as i32)
                            as usize
                    };
                    let peak = PANORAMA_CELL_PEAKS[cell(center_y)][cell(center_x)];
                    glow = glow.max(
                        peak * (1.0 - distance / PANORAMA_GLOW_RADIUS_TEXELS)
                            .powf(PANORAMA_GLOW_FALLOFF),
                    );
                }
            }

            data.push(std::array::from_fn(|channel| {
                let shown = BACKGROUND_COLOR[channel] + glow * PANORAMA_GLOW_COLOR[channel];
                f16::from_f32(srgb_to_linear(shown.min(1.0)))
            }));
        }
    }

    CpuTexture {
        name: "panorama".to_owned(),
        data: TextureData::RgbF16(data),
        width: PANORAMA_TILE_TEXELS as u32,
        height: PANORAMA_TILE_TEXELS as u32,
        ..Default::default()
    }
}

/// The shader applies sRGB again on the way to the screen, so what a texel means is what it is
/// worth once this has been taken off it.
fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// Scaled so a tile keeps a fixed size in logical pixels whatever the window and the display
/// density, and slid by [`PANORAMA_PARALLAX`] as the camera turns. Where the camera *is* does not
/// enter: a panorama is far enough away that only the direction it is seen from moves it, which
/// keeps the backdrop from reading as a plane the graph slides over.
///
/// The offset is taken straight from the view direction rather than from a yaw and a pitch, so it
/// stays continuous all the way around instead of snapping where an angle wraps.
pub(super) fn panorama_transform(viewport: Viewport, device_pixel_ratio: f32, view: Vec3) -> Mat3 {
    let tile = PANORAMA_TILE_PIXELS * device_pixel_ratio;
    // Sampling further along an axis pulls the backdrop the opposite way on screen, so turning
    // the camera right drifts the backdrop left, as something in the distance does.
    let drift = vec2(view.x, view.y) * PANORAMA_PARALLAX;
    Mat3::from_translation(drift)
        * Mat3::from_nonuniform_scale(viewport.width as f32 / tile, viewport.height as f32 / tile)
}
/// A normalized distance from the origin on a cyan-green-amber-magenta ramp, so how far a world
/// sits from the start of the game reads off its color. The stops stay bright, to hold up against
/// [`BACKGROUND_COLOR`].
fn distance_color(distance: f32) -> Srgba {
    const STOPS: [[f32; 3]; 4] = [
        [0.10, 0.90, 1.00],
        [0.35, 1.00, 0.45],
        [1.00, 0.82, 0.25],
        [1.00, 0.35, 0.75],
    ];
    let scaled = distance.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let stop = (scaled as usize).min(STOPS.len() - 2);
    let blend = scaled - stop as f32;
    let channel =
        |c: usize| ((STOPS[stop][c] + (STOPS[stop + 1][c] - STOPS[stop][c]) * blend) * 255.0) as u8;
    Srgba::new(channel(0), channel(1), channel(2), 255)
}

/// Pushes a color back toward the background, for everything off the highlighted route.
fn dim(color: Srgba) -> Srgba {
    scaled(color, DIMMED_BRIGHTNESS)
}

/// The eye's own weighting of the channels, which is what [`EDGE_LUMINANCE_EVENNESS`] levels the
/// distance ramp by.
fn luminance(color: Srgba) -> f32 {
    (0.2126 * color.r as f32 + 0.7152 * color.g as f32 + 0.0722 * color.b as f32) / 255.0
}

/// Keeps the hue and the alpha.
fn scaled(color: Srgba, brightness: f32) -> Srgba {
    let scale = |channel: u8| (channel as f32 * brightness) as u8;
    Srgba::new(scale(color.r), scale(color.g), scale(color.b), color.a)
}

/// Per connection, the color it carries when nothing is selected.
///
/// The depth ramp lives on the connections rather than the worlds, which carry pictures: a
/// connection wears the color of the world it is walked *from*, so following a line outward from
/// the origin walks the ramp. Its brightness says how much of the game lies through it, which
/// makes the trunk of the route tree stand out of its twigs -- read through a logarithm for the
/// same reason the node sizes are.
///
/// Which end is walked from is the canonical routes' answer where they have one. A connection that
/// is nobody's route home -- a shortcut across the tree -- is taken as walked from its shallower
/// end, which is the direction a player meets it in anyway.
fn edge_colors(
    graph: &Graph,
    routes: &world::Routes,
    descendants: &[u32],
    depth_colors: &[Srgba],
) -> Vec<Srgba> {
    // Against the busiest world there is, so the brightest connection is the one the whole game
    // hangs off rather than one at an arbitrary count.
    let busiest = (1.0 + descendants.iter().copied().max().unwrap_or(0) as f32).ln();
    // The dimmest the ramp's own stops reach, so the correction only ever darkens and no channel
    // has to clip. Sampled rather than restated, the ramp being [`distance_color`]'s to define.
    const SAMPLES: u32 = 64;
    let floor = (0..=SAMPLES)
        .map(|step| luminance(distance_color(step as f32 / SAMPLES as f32)))
        .fold(f32::INFINITY, f32::min);
    let mut colors = Vec::with_capacity(graph.edge_count());
    graph.visit_edges(|a, b, _| {
        let (a, b) = (a.index().index(), b.index().index());
        // Unreachable worlds are deeper than any depth rather than shallower than every one,
        // which is what `Option`'s own order would make of them.
        let depth = |world: usize| routes.depth[world].unwrap_or(u32::MAX);
        let from = match (routes.parents[b], routes.parents[a]) {
            (Some(parent), _) if parent == a => a,
            (_, Some(parent)) if parent == b => b,
            _ if depth(a) <= depth(b) => a,
            _ => b,
        };
        let to = if from == a { b } else { a };
        let reach = (1.0 + descendants[to] as f32).ln() / busiest.max(f32::MIN_POSITIVE);
        // Never above 1: a color already dimmer than the ramp's floor -- [`UNREACHED_COLOR`],
        // which is off the ramp entirely -- is left alone rather than lifted onto it.
        let even = (floor / luminance(depth_colors[from]).max(f32::MIN_POSITIVE)).min(1.0);
        colors.push(scaled(
            depth_colors[from],
            (EDGE_LEAF_BRIGHTNESS + (1.0 - EDGE_LEAF_BRIGHTNESS) * reach)
                * (1.0 - EDGE_LUMINANCE_EVENNESS * (1.0 - even)),
        ));
    });
    colors
}

#[cfg(test)]
mod tests {
    #[cfg(target_family = "wasm")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use three_d::renderer::*;

    #[test]
    fn writing_a_dash_position_is_composing_a_translation() {
        let along =
            rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), vec3(0.3, -0.7, 0.5).normalize());
        let basis = along * Mat4::from_nonuniform_scale(0.4, 0.08, 0.08);
        let at = vec3(3.0, -2.0, 11.0);
        let mut written = basis;
        written.w = at.extend(1.0);
        assert_eq!(Mat4::from_translation(at) * basis, written);
    }
}
