//! What the force simulation is asked for, and the conversion between its units and the
//! rendering space. See [`Layout`] and [`Layout::parameters`].

use super::*;

/// Side of the cube nodes are scattered over at the start of a layout, in simulation units.
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
/// The size of a world with [`NODE_HUB_DESCENDANTS`] worlds behind it. With [`NODE_LEAF_RADIUS`]
/// the two anchors the whole scale is drawn through, a size being the point between them a world's
/// descendant count reaches on a logarithmic curve.
///
/// Not a cap: the curve passes through this anchor rather than stopping at it, so the origin, with
/// the whole game behind it, is larger still.
pub(super) const NODE_HUB_RADIUS: f32 = 1.7;
/// Mass of a world of [`NODE_LEAF_RADIUS`]. [`node_mass`] scales every bigger world up from it.
const NODE_BASE_MASS: f32 = 21.0;
/// How much of a world's size settles how hard it pushes, until the panel's knob is dragged. At
/// `0.0` every world pushes alike; at `1.0` the push is proportional to the size, which spaces the
/// hubs generously and the rest of the layout with them.
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
/// every connection has and of the hub push. See [`edge_reach`] and [`LINK_REACH_DEFAULT`].
///
/// A hub seats a crowd of connections where a leaf seats one or two, and one ceiling for both
/// packs that crowd into the sphere a single connection gets: the children end up shoulder to
/// shoulder, pulling on the hub from every side at once.
///
/// Half of the size difference rather than all of it, the sizes spanning more than threefold: a
/// ceiling following them the whole way would put the origin's children as far out as it is meant
/// to stop them going. The hub push scales what is left, because turning that knob up against a
/// fixed ceiling only presses the crowd into it harder.
const HUB_REACH: f32 = 0.5;
/// Spacing between depth layers in layered mode, in simulation units.
///
/// Measured against how wide a layer actually settles rather than picked for looks: the busiest
/// hold a few hundred worlds and spread to a radius near 7000, and anything much smaller stacks
/// sixteen layers into one thickened cloud.
const DAG_LEVEL_DISTANCE: f32 = 1300.0;
/// Wider than [`DAG_LEVEL_DISTANCE`] because a flat layer is a line rather than a plane: it
/// carries the whole crowd on one line and may stack either side of it, so the gap has to hold
/// that stack and still read as a gap.
///
/// Not much wider than it has to be: the gaps multiply over sixteen layers, and a tree taller than
/// it is wide takes scrolling to read rather than one glance.
const DAG_LEVEL_DISTANCE_2D: f32 = 1900.0;
/// How far a world may sit from its layer per microstep of slack, in simulation units.
///
/// Two worlds across: a step of one leaves the worlds it separates touching, which reads as one
/// clump rather than a stack. Read off the node sizes rather than written as a number, because
/// those have been retuned more than once and a fixed microstep silently stopped being a step.
const DAG_LEVEL_MICROSTEP: f32 = 4.0 * NODE_LEAF_RADIUS / SIM_TO_WORLD;
/// How many [`DAG_LEVEL_MICROSTEP`]s of slack a flat layer has -- a count, not a distance.
///
/// A layer in three dimensions is a plane and spreads a crowd across it; in two it is a line, with
/// nowhere for the overflow but on top of itself. The band has to stay well under
/// [`DAG_LEVEL_DISTANCE_2D`] or the layers meet and stop reading as layers.
const DAG_LEVEL_SLACK_MICROSTEPS_2D: f32 = 3.0;
/// How far a connection may stretch before its spring stiffens, as a multiple of the spacing
/// between layers, until the panel's knob is dragged. See
/// [`SimulationParameters::link_distance_max`].
///
/// Without a ceiling the length of a connection is decided by how crowded the graph is rather than
/// by the connection: two worlds a single door apart could settle most of a layer away from each
/// other and read as unrelated. Written against the layer spacing because that is the distance the
/// eye already measures against.
const LINK_REACH_DEFAULT: f32 = 2.0;
// The layout choices. See [`Layout`].
const DIMENSIONS: &str = "dimensions";
const LAYERED: &str = "layered";
const HUB_REPULSION: &str = "hub-push";
const LINK_REACH: &str = "link-reach";

/// How far the hub push can be taken either way. Its middle is [`HUB_REPULSION_DEFAULT`].
pub(super) const HUB_REPULSION_RANGE: std::ops::RangeInclusive<f32> = 0.5..=1.5;

/// How far the link reach can be taken either way, in layers. See [`LINK_REACH_DEFAULT`].
///
/// The low end is under one layer, which pulls a connected pair together hard enough to read as
/// one clump. The high end is past the radius the busiest layer spreads to, which is the same as
/// no ceiling at all.
pub(super) const LINK_REACH_RANGE: std::ops::RangeInclusive<f32> = 0.5..=6.0;
/// The simulation parameters a [`Layout`] decides. Named rather than a tuple because they are four
/// numbers of three kinds, and a caller reading them out positionally has nothing to check itself
/// against.
pub(super) struct Solve {
    pub(super) dag_level_distance: Option<f32>,
    pub(super) dag_level_slack: f32,
    pub(super) force_charge: f32,
    pub(super) link_distance_max: Option<f32>,
}

/// How the person has asked for the graph to be laid out.
///
/// Together because they are one decision in several parts: set from one place, read from one
/// place, and handed to every graph built at once -- a rebuild carrying only some of them would
/// put the person somewhere they never asked to be. See [`Before`], which is how a rebuild picks
/// them up, and [`Layout::remembered`], which is how a run does.
#[derive(Clone, Copy, PartialEq)]
pub(super) struct Layout {
    /// Which of the two the layout is solved in.
    pub(super) dimensions: Dimensions,
    /// Whether the worlds are pinned to layers by how deep they sit.
    pub(super) layered: bool,
    /// How much of a world's size settles how hard it pushes. See [`node_mass`].
    pub(super) hub_repulsion: f32,
    /// How far a connection may stretch, in layers. See [`LINK_REACH_DEFAULT`].
    pub(super) link_reach: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            dimensions: Dimensions::Three,
            layered: true,
            hub_repulsion: HUB_REPULSION_DEFAULT,
            link_reach: LINK_REACH_DEFAULT,
        }
    }
}

impl Layout {
    /// Each part read on its own, so a store written by a version that knew fewer of them still
    /// gives up the ones it has. A value outside the range this version offers is brought inside
    /// it rather than thrown away, the same way the UI scale is.
    pub(super) fn remembered() -> Self {
        let fallback = Self::default();
        Self {
            dimensions: match store::read(DIMENSIONS).as_deref() {
                Some("2") => Dimensions::Two,
                Some("3") => Dimensions::Three,
                _ => fallback.dimensions,
            },
            layered: store::read(LAYERED)
                .and_then(|layered| layered.parse().ok())
                .unwrap_or(fallback.layered),
            hub_repulsion: store::read(HUB_REPULSION)
                .and_then(|push| push.parse().ok())
                .map_or(fallback.hub_repulsion, |push: f32| {
                    push.clamp(*HUB_REPULSION_RANGE.start(), *HUB_REPULSION_RANGE.end())
                }),
            link_reach: store::read(LINK_REACH)
                .and_then(|reach| reach.parse().ok())
                .map_or(fallback.link_reach, |reach: f32| {
                    reach.clamp(*LINK_REACH_RANGE.start(), *LINK_REACH_RANGE.end())
                }),
        }
    }

    pub(super) fn remember(&self) {
        store::write(
            DIMENSIONS,
            Some(match self.dimensions {
                Dimensions::Two => "2",
                Dimensions::Three => "3",
            }),
        );
        store::write(LAYERED, Some(&self.layered.to_string()));
        store::write(HUB_REPULSION, Some(&self.hub_repulsion.to_string()));
        store::write(LINK_REACH, Some(&self.link_reach.to_string()));
    }

    /// Two dimensions is not the same layout flattened. With every world on one plane there is
    /// nowhere for a crowd to go but sideways, so the layers close up and the push is turned far
    /// higher for the graph to read as anything at all. A connection's reach follows the same
    /// spacing and is turned up with everything else.
    pub(super) fn parameters(&self) -> Solve {
        let (spacing, slack, charge) = match self.dimensions {
            Dimensions::Two => (
                DAG_LEVEL_DISTANCE_2D,
                DAG_LEVEL_MICROSTEP * DAG_LEVEL_SLACK_MICROSTEPS_2D,
                FORCE_CHARGE_2D,
            ),
            Dimensions::Three => (DAG_LEVEL_DISTANCE, 0.0, FORCE_CHARGE),
        };
        Solve {
            dag_level_distance: self.layered.then_some(spacing),
            dag_level_slack: slack,
            force_charge: charge,
            link_distance_max: Some(spacing * self.link_reach),
        }
    }
}
/// Naive pseudo-RNG.
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
/// Scatters every node over the spawn volume and stops it dead, restarting the layout.
///
/// Dropping a dimension deforms the layout it was solved in and picking one back up cannot undo
/// that: a planar graph gives every pair the same z, so the repulsion along that axis is exactly
/// zero and the graph stays flat forever. Both switches therefore start over.
pub(super) fn scatter(data: &mut AppEntities) {
    let planar = data.graph.parameters().dimensions == Dimensions::Two;
    let rng = &mut data.rng;
    data.graph.visit_nodes_mut(|mut node| {
        node.set_position([
            rng.next_f32() * SPAWN_EXTENT,
            rng.next_f32() * SPAWN_EXTENT,
            if planar {
                0.0
            } else {
                rng.next_f32() * SPAWN_EXTENT
            },
        ]);
        node.set_velocity([0.0; 3]);
    });
}
/// How far one connection may stretch, as a multiple of what every connection is allowed.
///
/// A one-way connection is the long way round -- a chute, a warp, a door that does not open back
/// -- so the two worlds it joins are a step apart to walk and nowhere near each other on the map,
/// and it is left to stretch as far as the rest of the layout wants. Every other connection is
/// held: a leaf's to the reach itself, a hub's to more of it. See [`HUB_REACH`].
///
/// Read off the drawn size for the same reason [`node_mass`] is: a second measure of how much
/// hangs off a world would be one more place for the two to disagree.
pub(super) fn edge_reach(one_way: bool, radius: f32, hub_repulsion: f32) -> f32 {
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
/// The cube is chosen for the room it makes rather than for the physics -- nothing here integrates
/// a mass as inertia. What the mass settles is a distance: a pair's repulsion is capped, so the
/// pair pushes at that ceiling out to where the falloff drops it below, which works out as the
/// square root of the product of the two masses. Cubed, that is the radii to the power of one and
/// a half, which opens the hubs out well past the width of the pictures on them. A mass
/// proportional to the radius instead would hold each pair at about the sum of its radii, the
/// least that keeps the pictures apart.
pub(super) fn node_mass(radius: f32, hub_repulsion: f32) -> f32 {
    NODE_BASE_MASS * (1.0 + hub_repulsion * ((radius / NODE_LEAF_RADIUS).powi(3) - 1.0))
}
/// A simulation position -- pixel-ish, origin at a corner of the initial cube -- in world space.
pub(super) fn world_pos([x, y, z]: [f32; 3]) -> Vec3 {
    let center = SPAWN_EXTENT * 0.5;
    vec3(x - center, center - y, z - center) * SIM_TO_WORLD
}

/// Inverse of [`world_pos`], for turning a point picked on screen back into a simulation target.
pub(super) fn sim_pos(world: Vec3) -> Vec3 {
    let center = SPAWN_EXTENT * 0.5;
    let world = world / SIM_TO_WORLD;
    vec3(world.x + center, center - world.y, world.z + center)
}
