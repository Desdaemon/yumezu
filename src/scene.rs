//! The graph as something drawn: the worlds, their connections, the dashes that march along the
//! one-way ones, and the backdrop behind all of it. See [`entities`] and [`AppEntities`].

use super::*;

const EDGE_RADIUS: f32 = 0.05;
/// Sides of the tube a connection is drawn as. Tens of thousands of them come out a pixel wide, so
/// the cost is triangle setup rather than fill and the sides are very nearly the whole of it.
const EDGE_SIDES: u32 = 3;
/// World units rather than a share of the connection, which `layout::edge_reach` lets stretch as
/// far as the layout wants. Against `NODE_LEAF_RADIUS` for scale.
const EDGE_DASH_LENGTH: f32 = 1.0;
/// Start of one dash to the start of the next; the gap is what is left over.
const EDGE_DASH_PERIOD: f32 = 3.0;
/// Wider than a solid line because it is so much shorter: a dash as thin as the line it stands in
/// for reads as a worn-away line rather than a mark travelling along one.
const EDGE_DASH_WIDTH: f32 = 1.6;
/// Slots a second: at `1.0` a dash takes a second to reach where the dash ahead of it started. The
/// marching carries the direction, so nothing about a still frame points anywhere.
const EDGE_DASH_SPEED: f32 = 3.0;
/// sRGB, which is what both the clear and [`panorama_texture`]'s glows over it want.
pub(super) const BACKGROUND_COLOR: [f32; 3] = [0.03, 0.03, 0.05];
/// Side of the backdrop's repeating tile in texels, and the spacing of the lattice of glows inside
/// it. Four glows lie across the tile, each with its own brightness, so it does not read as one
/// cell stamped over and over. Its size on screen is [`PANORAMA_TILE_PIXELS`].
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
/// Small on purpose: a panorama sits far enough away that a turn barely shifts it, and that faint
/// drift is what makes the graph read as turning in front of a scene rather than a decal.
const PANORAMA_PARALLAX: f32 = 0.12;
/// Brightness of the connection into a world with nothing behind it, against the one into the
/// world the whole game is behind: the range the depth ramp is drawn over. See [`edge_colors`].
const EDGE_LEAF_BRIGHTNESS: f32 = 0.6;
/// How far the connection colors move toward even apparent brightness: at 0 the ramp keeps its own
/// stops, at 1 every stop takes the brightness of the dimmest.
///
/// The ramp is picked for hue, and hue drags brightness with it -- most of what the eye reads as
/// brightness is the green channel, so the green stop looks glaring beside the magenta. Levelled
/// far enough that no stop shouts, not so far that the ramp flattens.
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
/// Added rather than multiplied, a lit node already being painted white. High enough to pick the
/// node out of a crowd, low enough that the screenshot is still a screenshot.
const POINTED_BRIGHTNESS: f32 = 0.85;
/// Deliberately off the distance ramp: the worlds the origin cannot reach are at no distance at
/// all rather than at a large one, and reading them as the far end of the ramp would be a lie.
const UNREACHED_COLOR: Srgba = Srgba::new(110, 110, 120, 255);
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
    // `cylinder` has no uvs; `u` is the fraction along the line, scaled for `dash.frag`.
    mesh.uvs = Some(positions.iter().map(|at| vec2(at.x, 0.0)).collect());
    mesh
}
/// How long the app waits before fetching the thumbnail atlas again, after a try that could not be
/// had. Longer than the dump's retries because nothing waits on it: the graph draws without the
/// pictures. See [`Atlas`].
const ATLAS_RETRIED_AFTER_SECONDS: f32 = 10.0;

/// Where the thumbnail atlas has got to: a state rather than a handle because a try that fails is
/// retried. Modelled on [`Dump`], which waits the same way.
pub(super) enum Atlas {
    Loading(fetch::Pending<Option<CpuTexture>>),
    /// Seconds until the next try.
    Waiting(f32),
    /// On screen, or wrong-shaped and therefore never going to be: a mismatch between the atlas and
    /// the dump is not something trying again can mend.
    Settled,
}

/// A free function rather than a method: it reads the dump the app holds and hands back what the
/// app is about to hold beside it, two borrows of one [`App`] a method could not take.
pub(super) fn entities(
    dump: &world::Dump,
    before: Option<&Before>,
    ctx: &WindowedContext,
) -> AppEntities {
    let rng = Rng(0x5eed_1337);

    let worlds = &dump.worlds;
    let connections = world::connections(worlds);
    // One depth for the colors and the layered pins, so the two agree.
    let routes = world::routes_from(&connections, world::origin_world(worlds));
    let deepest = routes.depth.iter().flatten().copied().max().unwrap_or(0);
    let furthest = deepest.max(1) as f32;
    // Logarithmic, so a deep world with a handful behind it nears a shallow one with a hundred.
    let descendants = routes.descendant_counts();
    let node_radii: Vec<_> = descendants
        .iter()
        .map(|&descendants| {
            let growth = (1.0 + descendants as f32).ln() / (1.0 + NODE_HUB_DESCENDANTS).ln();
            NODE_LEAF_RADIUS + (NODE_HUB_RADIUS - NODE_LEAF_RADIUS) * growth
        })
        .collect();

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
    // Positions come from `scatter`, so the initial layout and a restart agree.
    let nodes: Vec<_> = routes
        .depth
        .iter()
        .enumerate()
        .map(|(world, depth)| {
            graph.add_node(NodeData {
                // Unreachable worlds get a layer past the deepest.
                level: depth.map_or(furthest + 1.0, |depth| depth as f32),
                mass: node_mass(node_radii[world], hub_repulsion),
                ..Default::default()
            })
        })
        .collect();
    // Counted off the edges' connections, so the panel's junctions match the drawn lines.
    let mut degrees = vec![0; worlds.len()];
    let mut dashed = 0;
    for (from, steps) in connections.iter().enumerate() {
        for step in steps {
            // Once per connection: two springs on one pair would pull twice as hard.
            if step.world <= from {
                continue;
            }
            let to = step.world;
            // From the leavable end, so the dashes march the walkable way.
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

    // The connections wear the depth; the worlds carry pictures.
    let depth_colors: Vec<_> = routes
        .depth
        .iter()
        .map(|depth| match depth {
            Some(depth) => distance_color(*depth as f32 / furthest),
            None => UNREACHED_COLOR,
        })
        .collect();
    let edge_colors = edge_colors(&graph, &routes, &descendants, &depth_colors);
    // Read off the same source as the atlas cells.
    let unvisited: Vec<usize> = worlds
        .iter()
        .enumerate()
        .filter_map(|(world, it)| it.cell().is_none().then_some(world))
        .collect();
    let titles: Vec<world::Title> = worlds.iter().map(world::World::titles).collect();
    let maps: Vec<Vec<world::Map>> = worlds.iter().map(world::World::maps).collect();
    // Both group every world, too costly per frame.
    let (authors, author_of) = dump.authors();
    let versions = dump.versions();
    // White, so a picture shows as itself.
    let thumbnail_instances = Instances {
        transformations: vec![Mat4::identity(); worlds.len()],
        colors: Some(vec![Srgba::WHITE; worlds.len()]),
        ..Default::default()
    };
    // Colors are left to the first `repaint`, which owns the same fan-out.
    let solid = edge_colors.len() - dashed;
    let edge_instances = Instances {
        transformations: vec![Mat4::identity(); solid],
        colors: Some(vec![Srgba::WHITE; solid]),
        ..Default::default()
    };
    let dash_instances = Instances {
        transformations: vec![Mat4::identity(); dashed],
        colors: Some(vec![Srgba::WHITE; dashed]),
        // Not a texture: each line's length, for `dash.frag`.
        texture_transformations: Some(vec![Mat3::identity(); dashed]),
    };
    let backdrop = ColorMaterial {
        color: Srgba::WHITE,
        texture: Some(Texture2DRef::from_cpu_texture(ctx, &panorama_tile())),
        // Colour only, no depth test: the quad stands in for the cleared background.
        render_states: RenderStates {
            write_mask: WriteMask::COLOR,
            depth_test: DepthTest::Always,
            ..Default::default()
        },
        is_transparent: false,
    };
    // Billboarded quads: a cube's uvs spread the image across six faces.
    let thumbnails = Gm::new(
        InstancedMesh::new(ctx, &thumbnail_instances, &CpuMesh::square()),
        ColorMaterial::default(),
    );
    // Additive, since a lit node is already white.
    let glow = Gm::new(
        Mesh::new(ctx, &CpuMesh::square()),
        ColorMaterial {
            color: scaled(Srgba::WHITE, POINTED_BRIGHTNESS),
            texture: None,
            render_states: RenderStates {
                write_mask: WriteMask::COLOR,
                // Equal passes: the lift toward the camera is under one depth step here.
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
    // The overlay comes first by a depth clear, not by skipping the depth test.
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
        framing: arriving,
        frame_route: false,
        ways: Vec::new(),
        way: 0,
        hub: world::hub_world(worlds),
        edge_colors,
        node_radii,
        hub_repulsion,
        link_reach,
        descendant_counts: descendants,
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
        unvisited_quads: Instances {
            transformations: vec![Mat4::identity(); unvisited.len()],
            colors: Some(vec![Srgba::WHITE; unvisited.len()]),
            ..Default::default()
        },
        lit_unvisited_quads: Instances {
            colors: Some(Vec::new()),
            ..Default::default()
        },
        detail: detail::Detail::new(
            worlds.iter().map(|world| world.image.clone()).collect(),
            // Or one shrinking out from under a placeholder.
            !unvisited.is_empty() || arrivals.turning(),
        ),
        dash_phase: 0.0,
        billboard: Mat4::identity(),
        arrivals,
    };
    data.repaint();
    scatter(&mut data);
    // A replacement graph carries on from where the previous one stood.
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
        // Per world on the walk, the world it is stepped to from.
        let mut walked = vec![None; self.titles.len()];
        if matches!(
            self.selected,
            Some(Highlight::Route(_) | Highlight::Path(..))
        ) {
            let walk = self.route();
            for (at, &world) in walk.iter().enumerate() {
                walked[world] = walk.get(at + 1).copied();
            }
        }

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

        let (routes, on_route, walked) = (&self.routes.parents, &on_route, &walked);
        let (solid, dashed, base) = (
            self.edge_instances.colors.as_mut().unwrap(),
            self.dash_instances.colors.as_mut().unwrap(),
            &self.edge_colors,
        );
        let (lit_lines, lit_dashed) = (&mut self.lit_lines, &mut self.lit_dashed);
        lit_lines.clear();
        lit_dashed.clear();
        // Same order and split as [`Self::rebuild_instances`] and [`Self::march_dashes`].
        let (mut edge, mut line, mut dash) = (0, 0, 0);
        self.graph.visit_edges(|a, b, data| {
            let (a, b) = (a.index().index(), b.index().index());
            let step = routes[a] == Some(b) || routes[b] == Some(a);
            let (color, shown) = match self.selected {
                None => (base[edge], false),
                // Only the line itself, not the step home from either end.
                Some(Highlight::Connection(at, far)) => {
                    if (a, b) == (at, far) || (a, b) == (far, at) {
                        (ROUTE_COLOR, true)
                    } else {
                        (dim(base[edge]), false)
                    }
                }
                // Never a canonical step, so it keeps its distance color.
                Some(Highlight::Layer(_)) if on_route[a] && on_route[b] => (base[edge], true),
                // Or a shortcut across the route would light up.
                Some(_) if walked[a] == Some(b) || walked[b] == Some(a) => (ROUTE_HOME_COLOR, true),
                // Directions do not follow the canonical tree.
                Some(Highlight::Path(..)) => (dim(base[edge]), false),
                Some(_) if step && on_route[a] && on_route[b] => (ROUTE_COLOR, true),
                Some(_) => (dim(base[edge]), false),
            };
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
        // Each upload builds four GPU buffers, so an empty overlay stays unwritten.
        let lighting =
            !self.lit_nodes.is_empty() || !self.lit_lines.is_empty() || !self.lit_dashed.is_empty();
        let standing = self.lit_thumbnails.instance_count() > 0
            || self.lit_edges.instance_count() > 0
            || self.lit_dashes.instance_count() > 0;
        if !lighting && !standing {
            return;
        }
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
        .chain(self.detail.drawn_lit(&self.lit_nodes))
    }

    /// Whether the overlay has anything at all to draw, and so whether the depth it wants out of
    /// the way is worth clearing.
    pub(super) fn lit_anything(&self) -> bool {
        self.drawn_lit().next().is_some()
    }

    fn edge_radius(&self) -> f32 {
        EDGE_RADIUS
    }

    /// `laid_out` is whether the layout itself moved. The node quads face the camera, so a turn
    /// dates them alone: the lines are only where the worlds are.
    pub(super) fn rebuild_instances(&mut self, camera: &Camera, laid_out: bool) {
        self.rebuild_nodes(camera);
        if laid_out {
            self.rebuild_edges();
        }
        self.gather_lit();
    }

    fn rebuild_nodes(&mut self, camera: &Camera) {
        let billboard = billboard(camera);
        let thumbnails = &mut self.thumbnail_instances.transformations;
        thumbnails.clear();
        let arrivals = &self.arrivals;
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            // Everything hung off the node quads copies this size.
            let radius = self.node_radii[world] * arrivals.grown(world);
            // The pictures' aspect, so none are cropped or stretched.
            thumbnails.push(
                Mat4::from_translation(world_pos(node.position()))
                    * billboard
                    * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0),
            );
        });
        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.billboard = billboard;
    }

    /// The dash lines are laid here with the solid ones rather than once a frame: `DashMaterial`
    /// cuts the dashes out as it draws, so nothing about them moves unless the layout does.
    fn rebuild_edges(&mut self) {
        let radius = self.edge_radius();
        let arrivals = &self.arrivals;
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
            let grown = arrivals
                .grown(a.index().index())
                .min(arrivals.grown(b.index().index()));
            let one_way = data.user_data;
            let radius = grown * if one_way { dash_radius } else { radius };
            // World to world: the opaque pictures write depth first and hide the line inside them.
            let placed = match span > 0.0 {
                true => {
                    Mat4::from_translation(from)
                        * rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), dir / span)
                        * Mat4::from_nonuniform_scale(span, radius, radius)
                }
                // Coincident worlds would make a NaN.
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
        self.edges.set_instances(&self.edge_instances);
        self.dashes.set_instances(&self.dash_instances);
    }

    /// Stands the placeholder over every world the player has not been to.
    ///
    /// Taken from the nodes' own quads rather than worked out again, so a placeholder is exactly
    /// where and how big its world is, and dims with it. Lifted toward the camera as a magnified
    /// picture is, coplanar quads otherwise leaving the depth test to pick per pixel.
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
        let wearing = self.unvisited.iter().copied().chain(self.arrivals.veiled());
        let (transformations, colors) = (
            &mut self.unvisited_quads.transformations,
            self.unvisited_quads.colors.as_mut().unwrap(),
        );
        let (lit_transformations, lit_colors) = (
            &mut self.lit_unvisited_quads.transformations,
            self.lit_unvisited_quads.colors.as_mut().unwrap(),
        );
        let lit_nodes = &self.lit_nodes;
        transformations.clear();
        colors.clear();
        lit_transformations.clear();
        lit_colors.clear();
        for world in wearing {
            let Some(node) = nodes.transformations.get(world) else {
                continue;
            };
            let placed =
                Mat4::from_translation(-forward * (self.node_radii[world] * detail::LIFT)) * node;
            let color = painted.map_or(Srgba::WHITE, |painted| painted[world]);
            // `lit_nodes` is in world order.
            if lit_nodes.binary_search(&world).is_ok() {
                lit_transformations.push(placed);
                lit_colors.push(color);
            }
            transformations.push(placed);
            colors.push(color);
        }
        self.detail
            .place_unvisited(context, &self.unvisited_quads, &self.lit_unvisited_quads);
    }

    /// Every frame, unlike the rest of the geometry: the marching is the whole point of the
    /// dashes, and a settled layout is exactly when it has to carry on regardless.
    pub(super) fn march_dashes(&mut self, dt: f32) {
        // Wrapped at one period so the phase keeps its precision.
        self.dash_phase = (self.dash_phase + dt * EDGE_DASH_SPEED).rem_euclid(EDGE_DASH_PERIOD);
        self.dashes.material.phase = self.dash_phase;
        self.lit_dashes.material.phase = self.dash_phase;
    }

    /// Points the thumbnail quads at their own cells of the atlas once it has arrived, and hands
    /// egui a copy for the catalog. Nothing is drawn until then.
    ///
    /// An atlas that could not be had is fetched again after [`ATLAS_RETRIED_AFTER_SECONDS`].
    /// `seconds` is the frame's own, so the wait runs on frames the app was drawing anyway and a
    /// run left alone defers its next try until something touches it.
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
        // One upload: a [`Texture2DRef`] clone is a Mat3 and an atomic.
        let atlas = Texture2DRef::from_cpu_texture(context, &atlas);
        self.glow.material.texture = Some(atlas.clone());
        self.lit_thumbnails.material.texture = Some(atlas.clone());
        // Last: [`Self::drawn_thumbnails`] reads it to decide the quads are drawable.
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
    /// The node's own quad is taken rather than worked out again, so the glow brightens exactly
    /// the shape on screen. Only the lift is added, twice [`detail::LIFT`] because a magnified
    /// world already sits one lift in front.
    ///
    /// That lift is too small to clear the depth buffer's rounding; the quad's `depth_test` is
    /// what makes that harmless.
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

    /// Every world the view needs more of than the atlas holds, nearest the camera first.
    ///
    /// What [`detail`] is driven from, and the reason it needs no view of its own: a world is here
    /// because its node is on screen and drawn wider than [`detail::LEAVE_PIXELS`], carrying the
    /// quad it would have been drawn on. The whole hysteresis band, which end applies being known
    /// only to [`detail::Detail::track`].
    ///
    /// A world in the middle of becoming itself is measured at the size it is about to be, so what
    /// grows back is the world's own picture from the first frame.
    pub(super) fn magnified(&self, camera: &Camera, viewport: Viewport) -> Vec<detail::Magnified> {
        let billboard = billboard(camera);
        let forward = camera.view_direction();
        let colors = self.thumbnail_instances.colors.as_ref();
        let across = camera.right_direction().normalize();
        let mut magnified: Vec<(f32, detail::Magnified)> = Vec::new();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            let position = world_pos(node.position());
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
            // Ranked at the settling size, drawn at the current one.
            let width = drawn_width(camera, across, full, position, center);
            if width < detail::LEAVE_PIXELS {
                return;
            }
            let radius = full * grown;
            magnified.push((
                (position - camera.position()).magnitude2(),
                detail::Magnified {
                    world,
                    width,
                    transformation: Mat4::from_translation(
                        position - forward * (radius * detail::LIFT),
                    ) * billboard
                        * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0),
                    color: colors.map_or(Srgba::WHITE, |colors| colors[world]),
                },
            ));
        });
        // Not drawn width: perspective inflates a node a third in the corner.
        magnified.sort_by(|(a, _), (b, _)| a.total_cmp(b));
        magnified.into_iter().map(|(_, it)| it).collect()
    }
}
/// A lattice of soft blue glows on the background color, in the style of the game's own panoramas.
///
/// Held in linear color, which the shader's sRGB encoding expects, and at half precision: eight
/// bits of linear bands at these brightnesses, and dithering trades the bands for grain.
fn panorama_tile() -> CpuTexture {
    let cells = PANORAMA_TILE_TEXELS / PANORAMA_CELL_TEXELS;
    let mut data = Vec::with_capacity(PANORAMA_TILE_TEXELS * PANORAMA_TILE_TEXELS);
    for y in 0..PANORAMA_TILE_TEXELS {
        for x in 0..PANORAMA_TILE_TEXELS {
            // Neighbours included, so glows straddling an edge match when the tile repeats.
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
/// enter, or the backdrop would read as a plane the graph slides over.
///
/// The offset is taken straight from the view direction rather than from a yaw and a pitch, so it
/// stays continuous all the way around instead of snapping where an angle wraps.
pub(super) fn panorama_transform(viewport: Viewport, device_pixel_ratio: f32, view: Vec3) -> Mat3 {
    let tile = PANORAMA_TILE_PIXELS * device_pixel_ratio;
    let drift = vec2(view.x, view.y) * PANORAMA_PARALLAX;
    Mat3::from_translation(drift)
        * Mat3::from_nonuniform_scale(viewport.width as f32 / tile, viewport.height as f32 / tile)
}
/// A normalized distance from the origin around the hues. Blue at the origin and on through cyan,
/// green, yellow and red to magenta: five sixths of a turn, so the deepest connection is not
/// painted the shallowest's blue.
///
/// Written as the corners themselves rather than as a hue swept through: neighbouring corners
/// share a channel at the full, so interpolating between two stays as saturated as both. The blue
/// is lifted off a pure one, which against [`BACKGROUND_COLOR`] would read as unlit.
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
/// connection wears the color of the world it is walked *from*. Its brightness says how much of
/// the game lies through it, through a logarithm as the node sizes are.
///
/// Which end is walked from is the canonical routes' answer where they have one. A shortcut across
/// the tree is taken as walked from its shallower end, which a player meets it from anyway.
fn edge_colors(
    graph: &Graph,
    routes: &world::Routes,
    descendants: &[u32],
    depth_colors: &[Srgba],
) -> Vec<Srgba> {
    // Against the busiest world, which the whole game hangs off.
    let busiest = (1.0 + descendants.iter().copied().max().unwrap_or(0) as f32).ln();
    // The ramp's floor, so the correction only darkens; [`distance_color`] defines the ramp.
    const SAMPLES: u32 = 64;
    let floor = (0..=SAMPLES)
        .map(|step| luminance(distance_color(step as f32 / SAMPLES as f32)))
        .fold(f32::INFINITY, f32::min);
    let mut colors = Vec::with_capacity(graph.edge_count());
    graph.visit_edges(|a, b, _| {
        let (a, b) = (a.index().index(), b.index().index());
        // Unreachable worlds sort deepest, unlike `Option`'s order.
        let depth = |world: usize| routes.depth[world].unwrap_or(u32::MAX);
        let from = match (routes.parents[b], routes.parents[a]) {
            (Some(parent), _) if parent == a => a,
            (_, Some(parent)) if parent == b => b,
            _ if depth(a) <= depth(b) => a,
            _ => b,
        };
        let to = if from == a { b } else { a };
        let reach = (1.0 + descendants[to] as f32).ln() / busiest.max(f32::MIN_POSITIVE);
        // Never above 1: [`UNREACHED_COLOR`] is off the ramp and stays as it is.
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
