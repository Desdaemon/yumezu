//! What the force simulation is given, and the conversion between simulation units and the
//! rendering space. See [`Layout`] and [`Layout::parameters`].

use super::*;

/// Side of the cube, centred on the origin, that nodes are scattered over at the start of a
/// layout, in simulation units.
const SPAWN_EXTENT: f32 = 1000.0;
/// Rendering-space units per pixel-sized simulation unit; "world" in [`world_pos`] and
/// [`sim_pos`] means this rendering space, not a game world.
const SIM_TO_WORLD: f32 = 0.012;
/// The smallest node the graph draws, and the radius every other size and mass is read against.
pub(super) const NODE_LEAF_RADIUS: f32 = 1.1;
/// Radius of a world with [`NODE_HUB_DESCENDANTS`] descendants. With [`NODE_LEAF_RADIUS`], an
/// anchor of the logarithmic size curve, which continues past this to larger hubs.
pub(super) const NODE_HUB_RADIUS: f32 = 1.7;
/// Mass of a world of [`NODE_LEAF_RADIUS`]. [`node_mass`] scales larger worlds up from this.
const NODE_BASE_MASS: f32 = 21.0;
/// Default weight of size in repulsion: at `0.0` every world repels alike, at `1.0` by size.
const HUB_REPULSION_DEFAULT: f32 = 0.83;

const FORCE_CHARGE: f32 = 1800.0;
const FORCE_CHARGE_2D: f32 = 10000.0;
pub(super) const SETTLE_AFTER: f32 = 32.0;

/// Descendant count at [`NODE_HUB_RADIUS`], which sets how quickly sizes grow. The two radii
/// stay close: a wider range makes hubs overlap their neighbours.
pub(super) const NODE_HUB_DESCENDANTS: f32 = 4.0;
/// Share of a world's extra size, times the hub push, added to how far the world's connections
/// may stretch: a hub's crowd of connections needs more room than a leaf's few. See [`edge_reach`].
const HUB_REACH: f32 = 0.5;
/// In the radial arrangements, the subtree size whose patch one connection length spans; a lead
/// grows with the square root of the subtree, as a patch's width does. See [`edge_lead`].
const PATCH_WORLDS_PER_LEAD: f32 = 16.0;
/// How many times longer a one-way connection's lead is than a two-way one's inside a branch,
/// so a chute still reads as a long line. See [`edge_lead`].
const ONE_WAY_LEAD: f32 = 2.5;
/// The 3D unit of length outside the tree, in simulation units: what a connection's lead and
/// the push's reach are measured in. See [`SKY_SHELLS`].
const DAG_LEVEL_DISTANCE: f32 = 4000.0;
/// Distance between depth rings in 2D layered mode, in simulation units.
const DAG_LEVEL_DISTANCE_2D: f32 = 6000.0;
/// Distance between the [`Arrangement::Tree`]'s depth planes, in simulation units.
const TREE_LEVEL_DISTANCE: f32 = 1300.0;
/// Distance between the [`Arrangement::FlatTree`]'s depth rows, in simulation units.
const TREE_LEVEL_DISTANCE_2D: f32 = 1900.0;
/// Distance from the Nexus to the orbital arrangement's sky, which holds every other world, in
/// [`DAG_LEVEL_DISTANCE`]s: about as much room per world as in the busy rings.
const SKY_SHELLS: f32 = 3.0;
/// How many times farther than the sky the first depth is drawn on the 3D sky. See
/// [`sky_distance`].
const SKY_NEAREST_DISTANCE: f32 = 1.3;
/// How much farther than the first each depth is drawn on the 3D sky, per e-fold of depth: the
/// shallow depths, which hold most of the worlds, sit the farthest apart.
const SKY_DEPTH_SPREAD: f32 = 0.5;
/// How far past the first depth a line on the 3D sky dims to 1/e brightness, in skies.
const SKY_LINE_FADE_LENGTH: f32 = 1.6;
/// How many rings or shells out from the Nexus keep the whole spacing between them: the
/// shared core, which holds most of the worlds and most of the connections between branches.
const CORE_SHELLS: f32 = 4.0;
/// The gap between shells past [`CORE_SHELLS`], as a fraction of the spacing. The
/// deep worlds are few and mostly in chains, which need less room.
const OUTER_SHELL_GAP: f32 = 0.5;
/// How far a world may sit from the shell per microstep of slack, in simulation units: the width
/// of two leaf worlds, so one step leaves the separated worlds touching.
const DAG_LEVEL_MICROSTEP: f32 = 4.0 * NODE_LEAF_RADIUS / SIM_TO_WORLD;
/// How many [`DAG_LEVEL_MICROSTEP`]s of slack a flat layer has, to spread a crowded line; the
/// band must stay well under [`DAG_LEVEL_DISTANCE_2D`] so the layers stay apart.
const DAG_LEVEL_SLACK_MICROSTEPS_2D: f32 = 3.0;
/// How far a world's push reaches when radial, as a multiple of the shell spacing. Short: the
/// branch patches spread the worlds, and the push only keeps neighbours apart.
const REPULSION_RANGE_SHELLS: f32 = 0.25;
const ARRANGEMENT: &str = "arrangement";
/// Read only, for a store written before [`ARRANGEMENT`].
const DIMENSIONS: &str = "dimensions";
/// Read only, for a store written before [`ARRANGEMENT`].
const LAYERED: &str = "layered";
const HUB_REPULSION: &str = "hub-push";
const PULL_ACROSS_BRANCHES: &str = "pull-across-branches";
/// Key prefix, followed by the arrangement's name: one link reach per arrangement.
const LINK_REACH: &str = "link-reach-";

/// How far the hub push can be taken either way from [`HUB_REPULSION_DEFAULT`].
pub(super) const HUB_REPULSION_RANGE: std::ops::RangeInclusive<f32> = 0.5..=20.0;

/// Bounds of the link reach, in layers: from a pull that clumps connected pairs to past the
/// busiest layer's radius, which is no limit. See [`Arrangement::default_link_reach`].
pub(super) const LINK_REACH_RANGE: std::ops::RangeInclusive<f32> = 0.5..=6.0;
/// The simulation parameters a [`Layout`] decides.
pub(super) struct Solve {
    pub(super) dag_level_distance: Option<f32>,
    pub(super) dag_layers: Layers,
    pub(super) dag_level_slack: f32,
    pub(super) force_charge: f32,
    pub(super) link_distance_max: Option<f32>,
    pub(super) repulsion_range: Option<f32>,
}

/// How the worlds are arranged: on a plane or in space, and whether by their depth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Arrangement {
    /// On a plane, where the forces leave them.
    Flat,
    /// On a plane, in rings by depth around the Nexus.
    Radial,
    /// On a plane, in rows across y by depth from the game's start, held by the forces alone.
    FlatTree,
    /// In space, where the forces leave them.
    Free,
    /// In space, on planes across y by depth from the game's start, held by the forces alone.
    Tree,
    /// On one sky around the Nexus, deeper worlds drawn farther out. See [`sky_distance`].
    Orbital,
}

impl Arrangement {
    pub(super) const PLANAR: [Self; 3] = [Self::Flat, Self::Radial, Self::FlatTree];
    pub(super) const SPATIAL: [Self; 3] = [Self::Free, Self::Tree, Self::Orbital];

    pub(super) fn dimensions(self) -> Dimensions {
        match self {
            Self::Flat | Self::Radial | Self::FlatTree => Dimensions::Two,
            Self::Free | Self::Tree | Self::Orbital => Dimensions::Three,
        }
    }

    /// Whether each world is held to a layer set by depth.
    pub(super) fn layered(self) -> bool {
        match self {
            Self::Flat | Self::Free => false,
            Self::Radial | Self::FlatTree | Self::Tree | Self::Orbital => true,
        }
    }

    /// Whether each world is held at a distance from the Nexus set by depth, inside the branch's
    /// patch and on loose leads. See [`edge_lead`].
    pub(super) fn radial(self) -> bool {
        match self {
            Self::Radial | Self::Orbital => true,
            Self::Flat | Self::FlatTree | Self::Free | Self::Tree => false,
        }
    }

    /// Default length a connection stretches before going taut, in layer spacings: long in the
    /// tree, whose planes are close; short on the sky, where branch patches spread the worlds.
    fn default_link_reach(self) -> f32 {
        match self {
            Self::Flat | Self::FlatTree | Self::Free => 2.0,
            Self::Radial => 1.5,
            Self::Tree => 3.0,
            Self::Orbital => 1.0,
        }
    }

    /// The link reach last chosen for this arrangement.
    pub(super) fn remembered_link_reach(self) -> f32 {
        store::read_clamped(&format!("{LINK_REACH}{}", self.stored()), LINK_REACH_RANGE)
            .unwrap_or_else(|| self.default_link_reach())
    }

    pub(super) fn name(self) -> String {
        match self {
            Self::Flat => t!("arrangement-flat"),
            Self::Radial => t!("arrangement-radial"),
            Self::Free => t!("arrangement-free"),
            Self::FlatTree | Self::Tree => t!("arrangement-tree"),
            Self::Orbital => t!("arrangement-orbital"),
        }
    }

    fn stored(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Radial => "radial",
            Self::Free => "free",
            Self::FlatTree => "flat-tree",
            Self::Tree => "tree",
            Self::Orbital => "orbital",
        }
    }

    /// From a store written before [`ARRANGEMENT`], which held two switches.
    fn from_switches(dimensions: Dimensions, layered: bool) -> Self {
        match (dimensions, layered) {
            (Dimensions::Two, false) => Self::Flat,
            (Dimensions::Two, true) => Self::Radial,
            (Dimensions::Three, false) => Self::Free,
            (Dimensions::Three, true) => Self::Orbital,
        }
    }
}

/// The user's layout choices, kept together so a rebuild carries all of them. See [`Before`]
/// and [`Layout::remembered`].
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Layout {
    pub(super) arrangement: Arrangement,
    /// Weight of size in a world's repulsion. See [`node_mass`].
    pub(super) hub_repulsion: f32,
    /// Whether connections between the Nexus's branches pull in the radial arrangements. See
    /// [`edge_lead`].
    pub(super) pull_across_branches: bool,
    /// How far a connection may stretch, in layers, in this arrangement. See
    /// [`Arrangement::default_link_reach`].
    pub(super) link_reach: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            arrangement: Arrangement::Orbital,
            hub_repulsion: HUB_REPULSION_DEFAULT,
            pull_across_branches: false,
            link_reach: Arrangement::Orbital.default_link_reach(),
        }
    }
}

impl Layout {
    /// Reads each part separately, so a store from an older version still yields the parts present.
    pub(super) fn remembered() -> Self {
        let fallback = Self::default();
        let stored = store::read(ARRANGEMENT);
        let chosen = (Arrangement::PLANAR.into_iter().chain(Arrangement::SPATIAL))
            .find(|arrangement| stored.as_deref() == Some(arrangement.stored()));
        let switches = || {
            let dimensions = match store::read(DIMENSIONS)?.as_str() {
                "2" => Dimensions::Two,
                "3" => Dimensions::Three,
                _ => return None,
            };
            let layered = store::read(LAYERED)?.parse().ok()?;
            Some(Arrangement::from_switches(dimensions, layered))
        };
        let arrangement = chosen.or_else(switches).unwrap_or(fallback.arrangement);
        Self {
            arrangement,
            hub_repulsion: store::read_clamped(HUB_REPULSION, HUB_REPULSION_RANGE)
                .unwrap_or(fallback.hub_repulsion),
            pull_across_branches: store::read_switch(PULL_ACROSS_BRANCHES)
                .unwrap_or(fallback.pull_across_branches),
            link_reach: arrangement.remembered_link_reach(),
        }
    }

    pub(super) fn remember(&self) {
        store::write(ARRANGEMENT, Some(self.arrangement.stored()));
        store::write(HUB_REPULSION, Some(&self.hub_repulsion.to_string()));
        store::write_switch(PULL_ACROSS_BRANCHES, self.pull_across_branches);
        let link_reach = format!("{LINK_REACH}{}", self.arrangement.stored());
        store::write(&link_reach, Some(&self.link_reach.to_string()));
    }

    /// In two dimensions a crowd can only spread sideways, so the layers sit farther apart and the
    /// push is far stronger. A connection's reach follows the layer spacing.
    pub(super) fn parameters(&self) -> Solve {
        let (spacing, slack, charge) = match self.arrangement {
            Arrangement::Flat | Arrangement::Radial => (
                DAG_LEVEL_DISTANCE_2D,
                DAG_LEVEL_MICROSTEP * DAG_LEVEL_SLACK_MICROSTEPS_2D,
                FORCE_CHARGE_2D,
            ),
            Arrangement::Free | Arrangement::Orbital => (DAG_LEVEL_DISTANCE, 0.0, FORCE_CHARGE),
            Arrangement::FlatTree => (
                TREE_LEVEL_DISTANCE_2D,
                DAG_LEVEL_MICROSTEP * DAG_LEVEL_SLACK_MICROSTEPS_2D,
                FORCE_CHARGE_2D,
            ),
            Arrangement::Tree => (TREE_LEVEL_DISTANCE, 0.0, FORCE_CHARGE),
        };
        let radial = self.arrangement.radial();
        Solve {
            dag_level_distance: self.arrangement.layered().then_some(spacing),
            dag_layers: match radial {
                true => Layers::Shells,
                false => Layers::Planes,
            },
            dag_level_slack: slack,
            force_charge: charge,
            link_distance_max: Some(spacing * self.link_reach),
            repulsion_range: radial.then_some(spacing * REPULSION_RANGE_SHELLS),
        }
    }
}
pub(super) struct Rng(pub(super) u32);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}
/// Restarts the layout, scattering each node at rest into the branch's patch of the shell when
/// radial, else over the spawn volume. Switching to 3D needs this: a flat graph has no z repulsion.
pub(super) fn scatter(data: &mut AppEntities) {
    let planar = data.dimensions() == Dimensions::Two;
    let spacing =
        (data.graph.parameters().dag_level_distance).filter(|_| data.arrangement.radial());
    let patches = branch_patches(&data.branches, planar);
    let (branches, rng) = (&data.branches, &mut data.rng);
    let mut world = 0;
    data.graph.visit_nodes_mut(|mut node| {
        let patch = branches[world].and_then(|branch| patches[branch]);
        world += 1;
        let position = match (spacing, patch) {
            (
                Some(spacing),
                Some(Patch {
                    u,
                    v,
                    width,
                    height,
                }),
            ) => {
                let longitude = (u + rng.next_f32() * width) * std::f32::consts::TAU;
                let y = match planar {
                    true => 0.0,
                    false => 2.0 * (v + rng.next_f32() * height) - 1.0,
                };
                let ring = (1.0 - y * y).sqrt();
                let radius = node.level() * spacing;
                let [x, z] = [ring * longitude.cos(), ring * longitude.sin()];
                match planar {
                    true => [x * radius, z * radius, 0.0],
                    false => [x * radius, y * radius, z * radius],
                }
            }
            _ => {
                let mut spawn = || (rng.next_f32() - 0.5) * SPAWN_EXTENT;
                [spawn(), spawn(), if planar { 0.0 } else { spawn() }]
            }
        };
        node.set_position(position);
        node.set_velocity([0.0; 3]);
    });
}

/// The layer a world at `depth` is held to, in layer spacings: the sky when orbital, one plane per
/// depth in the tree, else rings or shells one apart through the core and closer past the core.
pub(super) fn shell_level(depth: f32, arrangement: Arrangement) -> f32 {
    match (arrangement, depth > 0.0) {
        (Arrangement::Orbital, true) => SKY_SHELLS,
        (Arrangement::Orbital, false) => 0.0,
        (Arrangement::Tree | Arrangement::FlatTree, _) => depth,
        _ => depth.min(CORE_SHELLS) + (depth - CORE_SHELLS).max(0.0) * OUTER_SHELL_GAP,
    }
}

/// The gap between the shell at `depth` and the next shell in, in layer spacings: what a
/// connection's lead is scaled by. One throughout on the sky, where there are no shells.
pub(super) fn shell_gap(depth: f32, arrangement: Arrangement) -> f32 {
    match arrangement {
        Arrangement::Orbital => 1.0,
        _ => shell_level(depth, arrangement) - shell_level(depth - 1.0, arrangement),
    }
}

/// How many times farther than the sky a world at `depth` is drawn from the Nexus: direction from
/// the layout on the sky, distance from depth, as with a star.
pub(super) fn sky_distance(depth: f32, arrangement: Arrangement) -> f32 {
    match arrangement {
        Arrangement::Orbital => {
            SKY_NEAREST_DISTANCE * (1.0 + SKY_DEPTH_SPREAD * depth.max(1.0).ln())
        }
        _ => 1.0,
    }
}

/// On the 3D sky, lines dim with distance from the Nexus, and so toward their deeper end.
pub(super) fn sky_line_fade(arrangement: Arrangement) -> Option<LineFade> {
    let sky = SKY_SHELLS * DAG_LEVEL_DISTANCE * SIM_TO_WORLD;
    match arrangement {
        Arrangement::Orbital => Some(LineFade {
            whole_within: sky * sky_distance(1.0, arrangement),
            e_fold_length: sky * SKY_LINE_FADE_LENGTH,
        }),
        _ => None,
    }
}

/// Per world, the first world on the route from the Nexus, or `None` for the Nexus and
/// unreachable worlds.
pub(super) fn branches(routes: &world::Routes, nexus: usize) -> Vec<Option<usize>> {
    (0..routes.parents.len())
        .map(|mut world| {
            loop {
                match routes.parents[world] {
                    Some(parent) if parent == nexus => return Some(world),
                    Some(parent) => world = parent,
                    None => return None,
                }
            }
        })
        .collect()
}

/// Whether a connection joins two branches, or touches the Nexus or an unreachable world.
pub(super) fn crosses_branches(branches: &[Option<usize>], from: usize, to: usize) -> bool {
    branches[from] != branches[to] || branches[from].is_none()
}

/// A rectangle of the sphere's equal-area map: `u` runs round the longitude and `v` up the
/// height, both from 0 to 1, so a patch's area is a share of the sphere.
#[derive(Clone, Copy)]
struct Patch {
    u: f32,
    v: f32,
    width: f32,
    height: f32,
}

/// Per branch's first world, the patch of the sphere the branch is laid out in, sized by the
/// branch's world count. Planar, every patch is a whole-height slice: an arc of the ring.
fn branch_patches(branches: &[Option<usize>], planar: bool) -> Vec<Option<Patch>> {
    let mut sizes = vec![0usize; branches.len()];
    for &branch in branches.iter().flatten() {
        sizes[branch] += 1;
    }
    let mut largest_first: Vec<(usize, usize)> = (sizes.iter().copied().enumerate())
        .filter(|&(_, size)| size > 0)
        .collect();
    largest_first.sort_unstable_by_key(|&(branch, size)| (std::cmp::Reverse(size), branch));
    let mut patches = vec![None; branches.len()];
    let whole = Patch {
        u: 0.0,
        v: 0.0,
        width: 1.0,
        height: 1.0,
    };
    if !largest_first.is_empty() {
        split_into_patches(&largest_first, whole, planar, &mut patches);
    }
    patches
}

/// A treemap: halves the branches by size and the patch in proportion, across the longer side.
fn split_into_patches(
    branches: &[(usize, usize)],
    patch: Patch,
    planar: bool,
    patches: &mut [Option<Patch>],
) {
    if let [(branch, _)] = branches {
        patches[*branch] = Some(patch);
        return;
    }
    let total: usize = branches.iter().map(|&(_, size)| size).sum();
    let (mut cut, mut first_size) = (1, branches[0].1);
    while cut < branches.len() - 1 && (first_size + branches[cut].1) * 2 <= total {
        first_size += branches[cut].1;
        cut += 1;
    }
    let share = first_size as f32 / total as f32;
    let Patch {
        u,
        v,
        width,
        height,
    } = patch;
    // The longitude spans 2 pi and the height 2, so the sides compare in those units.
    let (first, second) = match planar || width * std::f32::consts::TAU >= height * 2.0 {
        true => (
            Patch {
                width: width * share,
                ..patch
            },
            Patch {
                u: u + width * share,
                width: width * (1.0 - share),
                ..patch
            },
        ),
        false => (
            Patch {
                height: height * share,
                ..patch
            },
            Patch {
                v: v + height * share,
                height: height * (1.0 - share),
                ..patch
            },
        ),
    };
    split_into_patches(&branches[..cut], first, planar, patches);
    split_into_patches(&branches[cut..], second, planar, patches);
}

/// A connection's lead: how far the connection may stretch, as a multiple of the shared reach,
/// and how hard the connection pulls before then. See [`edge_reach`].
///
/// Radial, a connection hangs loose within a lead spanning the patch of the bigger end's
/// `descendants`, scaled by `lead_scale`: the 2D ring gap, or the pair's apparent size on the 3D
/// sky. Connections across branches stretch freely, as one-way connections do outside a branch;
/// inside a branch, a one-way lead is [`ONE_WAY_LEAD`] times longer.
pub(super) fn edge_lead(
    one_way: bool,
    crosses_branches: bool,
    radius: f32,
    descendants: u32,
    hub_repulsion: f32,
    radial: bool,
    lead_scale: f32,
) -> (f32, f32) {
    let patch = ((1 + descendants) as f32 / PATCH_WORLDS_PER_LEAD).sqrt() * lead_scale;
    match (radial, crosses_branches, one_way) {
        (true, true, _) => (f32::INFINITY, 0.0),
        (true, false, false) => (patch, 0.0),
        (true, false, true) => (patch * ONE_WAY_LEAD, 0.0),
        (false, _, false) => (edge_reach(radius, hub_repulsion), 1.0),
        (false, _, true) => (f32::INFINITY, 1.0),
    }
}

/// How far one connection may stretch, as a multiple of the shared reach: a leaf's exactly the
/// reach, a hub's more. Read off the drawn radius, as [`node_mass`] is. See [`HUB_REACH`].
fn edge_reach(radius: f32, hub_repulsion: f32) -> f32 {
    1.0 + HUB_REACH * hub_repulsion * (radius / NODE_LEAF_RADIUS - 1.0)
}

/// How hard a world of `radius` pushes neighbours away, `hub_repulsion` mixing between one mass
/// for all and the mass of a solid ball of that radius.
///
/// The cube is for spacing: masses set a distance of `sqrt(m1 * m2)`, so radius to the power 1.5,
/// which opens hubs out past the width of their pictures.
pub(super) fn node_mass(radius: f32, hub_repulsion: f32) -> f32 {
    NODE_BASE_MASS * (1.0 + hub_repulsion * ((radius / NODE_LEAF_RADIUS).powi(3) - 1.0))
}
/// A simulation position -- pixel-ish, y down, sharing the origin -- in world space, `distance`
/// times farther from the origin. See [`sky_distance`].
pub(super) fn world_pos([x, y, z]: [f32; 3], distance: f32) -> Vec3 {
    vec3(x, -y, z) * (SIM_TO_WORLD * distance)
}

/// Inverse of [`world_pos`], for turning a point picked on screen back into a simulation target.
pub(super) fn sim_pos(world: Vec3, distance: f32) -> Vec3 {
    let sim = world / (SIM_TO_WORLD * distance);
    vec3(sim.x, -sim.y, sim.z)
}
