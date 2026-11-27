//! Scene construction and per-frame GPU buffers: world thumbnails, edges, animated dashes on
//! one-way edges, and the background. See [`entities`] and [`AppEntities`].

use super::*;

const EDGE_RADIUS: f32 = 0.05;
/// In world units, so dashes keep their length as connections stretch; compare `NODE_LEAF_RADIUS`.
const EDGE_DASH_LENGTH: f32 = 1.0;
/// Distance from one dash start to the next; the gap is the remainder.
const EDGE_DASH_PERIOD: f32 = 3.0;
/// Dash width relative to a solid edge; dashes as thin as the edge read as a broken line.
const EDGE_DASH_WIDTH: f32 = 1.6;
/// Dash speed in world units per second. The animation is the only cue for edge direction.
const EDGE_DASH_SPEED: f32 = 3.0;
/// sRGB, as both the clear color and [`panorama_tile`] expect.
pub(super) const BACKGROUND_COLOR: [f32; 3] = [0.03, 0.03, 0.05];
/// Background tile size and glow spacing in texels: a 4×4 grid of glows with varied brightness,
/// so the repetition is not obvious. Screen size is [`PANORAMA_TILE_PIXELS`].
const PANORAMA_TILE_TEXELS: usize = 256;
const PANORAMA_CELL_TEXELS: usize = 64;
/// Glow radius in texels and falloff exponent; the radius almost reaches the neighbouring glows.
const PANORAMA_GLOW_RADIUS_TEXELS: f32 = 52.0;
const PANORAMA_GLOW_FALLOFF: f32 = 2.2;
/// Lp norm for glow distance: 1.5 gives the rounded-square shape of the game's panoramas.
const PANORAMA_GLOW_NORM: f32 = 1.5;
/// Dim blue: visible without competing with the edges.
const PANORAMA_GLOW_COLOR: [f32; 3] = [3.0 / 255.0, 6.0 / 255.0, 42.0 / 255.0];
/// Peak brightness per cell as a fraction of [`PANORAMA_GLOW_COLOR`]; irregular but tileable.
const PANORAMA_CELL_PEAKS: [[f32; 4]; 4] = [
    [0.55, 0.30, 0.85, 0.40],
    [0.35, 1.00, 0.45, 0.65],
    [0.90, 0.50, 0.70, 0.30],
    [0.40, 0.75, 0.35, 0.95],
];
/// Tile size on screen in logical pixels, so the background keeps a constant scale on HiDPI.
const PANORAMA_TILE_PIXELS: f32 = 260.0;
/// Background parallax in tiles per unit of view direction x/y; small, to read as distant scenery.
const PANORAMA_PARALLAX: f32 = 0.12;
/// Brightness of an edge into a leaf world; an edge into the world with the most descendants is
/// 1.0. See [`edge_colors`].
const EDGE_LEAF_BRIGHTNESS: f32 = 0.6;
/// How far edge colors are equalized for luma: 0 keeps the ramp, 1 darkens every stop to the
/// dimmest. At equal ramp weight the green stop reads much brighter than magenta.
const EDGE_LUMINANCE_EVENNESS: f32 = 0.5;
/// Edges on the route back to the origin world.
const ROUTE_COLOR: Srgba = Srgba::new(255, 242, 194, 255);
/// Edges on the route home from the selected world; red to contrast with [`ROUTE_COLOR`].
const ROUTE_HOME_COLOR: Srgba = Srgba::new(255, 58, 74, 255);
/// Brightness of everything outside the highlight.
const DIMMED_BRIGHTNESS: f32 = 0.25;
/// How far out the horizon marker is drawn, in world units: past the deepest world, inside the far
/// plane.
const HORIZON_MARKER_DISTANCE: f32 = 4000.0;
/// Half the height of the horizon marker's tick, as a share of the view's half-height.
const HORIZON_MARKER_SHARE: f32 = 0.012;
/// Half the length of the horizon marker's stripe, in tick half-heights.
const HORIZON_STRIPE_LENGTH: f32 = 3.0;
/// Half the thickness of the horizon marker's tick and stripe, in tick half-heights.
const HORIZON_LINE_WIDTH: f32 = 0.12;
/// Strength of the additive glow on the world of a hovered list row. Additive because a
/// highlighted node is already full white.
const HOVERED_ROW_BRIGHTNESS: f32 = 0.85;
/// Gray, off the distance ramp: unreachable worlds have no distance.
const UNREACHED_COLOR: Srgba = Srgba::new(110, 110, 120, 255);
/// Material whose fragment shader, `dash.frag`, cuts dashes out of a ribbon, so one instance
/// covers a whole edge.
pub(super) struct DashMaterial {
    /// World units.
    phase: f32,
    render_states: RenderStates,
}

impl Material for DashMaterial {
    /// Borrowed ID: `EffectMaterialId` has no variant for custom materials and keys the shader
    /// cache, so another custom material must borrow a different one.
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

/// Atlas retry delay; longer than the dump's because the graph renders without thumbnails. See
/// [`Atlas`].
const ATLAS_RETRIED_AFTER_SECONDS: f32 = 10.0;

/// Thumbnail atlas loading state machine, with retry; mirrors [`Dump`].
pub(super) enum Atlas {
    Loading(fetch::Pending<Option<CpuTexture>>),
    /// Seconds until the next try.
    Waiting(f32),
    /// Loaded, or permanently mismatched with the dump (a retry cannot fix that).
    Settled,
}

/// Builds the scene from the dump. A free function because a method on [`App`] would need to
/// borrow `App::dump` while assigning `App::data`.
pub(super) fn entities(
    dump: &world::Dump,
    before: Option<&Before>,
    ctx: &WindowedContext,
) -> AppEntities {
    let rng = Rng(0x5eed_1337);

    let worlds = &dump.worlds;
    let connections = world::connections(worlds);
    // One depth computation for both edge colors and DAG levels.
    let routes = world::routes_from(&connections, world::origin(worlds));
    let deepest = routes.depth.iter().flatten().copied().max().unwrap_or(0);
    let nexus = opening_room(worlds);
    // Shells count from the Nexus, where the camera stands.
    let from_nexus = nexus.map(|nexus| world::routes_from(&connections, nexus));
    let shells = from_nexus
        .as_ref()
        .map_or(&routes.depth, |from_nexus| &from_nexus.depth);
    let branches = match (&from_nexus, nexus) {
        (Some(from_nexus), Some(nexus)) => branches(from_nexus, nexus),
        _ => vec![None; worlds.len()],
    };
    let outermost = shells.iter().flatten().copied().max().unwrap_or(0).max(1) as f32;
    // Node radius grows with log(descendant count).
    let descendants = routes.descendant_counts();
    let node_radii: Vec<_> = descendants
        .iter()
        .map(|&descendants| {
            let growth = (descendants as f32).ln_1p() / NODE_HUB_DESCENDANTS.ln_1p();
            NODE_LEAF_RADIUS + (NODE_HUB_RADIUS - NODE_LEAF_RADIUS) * growth
        })
        .collect();

    let layout = before.map_or_else(Layout::remembered, |before| before.layout);
    let Layout {
        arrangement,
        hub_repulsion,
        pull_across_branches,
        link_reach,
    } = layout;
    let Solve {
        dag_level_distance,
        dag_layers,
        dag_level_slack,
        force_charge,
        link_distance_max,
        repulsion_range,
    } = layout.parameters();
    let mut graph = Graph::new(SimulationParameters {
        dimensions: arrangement.dimensions(),
        dag_level_distance,
        dag_layers,
        dag_level_slack,
        force_charge,
        link_distance_max,
        repulsion_range,
        settle_after: Some(SETTLE_AFTER),
        ..Default::default()
    });
    // Unreachable worlds get a shell past the deepest.
    let shell_depths: Vec<f32> = shells
        .iter()
        .map(|depth| depth.map_or(outermost + 1.0, |depth| depth as f32))
        .collect();
    let start_depths: Vec<f32> = routes
        .depth
        .iter()
        .map(|depth| depth.map_or(deepest as f32 + 1.0, |depth| depth as f32))
        .collect();
    // Positions come from `scatter`, levels and masses from `set_masses_and_leads`.
    let nodes: Vec<_> = (0..worlds.len())
        .map(|_| graph.add_node(NodeData::default()))
        .collect();
    // Degrees counted from the drawn edges, so the panel matches the graph.
    let mut degrees = vec![0; worlds.len()];
    let mut dashed = 0;
    for (from, steps) in connections.iter().enumerate() {
        for step in steps {
            // One edge per pair; two springs would double the pull.
            if step.world <= from {
                continue;
            }
            let to = step.world;
            // Orient one-way edges in their passable direction, so the dashes move that way.
            let (from, to) = if step.outgoing.is_none() && step.incoming.is_some() {
                (to, from)
            } else {
                (from, to)
            };
            dashed += usize::from(step.one_way());
            graph.add_edge(
                nodes[from],
                nodes[to],
                // Leads are fitted to the finished layout: see `set_masses_and_leads`.
                EdgeData {
                    user_data: step.one_way(),
                    ..Default::default()
                },
            );
            degrees[from] += 1;
            degrees[to] += 1;
        }
    }

    // Depth colors go on the edges; nodes show thumbnails.
    let shares = depth_shares(&routes.depth);
    let depth_colors: Vec<_> = routes
        .depth
        .iter()
        .map(|depth| match depth {
            Some(depth) => distance_color(shares[*depth as usize]),
            None => UNREACHED_COLOR,
        })
        .collect();
    let edge_colors = edge_colors(&graph, &routes, &descendants, &depth_colors);
    // Same source as the atlas cells.
    let unvisited: Vec<usize> = worlds
        .iter()
        .enumerate()
        .filter_map(|(world, it)| it.cell().is_none().then_some(world))
        .collect();
    let titles: Vec<world::Title> = worlds.iter().map(world::World::titles).collect();
    let maps: Vec<Vec<world::Map>> = worlds.iter().map(world::World::maps).collect();
    // Computed once: both iterate every world.
    let (authors, author_of) = dump.authors();
    let versions = dump.versions();
    let thumbnail_instances = white_instances(worlds.len());
    // Colors are set by the first `repaint`.
    let solid = edge_colors.len() - dashed;
    let edge_instances = white_instances(solid);
    let dash_instances = Instances {
        // Repurposed: carries each edge's length to `dash.frag`.
        texture_transformations: Some(vec![Mat3::identity(); dashed]),
        ..white_instances(dashed)
    };
    let backdrop = ColorMaterial {
        color: Srgba::WHITE,
        texture: Some(Texture2DRef::from_cpu_texture(ctx, &panorama_tile())),
        // Color only, no depth test: the quad replaces the clear color.
        render_states: RenderStates {
            write_mask: WriteMask::COLOR,
            depth_test: DepthTest::Always,
            ..Default::default()
        },
        is_transparent: false,
    };
    // Billboarded quads, so the whole image faces the camera.
    let thumbnails = Gm::new(
        RefilledMesh::new(ctx, &thumbnail_instances, &CpuMesh::square()),
        ColorMaterial::default(),
    );
    // Additive, since a highlighted node is already white.
    let glow = Gm::new(
        Mesh::new(ctx, &CpuMesh::square()),
        ColorMaterial {
            color: scale_brightness(Srgba::WHITE, HOVERED_ROW_BRIGHTNESS),
            texture: None,
            render_states: RenderStates {
                write_mask: WriteMask::COLOR,
                // LessOrEqual: the offset toward the camera is smaller than one depth step.
                depth_test: DepthTest::LessOrEqual,
                blend: Blend::ADD,
                ..Default::default()
            },
            is_transparent: true,
        },
    );
    let horizon_marker = Gm::new(
        Mesh::new(ctx, &crosshair()),
        ColorMaterial {
            color: Srgba::WHITE,
            ..Default::default()
        },
    );
    let edges = Gm::new(
        RefilledMesh::ribbon(ctx, &edge_instances, false),
        ColorMaterial::default(),
    );
    let dash_material = || DashMaterial {
        phase: 0.0,
        render_states: RenderStates::default(),
    };
    let dashes = Gm::new(
        RefilledMesh::ribbon(ctx, &dash_instances, true),
        dash_material(),
    );
    // The highlight overlay draws on top after a depth clear.
    let (highlight_thumbnail_instances, highlight_edge_instances) =
        (Instances::default(), Instances::default());
    let highlight_thumbnails = Gm::new(
        RefilledMesh::new(ctx, &highlight_thumbnail_instances, &CpuMesh::square()),
        ColorMaterial::default(),
    );
    let highlight_edges = Gm::new(
        RefilledMesh::ribbon(ctx, &highlight_edge_instances, false),
        ColorMaterial::default(),
    );
    let highlight_dash_instances = Instances {
        texture_transformations: Some(Vec::new()),
        ..Default::default()
    };
    let highlight_dashes = Gm::new(
        RefilledMesh::ribbon(ctx, &highlight_dash_instances, true),
        dash_material(),
    );

    let concealed = world_mask(worlds.len(), unvisited.iter().copied());
    let untaken = untaken(&connections, &concealed);
    let graft = GraphGraft::new(
        &titles
            .iter()
            .map(|title| title.en.as_str())
            .collect::<Vec<_>>(),
        &concealed,
        &routes.depth,
        before,
    );
    let arriving = graft.arriving().is_some();

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
        opening: nexus,
        branches,
        sky_distances: vec![1.0; shell_depths.len()],
        start_depths,
        arrangement,
        shell_depths,
        deepest,
        right_press: None,
        menu: None,
        cursor: None,
        hover: None,
        hovered_row: None,
        row_follow_target: None,
        framing: arriving,
        resetting_camera: false,
        frame_route: false,
        ways: Vec::new(),
        way: 0,
        hub: world::hub_world(worlds),
        edge_colors,
        node_radii,
        hub_repulsion,
        pull_across_branches,
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
        highlight_thumbnails,
        highlight_edges,
        highlight_dashes,
        highlight_thumbnail_instances,
        highlight_edge_instances,
        highlight_dash_instances,
        highlight_nodes: Vec::new(),
        highlight_lines: Vec::new(),
        highlight_dashed: Vec::new(),

        recolored: true,
        glowing: None,
        horizon_marker,
        atlas: Atlas::Loading(thumbnails::load()),
        sheet: None,
        cells: worlds.iter().map(world::World::cell).collect(),
        unvisited: unvisited.clone(),
        unvisited_quads: white_instances(unvisited.len()),
        highlight_unvisited_quads: Instances {
            colors: Some(Vec::new()),
            ..Default::default()
        },
        lod: lod::LodPictures::new(
            worlds.iter().map(|world| world.image.clone()).collect(),
            // Also needed when an arriving world's placeholder is shrinking away.
            !unvisited.is_empty() || graft.turning(),
        ),
        dash_phase: 0.0,
        billboard: Mat4::identity(),
        graft,
    };
    data.repaint();
    data.set_masses_and_leads();
    data.fade_lines_by_depth();
    scatter(&mut data);
    // A rebuilt graph starts from the previous graph's positions.
    if let Some(before) = before {
        resume_from(&mut data, &before.standing);
    }
    data
}
impl AppEntities {
    /// Recolors instances for the current selection: route edges brightest, everything else
    /// dimmed. Uploads directly, since a settled graph never calls [`Self::rebuild_instances`].
    pub(super) fn repaint(&mut self) {
        self.recolored = true;
        let on_route = world_mask(self.titles.len(), self.highlighted());
        let highlighting = self.selected.is_some();
        // Per world on the route, the next world.
        let mut next_on_route = vec![None; self.titles.len()];
        if matches!(
            self.selected,
            Some(Highlight::Route(_) | Highlight::Path(..))
        ) {
            let route = self.route();
            for (at, &world) in route.iter().enumerate() {
                next_on_route[world] = route.get(at + 1).copied();
            }
        }

        let colors = self.thumbnail_instances.colors.as_mut().unwrap();
        for (color, &on_route) in colors.iter_mut().zip(&on_route) {
            *color = if !highlighting || on_route {
                Srgba::WHITE
            } else {
                dim(Srgba::WHITE)
            };
        }
        self.highlight_nodes.clear();
        if highlighting {
            self.highlight_nodes.extend(
                on_route
                    .iter()
                    .enumerate()
                    .filter_map(|(world, &on_route)| on_route.then_some(world)),
            );
        }

        let (routes, on_route, next_on_route) = (&self.routes.parents, &on_route, &next_on_route);
        let (solid, dashed, base) = (
            self.edge_instances.colors.as_mut().unwrap(),
            self.dash_instances.colors.as_mut().unwrap(),
            &self.edge_colors,
        );
        let (highlight_lines, highlight_dashed) =
            (&mut self.highlight_lines, &mut self.highlight_dashed);
        highlight_lines.clear();
        highlight_dashed.clear();
        // Same order and split as [`Self::rebuild_instances`] and [`Self::update_dashes`].
        let (mut edge, mut line, mut dash) = (0, 0, 0);
        self.graph.visit_edges(|a, b, data| {
            let (a, b) = (a.index().index(), b.index().index());
            let step = routes[a] == Some(b) || routes[b] == Some(a);
            let (color, shown) = match self.selected {
                None => (base[edge], false),
                // Only this edge, not the route home from either end.
                Some(Highlight::Connection(at, far)) => {
                    if (a, b) == (at, far) || (a, b) == (far, at) {
                        (ROUTE_COLOR, true)
                    } else {
                        (dim(base[edge]), false)
                    }
                }
                // Edges within a layer are never tree edges, so they keep their depth color.
                Some(Highlight::Layer(_)) if on_route[a] && on_route[b] => (base[edge], true),
                // Checked before the tree-edge case, so a shortcut across the route stays dimmed.
                Some(_) if next_on_route[a] == Some(b) || next_on_route[b] == Some(a) => {
                    (ROUTE_HOME_COLOR, true)
                }
                // Paths do not follow the canonical tree.
                Some(Highlight::Path(..)) => (dim(base[edge]), false),
                Some(_) if step && on_route[a] && on_route[b] => (ROUTE_COLOR, true),
                Some(_) => (dim(base[edge]), false),
            };
            if data.user_data {
                dashed[dash] = color;
                if shown {
                    highlight_dashed.push(dash);
                }
                dash += 1;
            } else {
                solid[line] = color;
                if shown {
                    highlight_lines.push(line);
                }
                line += 1;
            }
            edge += 1;
        });

        self.thumbnails
            .set_colors(self.thumbnail_instances.colors.as_deref());
        self.edges.set_colors(self.edge_instances.colors.as_deref());
        self.dashes
            .set_colors(self.dash_instances.colors.as_deref());
        self.copy_highlighted_instances();
    }

    /// Copies the highlighted slots into the overlay buffers; call after any write to the main
    /// buffers.
    fn copy_highlighted_instances(&mut self) {
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
            &mut self.highlight_edge_instances,
            &[&self.edge_instances],
            &[&self.highlight_lines],
        );
        gather(
            &mut self.highlight_dash_instances,
            &[&self.dash_instances],
            &[&self.highlight_dashed],
        );
        gather(
            &mut self.highlight_thumbnail_instances,
            &[&self.thumbnail_instances],
            &[&self.highlight_nodes],
        );
        let picked = |from: Option<&Vec<Mat3>>, at: &[usize]| {
            from.map(|from| at.iter().map(|&at| from[at]).collect())
        };
        self.highlight_thumbnail_instances.texture_transformations = picked(
            self.thumbnail_instances.texture_transformations.as_ref(),
            &self.highlight_nodes,
        );
        self.highlight_dash_instances.texture_transformations = picked(
            self.dash_instances.texture_transformations.as_ref(),
            &self.highlight_dashed,
        );
        self.highlight_edges
            .set_instances(&self.highlight_edge_instances);
        self.highlight_dashes
            .set_instances(&self.highlight_dash_instances);
        self.highlight_thumbnails
            .set_instances(&self.highlight_thumbnail_instances);
    }

    /// Highlight overlay objects, thumbnails before edges so their depth culls hidden edge
    /// fragments. Empty without a selection; thumbnails wait for the atlas.
    pub(super) fn drawn_highlighted(&self) -> impl Iterator<Item = &dyn Object> {
        let pictures = self.highlight_thumbnails.material.texture.is_some();
        [
            (pictures && !self.highlight_nodes.is_empty())
                .then_some(&self.highlight_thumbnails as &dyn Object),
            (!self.highlight_lines.is_empty()).then_some(&self.highlight_edges as &dyn Object),
            (!self.highlight_dashed.is_empty()).then_some(&self.highlight_dashes as &dyn Object),
        ]
        .into_iter()
        .flatten()
        .chain(self.lod.drawn_highlighted(&self.highlight_nodes))
    }

    /// Whether the overlay draws anything, and so needs a depth clear.
    pub(super) fn has_highlight_overlay(&self) -> bool {
        self.drawn_highlighted().next().is_some()
    }

    fn edge_radius(&self) -> f32 {
        EDGE_RADIUS
    }

    /// `laid_out`: whether node positions changed. Billboards depend on camera rotation, edges only
    /// on positions.
    pub(super) fn rebuild_instances(&mut self, camera: &Camera, laid_out: bool) {
        self.rebuild_nodes(camera);
        if laid_out {
            self.rebuild_edges();
        }
        self.copy_highlighted_instances();
    }

    fn rebuild_nodes(&mut self, camera: &Camera) {
        let billboard = billboard(camera);
        let mut thumbnails = std::mem::take(&mut self.thumbnail_instances.transformations);
        thumbnails.clear();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            let position = world_pos(node.position(), self.sky_distances[world]);
            let radius = self.node_radii[world] * self.graft.grown(world);
            thumbnails.push(node_quad(position, billboard, radius));
        });
        self.thumbnail_instances.transformations = thumbnails;
        self.thumbnails
            .set_transformations(&self.thumbnail_instances.transformations);
        self.billboard = billboard;
    }

    /// Dash ribbons are rebuilt with the solid edges; `DashMaterial` animates them in the shader.
    /// On the 3D sky a connection between branches is drawn only when highlighted.
    fn rebuild_edges(&mut self) {
        let hides_crossings = self.on_sky();
        let branches = &self.branches;
        let (mut shown_edges, mut shown_dashes) = (Vec::new(), Vec::new());
        let radius = self.edge_radius();
        let (graft, distances) = (&self.graft, &self.sky_distances);
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
            let (a_world, b_world) = (a.index().index(), b.index().index());
            let from = world_pos(a.position(), distances[a_world]);
            let to = world_pos(b.position(), distances[b_world]);
            let dir = to - from;
            let span = dir.magnitude();
            let grown = graft.grown(a_world).min(graft.grown(b_world));
            let one_way = data.user_data;
            let radius = grown * if one_way { dash_radius } else { radius };
            // Center to center: the opaque thumbnails draw first and hide the edge ends.
            let placed = match span > 0.0 {
                true => {
                    Mat4::from_translation(from)
                        * rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), dir / span)
                        * Mat4::from_nonuniform_scale(span, radius, radius)
                }
                // Coincident nodes would divide by zero.
                false => Mat4::from_scale(0.0),
            };
            let shown = match hides_crossings && crosses_branches(branches, a_world, b_world) {
                true => Mat4::from_scale(0.0),
                false => placed,
            };
            match one_way {
                false => {
                    edges.push(placed);
                    shown_edges.push(shown);
                }
                true => {
                    dashes.push(placed);
                    shown_dashes.push(shown);
                    spans.push(Mat3::from_nonuniform_scale(span, 1.0));
                }
            }
        });
        self.edges.set_transformations(&shown_edges);
        self.dashes.set_transformations(&shown_dashes);
        self.dashes
            .set_texture_transformations(self.dash_instances.texture_transformations.as_deref());
    }

    /// Places the placeholder over every unvisited world, every frame, as a copy of the node quad
    /// offset toward the camera to avoid z-fighting.
    pub(super) fn place_unvisited(&mut self, context: &Context, camera: &Camera) {
        if self.unvisited.is_empty() && !self.graft.turning() {
            return;
        }
        let eye = camera.position();
        let nodes = &self.thumbnail_instances;
        let painted = nodes.colors.as_ref();
        let wearing = self.unvisited.iter().copied().chain(self.graft.veiled());
        let (transformations, colors) = (
            &mut self.unvisited_quads.transformations,
            self.unvisited_quads.colors.as_mut().unwrap(),
        );
        let (highlight_transformations, highlight_colors) = (
            &mut self.highlight_unvisited_quads.transformations,
            self.highlight_unvisited_quads.colors.as_mut().unwrap(),
        );
        let highlight_nodes = &self.highlight_nodes;
        transformations.clear();
        colors.clear();
        highlight_transformations.clear();
        highlight_colors.clear();
        for world in wearing {
            let Some(node) = nodes.transformations.get(world) else {
                continue;
            };
            let placed = lift_toward_eye(eye, 2.0) * node;
            let color = painted.map_or(Srgba::WHITE, |painted| painted[world]);
            // `highlight_nodes` is in world order.
            if highlight_nodes.binary_search(&world).is_ok() {
                highlight_transformations.push(placed);
                highlight_colors.push(color);
            }
            transformations.push(placed);
            colors.push(color);
        }
        self.lod.place_unvisited(
            context,
            &self.unvisited_quads,
            &self.highlight_unvisited_quads,
        );
    }

    /// See [`sky_line_fade`].
    pub(super) fn fade_lines_by_depth(&mut self) {
        let fade = sky_line_fade(self.arrangement);
        self.edges.geometry.line_fade = fade;
        self.dashes.geometry.line_fade = fade;
        self.highlight_edges.geometry.line_fade = fade;
        self.highlight_dashes.geometry.line_fade = fade;
    }

    /// Every frame, even when the layout is settled.
    pub(super) fn update_dashes(&mut self, dt: f32) {
        // Wrapped at one period to keep f32 precision.
        self.dash_phase = (self.dash_phase + dt * EDGE_DASH_SPEED).rem_euclid(EDGE_DASH_PERIOD);
        self.dashes.material.phase = self.dash_phase;
        self.highlight_dashes.material.phase = self.dash_phase;
    }

    /// Sets each thumbnail's UVs and gives egui a copy once the atlas loads; a failed load retries
    /// after [`ATLAS_RETRIED_AFTER_SECONDS`] of frame time, so an idle app does not.
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
        self.thumbnails.set_texture_transformations(Some(&cells));
        self.thumbnail_instances.texture_transformations = Some(cells);
        // One upload; cloning a [`Texture2DRef`] is cheap (an `Arc` and a `Mat3`).
        let atlas = Texture2DRef::from_cpu_texture(context, &atlas);
        self.glow.material.texture = Some(atlas.clone());
        self.highlight_thumbnails.material.texture = Some(atlas.clone());
        // Last: [`Self::drawn_thumbnails`] treats this texture as the ready signal.
        self.thumbnails.material.texture = Some(atlas);
        self.copy_highlighted_instances();
    }

    /// `None` until the atlas loads; an untextured [`ColorMaterial`] would draw flat white quads.
    pub(super) fn drawn_thumbnails(&self) -> Option<&dyn Object> {
        self.thumbnails
            .material
            .texture
            .is_some()
            .then_some(&self.thumbnails as &dyn Object)
    }

    /// Places the glow quad on the hovered row's world: the node quad lifted in front of the
    /// high-LoD quad and the placeholder.
    pub(super) fn aim_glow(&mut self, camera: &Camera) {
        let Some(world) = self.hovered_row else {
            return;
        };
        let Some(cells) = self.thumbnail_instances.texture_transformations.as_ref() else {
            return;
        };
        let Some(quad) = self.thumbnail_instances.transformations.get(world) else {
            return;
        };
        let quad = lift_toward_eye(camera.position(), 3.0) * quad;
        self.glow.set_transformation(quad);
        if let Some(texture) = self.glow.material.texture.as_mut() {
            texture.transformation = cells[world];
        }
    }

    /// Places the crosshair on the horizon at heading zero, past every world, at a fixed screen
    /// size. Upright, with the stripe along the horizon and the tick along the meridian.
    pub(super) fn aim_horizon_marker(&mut self, camera: &Camera) {
        let half_height = match camera.projection_type() {
            three_d_asset::ProjectionType::Perspective { field_of_view_y } => {
                (field_of_view_y.0 * 0.5).tan()
            }
            _ => return,
        };
        let size = HORIZON_MARKER_DISTANCE * half_height * HORIZON_MARKER_SHARE;
        let at = camera.position() + heading_zero() * HORIZON_MARKER_DISTANCE;
        self.horizon_marker
            .set_transformation(Mat4::from_translation(at) * Mat4::from_scale(size));
    }

    /// `None` off the 3D sky, where the camera does not stand in place.
    pub(super) fn drawn_horizon_marker(&self) -> Option<&dyn Object> {
        self.on_sky().then_some(&self.horizon_marker as &dyn Object)
    }

    /// `None` without a hovered row or a loaded atlas.
    pub(super) fn drawn_glow(&self) -> Option<&dyn Object> {
        let ready = self.hovered_row.is_some() && self.glow.material.texture.is_some();
        ready.then_some(&self.glow as &dyn Object)
    }

    /// On-screen worlds wider than [`lod::HI_LOD_EXIT_PIXELS`], nearest first, with their quads;
    /// arriving worlds count at full size to have the high LoD ready.
    pub(super) fn lod_candidates(
        &self,
        camera: &Camera,
        viewport: Viewport,
    ) -> Vec<lod::LodCandidate> {
        let billboard = billboard(camera);
        let forward = camera.view_direction();
        let colors = self.thumbnail_instances.colors.as_ref();
        let across = camera.right_direction().normalize();
        let mut candidates: Vec<(f32, lod::LodCandidate)> = Vec::new();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            let position = world_pos(node.position(), self.sky_distances[world]);
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
            let grown = self.graft.grown(world);
            // Measured at full size, drawn at the current one.
            let width = drawn_width(camera, across, full, position, center);
            if width < lod::HI_LOD_EXIT_PIXELS {
                return;
            }
            let radius = full * grown;
            candidates.push((
                (position - camera.position()).magnitude2(),
                lod::LodCandidate {
                    world,
                    width,
                    transformation: lift_toward_eye(camera.position(), 1.0)
                        * node_quad(position, billboard, radius),
                    color: colors.map_or(Srgba::WHITE, |colors| colors[world]),
                },
            ));
        });
        // Sorted by distance: perspective makes corner nodes up to a third wider.
        candidates.sort_by(|(a, _), (b, _)| a.total_cmp(b));
        candidates.into_iter().map(|(_, it)| it).collect()
    }
}
/// Background tile: a grid of soft blue glows, in the style of the game's panoramas. Linear f16:
/// 8-bit linear shows banding at these brightnesses.
fn panorama_tile() -> CpuTexture {
    let cells = PANORAMA_TILE_TEXELS / PANORAMA_CELL_TEXELS;
    let mut data = Vec::with_capacity(PANORAMA_TILE_TEXELS * PANORAMA_TILE_TEXELS);
    for y in 0..PANORAMA_TILE_TEXELS {
        for x in 0..PANORAMA_TILE_TEXELS {
            // Include neighbouring cells so glows crossing the tile edge wrap seamlessly.
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

/// Inverse of the shader's output sRGB encoding.
fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// Background UV transform: fixed tile size in logical pixels, offset by [`PANORAMA_PARALLAX`]
/// from the view direction, which stays continuous where yaw wraps.
pub(super) fn panorama_transform(viewport: Viewport, device_pixel_ratio: f32, view: Vec3) -> Mat3 {
    let tile = PANORAMA_TILE_PIXELS * device_pixel_ratio;
    let drift = vec2(view.x, view.y) * PANORAMA_PARALLAX;
    Mat3::from_translation(drift)
        * Mat3::from_nonuniform_scale(viewport.width as f32 / tile, viewport.height as f32 / tile)
}
/// Depth color ramp over 5/6 of the hue wheel, blue to magenta. Explicit RGB stops keep
/// interpolation saturated; the blue is lightened against [`BACKGROUND_COLOR`].
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

/// Each depth's place on the color ramp, by how many reachable worlds are shallower: crowded
/// depths spread across most of the hues and the sparse deep ones share the last.
fn depth_shares(depths: &[Option<u32>]) -> Vec<f32> {
    let deepest = depths.iter().flatten().copied().max().unwrap_or(0) as usize;
    let mut counts = vec![0u32; deepest + 1];
    for &depth in depths.iter().flatten() {
        counts[depth as usize] += 1;
    }
    // Each depth at the middle of the span of worlds at that depth.
    let mut shallower = 0;
    let middles: Vec<f32> = counts
        .iter()
        .map(|&count| {
            let middle = shallower as f32 + count as f32 / 2.0;
            shallower += count;
            middle
        })
        .collect();
    let (first, last) = (middles[0], middles[deepest]);
    middles
        .iter()
        .map(|middle| (middle - first) / (last - first).max(f32::MIN_POSITIVE))
        .collect()
}

/// Darkens a color outside the highlight.
fn dim(color: Srgba) -> Srgba {
    scale_brightness(color, DIMMED_BRIGHTNESS)
}

/// Luma (Rec. 709 weights on sRGB values), used by [`EDGE_LUMINANCE_EVENNESS`].
fn luminance(color: Srgba) -> f32 {
    (0.2126 * color.r as f32 + 0.7152 * color.g as f32 + 0.0722 * color.b as f32) / 255.0
}

/// Scales RGB, keeping alpha.
fn scale_brightness(color: Srgba, brightness: f32) -> Srgba {
    let scale = |channel: u8| (channel as f32 * brightness) as u8;
    Srgba::new(scale(color.r), scale(color.g), scale(color.b), color.a)
}

/// Unselected edge colors: the source's depth color, brightened by log(target's descendants). The
/// source is the tree parent, or the shallower end of a non-tree edge.
fn edge_colors(
    graph: &Graph,
    routes: &world::Routes,
    descendants: &[u32],
    depth_colors: &[Srgba],
) -> Vec<Srgba> {
    // Normalized against the world with the most descendants.
    let busiest = (descendants.iter().copied().max().unwrap_or(0) as f32).ln_1p();
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
        let reach = (descendants[to] as f32).ln_1p() / busiest.max(f32::MIN_POSITIVE);
        // Never above 1: [`UNREACHED_COLOR`] is off the ramp and stays unchanged.
        let even = (floor / luminance(depth_colors[from]).max(f32::MIN_POSITIVE)).min(1.0);
        colors.push(scale_brightness(
            depth_colors[from],
            (EDGE_LEAF_BRIGHTNESS + (1.0 - EDGE_LEAF_BRIGHTNESS) * reach)
                * (1.0 - EDGE_LUMINANCE_EVENNESS * (1.0 - even)),
        ));
    });
    colors
}

/// `count` identity instances, white so textures show unmodified.
fn white_instances(count: usize) -> Instances {
    Instances {
        transformations: vec![Mat4::identity(); count],
        colors: Some(vec![Srgba::WHITE; count]),
        ..Default::default()
    }
}

/// The horizon marker: a short vertical tick across a longer horizontal stripe, on `z = 0`.
fn crosshair() -> CpuMesh {
    let rectangle = |half_width: f32, half_height: f32| {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(x, y)| vec3(x * half_width, y * half_height, 0.0))
    };
    let stripe = rectangle(HORIZON_STRIPE_LENGTH, HORIZON_LINE_WIDTH);
    let tick = rectangle(HORIZON_LINE_WIDTH, 1.0);
    CpuMesh {
        positions: Positions::F32([stripe, tick].concat()),
        indices: Indices::U8(vec![0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]),
        ..Default::default()
    }
}

/// Shrinks a quad toward the eye by `lifts` [`lod::LIFT`]s: the same pixels, drawn in front of
/// whatever the quad covers, at any distance.
fn lift_toward_eye(eye: Vec3, lifts: f32) -> Mat4 {
    let kept = 1.0 - lod::LIFT * lifts;
    Mat4::from_translation(eye) * Mat4::from_scale(kept) * Mat4::from_translation(-eye)
}

/// A world's billboard quad at `position`, in the thumbnail aspect ratio.
fn node_quad(position: Vec3, billboard: Mat4, radius: f32) -> Mat4 {
    Mat4::from_translation(position)
        * billboard
        * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0)
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
