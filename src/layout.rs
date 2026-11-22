//! What the force simulation is given, and the conversion between its units and the
//! rendering space. See [`Layout`] and [`Layout::parameters`].

use super::*;

/// Side of the cube, centred on the origin, that nodes are scattered over at the start of a
/// layout, in simulation units.
const SPAWN_EXTENT: f32 = 1000.0;
/// World units per simulation unit: [`world_pos`] multiplies by it, [`sim_pos`] divides.
///
/// Simulation coordinates are pixel-sized, so a graph a thousand of them across has to stand in
/// front of a camera that measures in tens.
///
/// "World" here is the rendering space, not one of the game's worlds -- which is what the name
/// means everywhere else in this file.
const SIM_TO_WORLD: f32 = 0.012;
/// The smallest node the graph draws, and the radius every other size and mass is read against.
pub(super) const NODE_LEAF_RADIUS: f32 = 1.1;
/// The size of a world with [`NODE_HUB_DESCENDANTS`] worlds behind it, and with
/// [`NODE_LEAF_RADIUS`] one of the two anchors a logarithmic curve is drawn through.
///
/// Not a cap: the curve passes through this anchor rather than stopping at it, so the origin, with
/// the whole game behind it, is larger still.
pub(super) const NODE_HUB_RADIUS: f32 = 1.7;
/// Mass of a world of [`NODE_LEAF_RADIUS`]. [`node_mass`] scales every bigger world up from it.
const NODE_BASE_MASS: f32 = 21.0;
/// How much of a world's size settles how hard it pushes, until the panel's knob is dragged. At
/// `0.0` every world pushes alike, at `1.0` the push follows the size.
const HUB_REPULSION_DEFAULT: f32 = 0.83;

const FORCE_CHARGE: f32 = 1800.0;
const FORCE_CHARGE_2D: f32 = 10000.0;
pub(super) const SETTLE_AFTER: f32 = 32.0;

/// Where on the descendant counts [`NODE_HUB_RADIUS`] sits, and so how quickly the sizes climb.
///
/// The two radii are near each other on purpose: a wider range is more than the layout can hold
/// without the hubs eating their neighbours.
pub(super) const NODE_HUB_DESCENDANTS: f32 = 4.0;
/// How much of a world's size goes into how far its connections may stretch, on top of the reach
/// every connection has and of the hub push. See [`edge_reach`] and
/// [`Arrangement::default_link_reach`].
///
/// A hub seats a crowd of connections where a leaf seats one or two, so one ceiling for both packs
/// that crowd into the sphere a single connection gets.
///
/// Half of the size difference rather than all of it, the sizes spanning more than threefold: a
/// ceiling following them the whole way would put the origin's children as far out as it is meant
/// to stop them going. The hub push scales what is left, a fixed ceiling only being pressed into
/// harder as that knob goes up.
const HUB_REACH: f32 = 0.5;
/// The 3D unit of length outside the tree, in simulation units: what a connection's lead and
/// the push's reach are measured in. See [`SKY_SHELLS`].
const DAG_LEVEL_DISTANCE: f32 = 4000.0;
/// Distance between depth rings in 2D layered mode, in simulation units.
const DAG_LEVEL_DISTANCE_2D: f32 = 6000.0;
/// Distance between the [`Arrangement::Tree`]'s depth planes, in simulation units.
const TREE_LEVEL_DISTANCE: f32 = 1300.0;
/// Distance between the [`Arrangement::FlatTree`]'s depth rows, in simulation units.
const TREE_LEVEL_DISTANCE_2D: f32 = 1900.0;
/// How far the sky every world but the Nexus sits on in the orbital arrangement is from the
/// Nexus, in [`DAG_LEVEL_DISTANCE`]s. Sized so the worlds have about as much room each as in the busy rings.
const SKY_SHELLS: f32 = 3.0;
/// How many times farther each depth past the first is drawn than the one before it on the 3D
/// sky, and so how much smaller it looks from the Nexus. See [`sky_distance`].
const SKY_DISTANCE_STEP: f32 = 1.1;
/// How many rings or shells out from the Nexus keep the whole spacing between them: the
/// shared core, which holds most of the worlds and most of the connections between branches.
const CORE_SHELLS: f32 = 4.0;
/// The gap between shells past [`CORE_SHELLS`], as a fraction of the spacing. The
/// deep worlds are few and mostly in chains, which need less room.
const OUTER_SHELL_GAP: f32 = 0.5;
/// How far a world may sit from its shell per microstep of slack, in simulation units.
///
/// Two worlds across: a step of one leaves the worlds it separates touching. Read off the node
/// sizes rather than written as a number, which those have outgrown more than once.
const DAG_LEVEL_MICROSTEP: f32 = 4.0 * NODE_LEAF_RADIUS / SIM_TO_WORLD;
/// How many [`DAG_LEVEL_MICROSTEP`]s of slack a flat layer has -- a count, not a distance.
///
/// A layer in two dimensions is a line, with nowhere for the overflow but on top of itself. The
/// band has to stay well under [`DAG_LEVEL_DISTANCE_2D`] or the layers meet.
const DAG_LEVEL_SLACK_MICROSTEPS_2D: f32 = 3.0;
/// How far a world's push reaches when radial, as a multiple of the shell spacing.
///
/// Short, so a connection with lead left is not pushed taut: the branch patches spread the worlds,
/// and the push only keeps neighbours apart.
const REPULSION_RANGE_SHELLS: f32 = 0.25;
const ARRANGEMENT: &str = "arrangement";
/// Read only, for a store written before [`ARRANGEMENT`].
const DIMENSIONS: &str = "dimensions";
/// Read only, for a store written before [`ARRANGEMENT`].
const LAYERED: &str = "layered";
const HUB_REPULSION: &str = "hub-push";
/// Followed by the arrangement's name: each keeps its own.
const LINK_REACH: &str = "link-reach-";

/// How far the hub push can be taken either way. Its middle is [`HUB_REPULSION_DEFAULT`].
pub(super) const HUB_REPULSION_RANGE: std::ops::RangeInclusive<f32> = 0.5..=1.5;

/// How far the link reach can be taken either way, in layers. See
/// [`Arrangement::default_link_reach`].
///
/// The low end is under one layer, which pulls a connected pair into one clump; the high end is
/// past the radius the busiest layer spreads to, which is no ceiling at all.
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
    /// On one sky around the Nexus, each drawn farther out the deeper it is. See [`sky_distance`].
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

    /// Whether each world is held to a layer set by its depth.
    pub(super) fn layered(self) -> bool {
        match self {
            Self::Flat | Self::Free => false,
            Self::Radial | Self::FlatTree | Self::Tree | Self::Orbital => true,
        }
    }

    /// Whether each world is held at a distance from the Nexus by its depth, in its branch's patch
    /// and on loose leads. See [`edge_lead`].
    pub(super) fn radial(self) -> bool {
        match self {
            Self::Radial | Self::Orbital => true,
            Self::Flat | Self::FlatTree | Self::Free | Self::Tree => false,
        }
    }

    /// How much lead a connection has before it goes taut, as a multiple of the layer spacing,
    /// until the panel's knob is dragged. See [`SimulationParameters::link_distance_max`].
    ///
    /// Long in the tree, whose planes are close and whose connections run across them; short on
    /// the sky, where the branch patches and not the connections spread the worlds.
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

    /// From a store written before there was one setting: the two switches it had.
    fn from_switches(dimensions: Dimensions, layered: bool) -> Self {
        match (dimensions, layered) {
            (Dimensions::Two, false) => Self::Flat,
            (Dimensions::Two, true) => Self::Radial,
            (Dimensions::Three, false) => Self::Free,
            (Dimensions::Three, true) => Self::Orbital,
        }
    }
}

/// How the person has chosen to have the graph laid out.
///
/// Together because they are one decision in several parts, and a rebuild carrying only some of
/// them would put the person somewhere they never chose. See [`Before`] and
/// [`Layout::remembered`].
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Layout {
    pub(super) arrangement: Arrangement,
    /// How much of a world's size settles how hard it pushes. See [`node_mass`].
    pub(super) hub_repulsion: f32,
    /// How far a connection may stretch, in layers, in this arrangement. See
    /// [`Arrangement::default_link_reach`].
    pub(super) link_reach: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            arrangement: Arrangement::Orbital,
            hub_repulsion: HUB_REPULSION_DEFAULT,
            link_reach: Arrangement::Orbital.default_link_reach(),
        }
    }
}

impl Layout {
    /// Each part read on its own, so a store written by a version that knew fewer of them still
    /// gives up the ones it has.
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
            link_reach: arrangement.remembered_link_reach(),
        }
    }

    pub(super) fn remember(&self) {
        store::write(ARRANGEMENT, Some(self.arrangement.stored()));
        store::write(HUB_REPULSION, Some(&self.hub_repulsion.to_string()));
        let link_reach = format!("{LINK_REACH}{}", self.arrangement.stored());
        store::write(&link_reach, Some(&self.link_reach.to_string()));
    }

    /// Two dimensions is not the same layout flattened: a crowd on one plane can only go sideways,
    /// so the layers close up and the push is turned far higher. A connection's reach follows the
    /// same spacing.
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
/// Scatters every node and stops it dead, restarting the layout: into its branch's patch of its
/// shell when radial, over the spawn volume otherwise.
///
/// Both switches start over: a planar graph gives every pair the same z, so the repulsion along
/// that axis is exactly zero and picking the dimension back up leaves it flat forever.
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

/// Which layer a world at `depth` is held to, in layer spacings: on the sky when orbital, one
/// plane a depth in the tree, otherwise on rings or shells one apart through the core and closer
/// together past it.
pub(super) fn shell_level(depth: f32, arrangement: Arrangement) -> f32 {
    match (arrangement, depth > 0.0) {
        (Arrangement::Orbital, true) => SKY_SHELLS,
        (Arrangement::Orbital, false) => 0.0,
        (Arrangement::Tree | Arrangement::FlatTree, _) => depth,
        _ => depth.min(CORE_SHELLS) + (depth - CORE_SHELLS).max(0.0) * OUTER_SHELL_GAP,
    }
}

/// The gap between the shell at `depth` and the one inside it, in layer spacings: what a
/// connection's lead is scaled by. One throughout on the sky, where there are no shells.
pub(super) fn shell_gap(depth: f32, arrangement: Arrangement) -> f32 {
    match arrangement {
        Arrangement::Orbital => 1.0,
        _ => shell_level(depth, arrangement) - shell_level(depth - 1.0, arrangement),
    }
}

/// How many times farther than the sky a world at `depth` is drawn from the Nexus: its direction
/// comes from the layout on the sky, its distance from its depth, as a star's does.
pub(super) fn sky_distance(depth: f32, arrangement: Arrangement) -> f32 {
    match arrangement {
        Arrangement::Orbital => SKY_DISTANCE_STEP.powf((depth - 1.0).max(0.0)),
        _ => 1.0,
    }
}

/// Per world, the door out of the Nexus its route passes through, or `None` for the Nexus and for
/// worlds it has no route to.
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

/// Whether a connection joins two branches, or the Nexus or a world it has no route to.
pub(super) fn crosses_branches(branches: &[Option<usize>], from: usize, to: usize) -> bool {
    branches[from] != branches[to] || branches[from].is_none()
}

/// A rectangle of the sphere's equal-area map: `u` runs round the longitude and `v` up the
/// height, both from 0 to 1, so a patch's area is its share of the sphere.
#[derive(Clone, Copy)]
struct Patch {
    u: f32,
    v: f32,
    width: f32,
    height: f32,
}

/// Per world standing for a branch, the patch of the sphere its branch is laid out in, sized by how
/// many worlds the branch holds. Planar, every patch is a whole-height slice: an arc of the ring.
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

/// A treemap: halves the branches by size and the patch in proportion, across its longer side.
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

/// What one connection's lead is in a layout: how far it may stretch, as a multiple of what every
/// connection is allowed, and how hard it pulls before then. See [`edge_reach`].
///
/// Radial, a connection hangs loose until its lead runs out, and one between two branches has no
/// lead at all: the branch patches, not the connections, decide where a branch sits. The lead
/// is scaled by `lead_scale`: the 2D ring gap at the deeper end, or on the 3D sky how small the
/// pair looks from the Nexus.
pub(super) fn edge_lead(
    one_way: bool,
    crosses_branches: bool,
    radius: f32,
    hub_repulsion: f32,
    radial: bool,
    lead_scale: f32,
) -> (f32, f32) {
    match (radial, crosses_branches) {
        (true, true) => (f32::INFINITY, 0.0),
        (true, false) => (edge_reach(one_way, radius, hub_repulsion) * lead_scale, 0.0),
        (false, _) => (edge_reach(one_way, radius, hub_repulsion), 1.0),
    }
}

/// How far one connection may stretch, as a multiple of what every connection is allowed.
///
/// A one-way connection -- a chute, a warp, a door that does not open back -- joins two worlds a
/// step apart to walk and nowhere near each other on the map, so it is left to stretch. Every
/// other is held: a leaf's to the reach itself, a hub's to more of it. See [`HUB_REACH`].
///
/// Read off the drawn size for the same reason [`node_mass`] is: a second measure of how much
/// hangs off a world would be one more place for the two to disagree.
fn edge_reach(one_way: bool, radius: f32, hub_repulsion: f32) -> f32 {
    if one_way {
        return f32::INFINITY;
    }
    1.0 + HUB_REACH * hub_repulsion * (radius / NODE_LEAF_RADIUS - 1.0)
}

/// How hard a world of a given radius pushes its neighbours away, `hub_repulsion` mixing between
/// one mass for everything and the mass a solid ball of that radius would have.
///
/// At zero every world repels alike and the sizes are drawn on top of a layout spaced for the
/// smallest of them, so the hubs sit over their neighbours.
///
/// The cube is chosen for the room it makes rather than for the physics -- nothing integrates a
/// mass as inertia here. What a mass settles is a distance, the square root of the two masses'
/// product; cubed, that is the radii to the power of one and a half, which opens the hubs out well
/// past the width of the pictures on them, where a linear mass would barely clear them.
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
