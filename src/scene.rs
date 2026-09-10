//! The graph as something drawn: the worlds, their connections, the dashes that march along the
//! one-way ones, and the backdrop behind all of it. See [`entities`] and [`AppEntities`].

use super::*;

/// How thick a connection is drawn.
const EDGE_RADIUS: f32 = 0.05;
/// Sides of the tube a connection is drawn as. Tens of thousands of them come out a pixel wide, so
/// the cost is triangle setup rather than fill and the sides are very nearly the whole of it.
const EDGE_SIDES: u32 = 3;
/// World units rather than a share of the connection: `layout::edge_reach` lets a one-way
/// connection stretch as far as the layout wants, so a share of one is no fixed size at all.
/// Against `NODE_LEAF_RADIUS` for scale.
const EDGE_DASH_LENGTH: f32 = 1.0;
/// Start of one dash to the start of the next; the gap is what is left over.
const EDGE_DASH_PERIOD: f32 = 3.0;
/// Wider than a solid line because it is so much shorter: a dash as thin as the line it stands in
/// for reads as a worn-away line rather than a mark travelling along one.
const EDGE_DASH_WIDTH: f32 = 1.6;
/// Slots a second: at `1.0` a dash takes a second to reach where the dash ahead of it started. The
/// marching carries the direction on its own, so nothing about a still frame points anywhere.
/// World units a second.
const EDGE_DASH_SPEED: f32 = 3.0;
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
/// The steps home from the world that was picked, told apart from the steps on into what hangs
/// off it. Red rather than a second pale one: the two have to be read apart across a whole graph.
const ROUTE_HOME_COLOR: Srgba = Srgba::new(255, 58, 74, 255);
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
/// Cuts the dashes out of a solid line as it is drawn, so one instance is a whole connection
/// whatever its length and however many dashes fit along it.
pub(super) struct DashMaterial {
    /// World units.
    phase: f32,
    render_states: RenderStates,
}

impl Material for DashMaterial {
    /// Borrowed: `EffectMaterialId` has no slot for a material from outside three-d, and it keys
    /// the shader program cache. A second such material has to take a different one.
    fn id(&self) -> EffectMaterialId {
        EffectMaterialId::WireframeMaterial
    }

    fn fragment_shader_source(&self, _lights: &[&dyn Light]) -> String {
        format!(
            "{}{}",
            ColorMapping::fragment_shader_source(),
            include_str!("dash.frag")
        )
    }

    fn use_uniforms(&self, program: &Program, viewer: &dyn Viewer, _lights: &[&dyn Light]) {
        viewer.color_mapping().use_uniforms(program);
        program.use_uniform("dashLength", EDGE_DASH_LENGTH);
        program.use_uniform("dashPeriod", EDGE_DASH_PERIOD);
        program.use_uniform("dashPhase", self.phase);
    }

    fn render_states(&self) -> RenderStates {
        self.render_states
    }

    fn material_type(&self) -> MaterialType {
        MaterialType::Opaque
    }
}

fn dash_line() -> CpuMesh {
    let mut mesh = CpuMesh::cylinder(EDGE_SIDES);
    let Positions::F32(positions) = &mesh.positions else {
        unreachable!("`cylinder` builds its positions as f32");
    };
    // `cylinder` carries no uvs of its own. `u` is the fraction along the line, which each
    // instance scales into world units for `dash.frag` to measure the pattern against.
    mesh.uvs = Some(positions.iter().map(|at| vec2(at.x, 0.0)).collect());
    mesh
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
    // One dash instance per one-way connection.
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
        transformations: vec![Mat4::identity(); dashed],
        colors: Some(vec![Srgba::WHITE; dashed]),
        // Not a texture: each line's own length, which `dash.frag` measures the pattern against.
        texture_transformations: Some(vec![Mat3::identity(); dashed]),
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
    let dash_material = || DashMaterial {
        phase: 0.0,
        render_states: RenderStates::default(),
    };
    let dashes = Gm::new(
        InstancedMesh::new(ctx, &dash_instances, &dash_line()),
        dash_material(),
    );
    // The scene's own materials: the overlay is put in front by its depth being cleared first,
    // not by refusing the depth test, so two lit worlds that overlap settle it as any two do.
    let (lit_thumbnail_instances, lit_edge_instances) =
        (Instances::default(), Instances::default());
    let lit_thumbnails = Gm::new(
        InstancedMesh::new(ctx, &lit_thumbnail_instances, &CpuMesh::square()),
        ColorMaterial::default(),
    );
    let lit_edges = Gm::new(
        InstancedMesh::new(ctx, &lit_edge_instances, &CpuMesh::cylinder(EDGE_SIDES)),
        ColorMaterial::default(),
    );
    let lit_dash_instances = Instances {
        texture_transformations: Some(Vec::new()),
        ..Default::default()
    };
    let lit_dashes = Gm::new(
        InstancedMesh::new(ctx, &lit_dash_instances, &dash_line()),
        dash_material(),
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
        opening: opening_room(worlds),
        deepest,
        right_press: None,
        menu: None,
        cursor: None,
        hover: None,
        pointed: None,
        leaning_at: None,
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
        lit_thumbnails,
        lit_edges,
        lit_dashes,
        lit_thumbnail_instances,
        lit_edge_instances,
        lit_dash_instances,
        lit_nodes: Vec::new(),
        lit_lines: Vec::new(),
        lit_dashed: Vec::new(),

        recolored: true,
        glowing: None,
        atlas: Atlas::Loading(thumbnails::load()),
        sheet: None,
        cells: worlds.iter().map(world::World::cell).collect(),
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
        // Empty for every selection but a route, so the match arm below never fires for one.
        let mut homeward = vec![false; self.titles.len()];
        if matches!(self.selected, Some(Highlight::Route(_))) {
            for node in self.route() {
                homeward[node] = true;
            }
        }

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
        self.lit_nodes.clear();
        if lit {
            self.lit_nodes.extend(
                on_route
                    .iter()
                    .enumerate()
                    .filter_map(|(world, &on_route)| on_route.then_some(world)),
            );
        }

        let (routes, on_route, homeward) = (&self.routes.parents, &on_route, &homeward);
        let (solid, dashed, base) = (
            self.edge_instances.colors.as_mut().unwrap(),
            self.dash_instances.colors.as_mut().unwrap(),
            &self.edge_colors,
        );
        let (lit_lines, lit_dashed) = (&mut self.lit_lines, &mut self.lit_dashed);
        lit_lines.clear();
        lit_dashed.clear();
        // The same order, and the same split between the two, that [`Self::rebuild_instances`] and
        // [`Self::march_dashes`] write the transformations in.
        let (mut edge, mut line, mut dash) = (0, 0, 0);
        self.graph.visit_edges(|a, b, data| {
            let (a, b) = (a.index().index(), b.index().index());
            // Whether this edge is a canonical step: from one of its ends to that end's parent.
            let step = routes[a] == Some(b) || routes[b] == Some(a);
            let (color, shown) = match self.selected {
                None => (base[edge], false),
                // The one line it is about, and not even the step home from either of its ends,
                // which is a different way through the graph than the one the reader asked for.
                Some(Highlight::Connection(at, far)) => {
                    if (a, b) == (at, far) || (a, b) == (far, at) {
                        (ROUTE_COLOR, true)
                    } else {
                        (dim(base[edge]), false)
                    }
                }
                // A layer is a shell rather than a walk, so what is worth seeing across it is
                // where it is stitched to itself, not the step each world takes home. Both ends
                // being lit is the whole test, and such an edge is never a canonical step -- a
                // parent is always exactly one depth in -- so it keeps its own distance color and
                // only escapes the dimming.
                Some(Highlight::Layer(_)) if on_route[a] && on_route[b] => (base[edge], true),
                // Both ends being lit is not enough: it also has to be the step from one of them
                // to that end's parent, or a shortcut between two distant points of a route would
                // light up as if the walk went through it.
                Some(_) if step && homeward[a] && homeward[b] => (ROUTE_HOME_COLOR, true),
                Some(_) if step && on_route[a] && on_route[b] => (ROUTE_COLOR, true),
                Some(_) => (dim(base[edge]), false),
            };
            // Every dash of a one-way connection takes the same color, so it reads as the one
            // line it stands for.
            if data.user_data {
                dashed[dash] = color;
                if shown {
                    lit_dashed.push(dash);
                }
                dash += 1;
            } else {
                solid[line] = color;
                if shown {
                    lit_lines.push(line);
                }
                line += 1;
            }
            edge += 1;
        });

        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.edges.set_instances(&self.edge_instances);
        self.dashes.set_instances(&self.dash_instances);
        self.gather_lit();
    }

    /// Called wherever the scene's own buffers are written, this being a copy of their lit slots
    /// and nothing more.
    fn gather_lit(&mut self) {
        let gather = |instances: &mut Instances, from: &[&Instances], at: &[&[usize]]| {
            instances.transformations.clear();
            let colors = instances.colors.get_or_insert_default();
            colors.clear();
            for (from, at) in from.iter().zip(at) {
                instances
                    .transformations
                    .extend(at.iter().map(|&at| from.transformations[at]));
                let source = from
                    .colors
                    .as_ref()
                    .expect("the scene's own are all colored");
                colors.extend(at.iter().map(|&at| source[at]));
            }
        };
        gather(
            &mut self.lit_edge_instances,
            &[&self.edge_instances],
            &[&self.lit_lines],
        );
        gather(
            &mut self.lit_dash_instances,
            &[&self.dash_instances],
            &[&self.lit_dashed],
        );
        gather(
            &mut self.lit_thumbnail_instances,
            &[&self.thumbnail_instances],
            &[&self.lit_nodes],
        );
        let picked = |from: Option<&Vec<Mat3>>, at: &[usize]| {
            from.map(|from| at.iter().map(|&at| from[at]).collect())
        };
        self.lit_thumbnail_instances.texture_transformations = picked(
            self.thumbnail_instances.texture_transformations.as_ref(),
            &self.lit_nodes,
        );
        self.lit_dash_instances.texture_transformations = picked(
            self.dash_instances.texture_transformations.as_ref(),
            &self.lit_dashed,
        );
        self.lit_edges.set_instances(&self.lit_edge_instances);
        self.lit_dashes.set_instances(&self.lit_dash_instances);
        self.lit_thumbnails
            .set_instances(&self.lit_thumbnail_instances);
    }

    /// The overlay, pictures before lines for the reason the scene's own passes are in that order:
    /// the depth a picture writes drops the lines behind it unshaded.
    ///
    /// Empty while nothing is selected, and the pictures wait for the atlas as
    /// [`Self::drawn_thumbnails`] does.
    pub(super) fn drawn_lit(&self) -> impl Iterator<Item = &dyn Object> {
        let pictures = self.lit_thumbnails.material.texture.is_some();
        [
            (pictures && !self.lit_nodes.is_empty()).then_some(&self.lit_thumbnails as &dyn Object),
            (!self.lit_lines.is_empty()).then_some(&self.lit_edges as &dyn Object),
            (!self.lit_dashed.is_empty()).then_some(&self.lit_dashes as &dyn Object),
        ]
        .into_iter()
        .flatten()
    }

    /// Whether the overlay has anything at all to draw, and so whether the depth it wants out of
    /// the way is worth clearing.
    pub(super) fn lit_anything(&self) -> bool {
        self.drawn_lit().next().is_some()
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
        // The dash lines are laid here with the solid ones rather than once a frame: `DashMaterial`
        // cuts the dashes out as it draws, so nothing about them moves unless the layout does.
        let dash_radius = radius * EDGE_DASH_WIDTH;
        let (edges, dashes, spans) = (
            &mut self.edge_instances.transformations,
            &mut self.dash_instances.transformations,
            self.dash_instances
                .texture_transformations
                .as_mut()
                .expect("the dash lines are built carrying their own lengths"),
        );
        edges.clear();
        dashes.clear();
        spans.clear();
        self.graph.visit_edges(|a, b, data| {
            let (from, to) = (world_pos(a.position()), world_pos(b.position()));
            let dir = to - from;
            let span = dir.magnitude();
            // As thin as the later of the two worlds it joins is small, so a line is not drawn to
            // somewhere that is not there yet.
            let grown = arrivals
                .grown(a.index().index())
                .min(arrivals.grown(b.index().index()));
            let one_way = data.user_data;
            let radius = grown * if one_way { dash_radius } else { radius };
            // World to world: the pictures are opaque and write their depth first, so what a line
            // lays inside one is hidden there rather than having to be held out of it.
            let placed = match span > 0.0 {
                true => {
                    Mat4::from_translation(from)
                        * rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), dir / span)
                        * Mat4::from_nonuniform_scale(span, radius, radius)
                }
                // Two worlds in the same place point a line nowhere; collapsed, not left a NaN.
                false => Mat4::from_scale(0.0),
            };
            match one_way {
                false => edges.push(placed),
                true => {
                    dashes.push(placed);
                    spans.push(Mat3::from_nonuniform_scale(span, 1.0));
                }
            }
        });
        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.edges.set_instances(&self.edge_instances);
        self.dashes.set_instances(&self.dash_instances);
        self.gather_lit();
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
    pub(super) fn march_dashes(&mut self, dt: f32) {
        // Past one period every dash stands where the dash ahead of it stood, so the phase can
        // start over rather than growing until it loses its precision.
        self.dash_phase = (self.dash_phase + dt * EDGE_DASH_SPEED).rem_euclid(EDGE_DASH_PERIOD);
        self.dashes.material.phase = self.dash_phase;
        self.lit_dashes.material.phase = self.dash_phase;
    }

    /// Points the thumbnail quads at their own cells of the atlas once it has arrived, and hands
    /// egui a copy for the catalog. Nothing is drawn until then.
    ///
    /// An atlas that could not be had is asked for again after [`ATLAS_RETRIED_AFTER_SECONDS`]:
    /// `seconds` is the frame's own, and the wait therefore runs on frames the app was drawing
    /// anyway rather than keeping it awake. So a run left alone defers its next try until something
    /// touches it, which is the right trade for pictures the graph reads fine without.
    pub(super) fn receive_atlas(&mut self, seconds: f32, context: &Context, egui: &egui::Context) {
        let loaded = match &mut self.atlas {
            Atlas::Settled => return,
            Atlas::Waiting(until) => {
                *until -= seconds;
                if *until <= 0.0 {
                    self.atlas = Atlas::Loading(thumbnails::load());
                }
                return;
            }
            Atlas::Loading(pending) => match pending.take() {
                Some(loaded) => loaded,
                None => return,
            },
        };
        // Logged where it is found. The graph goes on being drawn as it was before thumbnails
        // existed, so the only cost of another try is the try.
        let Some(atlas) = loaded else {
            self.atlas = Atlas::Waiting(ATLAS_RETRIED_AFTER_SECONDS);
            return;
        };
        self.atlas = Atlas::Settled;
        self.sheet = thumbnails::Sheet::new(egui, &atlas);
        let Some(cells) = thumbnails::cells(&self.cells, &atlas) else {
            return;
        };
        self.thumbnail_instances.texture_transformations = Some(cells);
        self.thumbnails.set_instances(&self.thumbnail_instances);
        // One upload, sampled by both: a [`Texture2DRef`] is a handle over a shared texture, so
        // the clone costs a Mat3 and an atomic rather than a second copy of the atlas.
        let atlas = Texture2DRef::from_cpu_texture(context, &atlas);
        self.glow.material.texture = Some(atlas.clone());
        self.lit_thumbnails.material.texture = Some(atlas.clone());
        // Last, being what [`Self::drawn_thumbnails`] reads to decide the quads are drawable.
        self.thumbnails.material.texture = Some(atlas);
        self.gather_lit();
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
/// A normalized distance from the origin around the hues, so how far a world sits from the start
/// of the game reads off its color. Blue at the origin and on round through cyan, green, yellow
/// and red to magenta: a turn of five sixths rather than a whole one, so the deepest connection
/// is not painted the same blue as the shallowest.
///
/// Written as the corners themselves rather than as a hue swept through: neighbouring corners
/// share a channel at the full, so interpolating between two of them stays as saturated as both,
/// and no stop is a number this file would have to trust a conversion for. The stops stay bright
/// to hold up against [`BACKGROUND_COLOR`], the blue lifted off a pure one -- which at this end
/// of the range would read as unlit rather than as near.
fn distance_color(distance: f32) -> Srgba {
    const STOPS: [[f32; 3]; 6] = [
        [0.25, 0.35, 1.00],
        [0.10, 0.90, 1.00],
        [0.35, 1.00, 0.45],
        [1.00, 0.95, 0.30],
        [1.00, 0.35, 0.25],
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
