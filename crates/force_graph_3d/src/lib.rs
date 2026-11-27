//! A 3D force-directed graph, derived from [`force_graph`](https://github.com/t-mw/force-graph-rs),
//! after [Graphoon](https://github.com/rm-code/Graphoon/).
//!
//! # Example
//!
//! ```
//! use force_graph_3d::{ForceGraph, NodeData};
//!
//! let mut graph = <ForceGraph>::new(Default::default());
//! let hub = graph.add_node(NodeData {
//!     x: 500.0,
//!     y: 500.0,
//!     z: 500.0,
//!     is_anchor: true,
//!     ..Default::default()
//! });
//! let spoke = graph.add_node(NodeData {
//!     x: 250.0,
//!     y: 250.0,
//!     z: 750.0,
//!     ..Default::default()
//! });
//! graph.add_edge(hub, spoke, Default::default());
//!
//! graph.update(0.03);
//! graph.visit_nodes(|node| println!("{:?}", node.position()));
//! ```

mod octree;

use octree::{Bodies, Interactions, Octree};
use petgraph::{
    stable_graph::{NodeIndex, StableUnGraph},
    visit::{EdgeRef, IntoEdgeReferences},
};

pub type DefaultNodeIdx = NodeIndex<petgraph::stable_graph::DefaultIx>;

/// Added to every squared distance so a coincident pair yields a zero force instead of a NaN: the
/// direction vector is zero there.
const SOFTENING: f32 = 1e-6;

/// Eight `f32` lanes fill an AVX2 register and are a whole multiple of the four wasm `simd128`,
/// SSE and NEON provide, so the repulsion loop vectorizes on all of them.
const LANES: usize = 8;

/// Seconds of continued disturbance needed to restore the full damping. A settled layout still
/// holds unbalanced forces, so a gradual restore keeps a brief touch from releasing them all.
const WAKE_TIME: f32 = 0.5;

/// Rate at which a constrained axis closes the gap to the target, per second: a fixed fraction of
/// the remaining distance each step, which converges without overshoot at any frame time.
const CONSTRAINT_RATE: f32 = 8.0;

/// The one timestep the simulation steps by, in seconds; [`ForceGraph::update`] banks elapsed
/// time and spends that time in these steps. [`SimulationParameters::damping_factor`] is per step
/// and force becomes displacement as `dt.powi(3)`, so a varying `dt` changes the trajectory.
///
/// The forces, damping and [`SimulationParameters::settle_speed`] are tuned to this step.
const FIXED_STEP: f32 = 1.0 / 60.0;

/// How many [`FIXED_STEP`]s one [`ForceGraph::update`] call may spend; time beyond is dropped, so
/// returning to a backgrounded tab does not stall a frame catching up.
const MAX_STEPS_PER_UPDATE: usize = 3;

/// Distance below which a constrained axis is set to the target exactly, since geometric decay
/// only approaches the target and [`Dimensions::Two`] must be exactly planar.
const CONSTRAINT_SNAP: f32 = 1e-2;

/// How much stiffer a spring gets past [`SimulationParameters::link_distance_max`], as a multiple
/// of the base rate: stiff enough to visibly hold an edge, soft enough for the fixed step.
const OVERSTRETCH_STIFFNESS: f32 = 20.0;

/// Which axes the layout may use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dimensions {
    /// Layout on the `z = 0` plane. Repulsion and springs still run in 3D, so switching at runtime
    /// collapses the graph onto the plane gradually.
    Two,
    #[default]
    Three,
}

/// The surface each depth layer is held to in layered mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layers {
    /// Planes across y, so y reads as depth.
    Planes,
    /// Shells around the origin, so distance from the origin reads as depth. By symmetry, inner
    /// shells push a node straight outward, a push the shell constraint discards whole.
    #[default]
    Shells,
}

#[derive(Clone, Debug)]
pub struct SimulationParameters {
    pub force_charge: f32,
    pub force_spring: f32,
    /// Ceiling per component on one repulsion interaction, so one close neighbour cannot dominate.
    /// The springs and [`ForceGraph::apply_force`] are unclamped.
    pub force_max: f32,
    pub node_speed: f32,
    pub damping_factor: f32,
    /// Barnes-Hut opening angle: a distant group is replaced by the center of mass once the group
    /// width subtends less than this, making repulsion O(n log n). `0.0` sums every pair exactly.
    pub theta: f32,
    /// Whether the layout may use the z axis. Like [`Self::dag_level_distance`], free to change
    /// between any two steps: both constrain positions after the forces run.
    pub dimensions: Dimensions,
    /// Speed below which the layout counts as settled, in position units per second.
    ///
    /// The default is a tenth of a position unit per frame at 60 Hz. `0.0` settles only a layout
    /// that is exactly at rest.
    pub settle_speed: f32,
    /// Simulated seconds over which the layout is brought to a standstill, or `None` to wait on
    /// [`SimulationParameters::settle_speed`] however long that takes.
    ///
    /// [`SimulationParameters::damping_factor`] is ramped to zero across this window, so a sparse
    /// graph whose repulsion outweighs the springs, which never converges, still comes to rest. A
    /// disturbance restores the damping over [`WAKE_TIME`].
    pub settle_after: Option<f32>,
    /// Layered ("DAG") mode: spacing between the [`Layers`], or `None` for free placement. Each
    /// node is held at [`NodeData::level`] times this, along y or from the origin.
    pub dag_level_distance: Option<f32>,
    /// Layered mode: what each layer is.
    pub dag_layers: Layers,
    /// Layered mode: how far a node may sit off the layer, or `0.0` to pin the node there. Only
    /// read with [`SimulationParameters::dag_level_distance`] set, and best kept well under that.
    ///
    /// Inside the band the radius is free, so a crowded [`Dimensions::Two`] ring stacks across the
    /// band. The edge is soft, decaying at [`CONSTRAINT_RATE`].
    pub dag_level_slack: f32,
    /// How far an edge may stretch, or `None` for linear springs at any length.
    ///
    /// The summed repulsion grows with node count while one edge's pull does not, so a linear
    /// spring stretches with the graph. Past this distance the spring gains a second term,
    /// [`OVERSTRETCH_STIFFNESS`] times stiffer, proportional to the excess. The ceiling is soft.
    pub link_distance_max: Option<f32>,
    /// How far a node's repulsion reaches, or `None` for every distance. Past this distance nothing
    /// pushes, so slack edges can stay slack.
    pub repulsion_range: Option<f32>,
}

impl Default for SimulationParameters {
    fn default() -> Self {
        SimulationParameters {
            force_charge: 12000.0,
            force_spring: 0.3,
            force_max: 280.0,
            node_speed: 7000.0,
            damping_factor: 0.95,
            theta: 0.7,
            settle_speed: 5.0,
            settle_after: Some(5.0),
            dimensions: Dimensions::default(),
            dag_level_distance: None,
            dag_layers: Layers::default(),
            dag_level_slack: 0.0,
            link_distance_max: None,
            repulsion_range: None,
        }
    }
}

pub struct NodeData<UserNodeData = ()> {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Which depth shell the node belongs to, as a count the caller defines. Read only while
    /// [`SimulationParameters::dag_level_distance`] is set, which scales the level to a distance.
    pub level: f32,
    /// A heavier node repels neighbours harder.
    pub mass: f32,
    pub is_anchor: bool,
    pub user_data: UserNodeData,
}

impl<UserNodeData> Default for NodeData<UserNodeData>
where
    UserNodeData: Default,
{
    fn default() -> Self {
        NodeData {
            x: 0.0,
            y: 0.0,
            z: 0.0,
            level: 0.0,
            mass: 10.0,
            is_anchor: false,
            user_data: Default::default(),
        }
    }
}

pub struct EdgeData<UserEdgeData = ()> {
    /// This edge's own ceiling, as a multiple of [`SimulationParameters::link_distance_max`];
    /// `f32::INFINITY` exempts the edge, leaving a plain linear spring.
    pub reach: f32,
    /// Pull within the ceiling, as a multiple of [`SimulationParameters::force_spring`]. `0.0`
    /// means slack: past the ceiling the overstretch term still holds the edge.
    pub stiffness: f32,
    pub user_data: UserEdgeData,
}

impl<UserEdgeData> Default for EdgeData<UserEdgeData>
where
    UserEdgeData: Default,
{
    fn default() -> Self {
        EdgeData {
            reach: 1.0,
            stiffness: 1.0,
            user_data: Default::default(),
        }
    }
}

/// Node state, one array per component, indexed by [`DefaultNodeIdx::index`].
///
/// A slot outlives the node until [`StableUnGraph`] reuses the index. A free slot has `mass == 0.0`
/// and `mobility == 0.0`, exerting no force and receiving no motion, so the hot loops scan every
/// slot without a liveness test.
#[derive(Default)]
struct NodeStore {
    x: Vec<f32>,
    y: Vec<f32>,
    z: Vec<f32>,
    vx: Vec<f32>,
    vy: Vec<f32>,
    vz: Vec<f32>,
    ax: Vec<f32>,
    ay: Vec<f32>,
    az: Vec<f32>,
    mass: Vec<f32>,
    /// `1.0` for a node the simulation may move, `0.0` for an anchor or a free slot.
    mobility: Vec<f32>,
    /// Depth shell per node, as handed over in [`NodeData::level`]. Read only in layered mode.
    level: Vec<f32>,
}

impl NodeStore {
    fn len(&self) -> usize {
        self.x.len()
    }

    #[expect(clippy::too_many_arguments)]
    fn write(
        &mut self,
        slot: usize,
        x: f32,
        y: f32,
        z: f32,
        level: f32,
        mass: f32,
        is_anchor: bool,
    ) {
        if slot >= self.len() {
            let len = slot + 1;
            for array in self.arrays_mut() {
                array.resize(len, 0.0);
            }
        }
        self.x[slot] = x;
        self.y[slot] = y;
        self.z[slot] = z;
        self.vx[slot] = 0.0;
        self.vy[slot] = 0.0;
        self.vz[slot] = 0.0;
        self.ax[slot] = 0.0;
        self.ay[slot] = 0.0;
        self.az[slot] = 0.0;
        self.mass[slot] = mass;
        self.mobility[slot] = if is_anchor { 0.0 } else { 1.0 };
        self.level[slot] = level;
    }

    /// Marks `slot` free: no mass to repel with, no mobility to move with.
    fn release(&mut self, slot: usize) {
        if slot < self.len() {
            self.write(slot, 0.0, 0.0, 0.0, 0.0, 0.0, true);
        }
    }

    fn arrays_mut(&mut self) -> [&mut Vec<f32>; 12] {
        [
            &mut self.x,
            &mut self.y,
            &mut self.z,
            &mut self.vx,
            &mut self.vy,
            &mut self.vz,
            &mut self.ax,
            &mut self.ay,
            &mut self.az,
            &mut self.mass,
            &mut self.mobility,
            &mut self.level,
        ]
    }

    fn clear(&mut self) {
        for array in self.arrays_mut() {
            array.clear();
        }
    }

    fn add_force(&mut self, slot: usize, [fx, fy, fz]: [f32; 3]) {
        self.ax[slot] += fx;
        self.ay[slot] += fy;
        self.az[slot] += fz;
    }

    /// So a settled layout resumes from rest, without the residual drift.
    fn halt(&mut self) {
        self.vx.fill(0.0);
        self.vy.fill(0.0);
        self.vz.fill(0.0);
    }

    fn fastest_speed_sqrd(&self) -> f32 {
        let n = self.len();
        let (vx, vy, vz) = (&self.vx[..n], &self.vy[..n], &self.vz[..n]);
        let mut fastest = 0.0f32;
        for i in 0..n {
            fastest = fastest.max(vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
        }
        fastest
    }

    fn clear_forces(&mut self) {
        self.ax.fill(0.0);
        self.ay.fill(0.0);
        self.az.fill(0.0);
    }
}

/// Whether the layout is at rest, and how much of the damping is in play. One value, so a method
/// that disturbs the layout wakes the whole value without knowing how damping is managed.
struct Rest {
    settled: bool,
    /// Fraction of [`SimulationParameters::damping_factor`] in effect: withdrawn across the
    /// settling window while the layout is left alone, restored while the layout is disturbed.
    liveliness: f32,
    disturbed_since_step: bool,
}

impl Default for Rest {
    fn default() -> Self {
        Rest {
            settled: false,
            liveliness: 1.0,
            disturbed_since_step: false,
        }
    }
}

impl Rest {
    fn wake(&mut self) {
        self.settled = false;
        self.disturbed_since_step = true;
    }

    /// Hands the damping back at once and starts the settling window over.
    fn revive(&mut self) {
        *self = Rest::default();
    }

    /// Fraction of the damping now in effect: rises while the layout is disturbed and falls
    /// otherwise, so a brief touch gives a brief response.
    fn update(&mut self, dt: f32, settle_after: Option<f32>) -> f32 {
        let Some(window) = settle_after else {
            self.liveliness = 1.0;
            return 1.0;
        };
        let step = if std::mem::take(&mut self.disturbed_since_step) {
            dt / WAKE_TIME
        } else {
            -dt / window.max(f32::MIN_POSITIVE)
        };
        self.liveliness = (self.liveliness + step).clamp(0.0, 1.0);
        // Cubed, so damping holds near full and withdraws only at the end.
        let withdrawn = 1.0 - self.liveliness;
        1.0 - withdrawn * withdrawn * withdrawn
    }
}

pub struct ForceGraph<UserNodeData = (), UserEdgeData = ()> {
    parameters: SimulationParameters,
    /// Woken by every path that can disturb the layout, hence the accessor
    /// [`ForceGraph::parameters`].
    rest: Rest,
    graph: StableUnGraph<UserNodeData, EdgeData<UserEdgeData>>,
    nodes: NodeStore,
    /// Rebuilt every step by the approximate pass; kept to reuse the allocations.
    tree: Octree,
    interactions: Interactions,
    /// `force_max` per node, the uniform interaction limit the exact pass hands the kernel.
    limits: Vec<f32>,
    /// Target coordinate per node for the axis a constraint is closing on; kept to reuse the
    /// allocation.
    targets: Vec<f32>,
    unspent_time: f32,
}

impl<UserNodeData, UserEdgeData> ForceGraph<UserNodeData, UserEdgeData> {
    /// With default parameters:
    /// ```
    /// use force_graph_3d::ForceGraph;
    /// let graph = <ForceGraph>::new(Default::default());
    /// ```
    pub fn new(parameters: SimulationParameters) -> Self {
        ForceGraph {
            parameters,
            rest: Rest::default(),
            graph: StableUnGraph::default(),
            nodes: NodeStore::default(),
            tree: Octree::default(),
            interactions: Interactions::default(),
            limits: Vec::new(),
            targets: Vec::new(),
            unspent_time: 0.0,
        }
    }

    pub fn parameters(&self) -> &SimulationParameters {
        &self.parameters
    }

    /// Borrows the parameters for modification, and wakes the layout: a parameter change
    /// invalidates the rest, and the graph has no other way to notice.
    pub fn parameters_mut(&mut self) -> &mut SimulationParameters {
        self.rest.wake();
        &mut self.parameters
    }

    /// Starts the settling window over, as though the layout had just been built. For a caller that
    /// rearranged the layout wholesale, which the gradual wake over [`WAKE_TIME`] is too short for.
    pub fn revive(&mut self) {
        self.rest.revive();
        self.unspent_time = 0.0;
    }

    /// Whether the layout has come to rest, leaving [`ForceGraph::update`] with nothing to do.
    ///
    /// Cleared by anything that disturbs the layout: a force, a moved or altered node, a new or
    /// removed node or edge, and a parameter change.
    pub fn is_settled(&self) -> bool {
        self.rest.settled
    }

    /// The topology and the user data. Node positions live in the arrays behind
    /// [`ForceGraph::visit_nodes`].
    pub fn get_graph(&self) -> &StableUnGraph<UserNodeData, EdgeData<UserEdgeData>> {
        &self.graph
    }

    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    pub fn add_node(&mut self, node_data: NodeData<UserNodeData>) -> DefaultNodeIdx {
        let NodeData {
            x,
            y,
            z,
            level,
            mass,
            is_anchor,
            user_data,
        } = node_data;
        let idx = self.graph.add_node(user_data);
        self.nodes
            .write(idx.index(), x, y, z, level, mass, is_anchor);
        self.rest.wake();
        idx
    }

    pub fn remove_node(&mut self, idx: DefaultNodeIdx) {
        if self.graph.remove_node(idx).is_some() {
            self.nodes.release(idx.index());
            self.rest.wake();
        }
    }

    pub fn add_edge(
        &mut self,
        n1_idx: DefaultNodeIdx,
        n2_idx: DefaultNodeIdx,
        edge: EdgeData<UserEdgeData>,
    ) {
        self.graph.update_edge(n1_idx, n2_idx, edge);
        self.rest.wake();
    }

    pub fn clear(&mut self) {
        self.graph.clear();
        self.nodes.clear();
        self.rest.wake();
    }

    /// Steps the simulation over `dt` seconds elapsed since the previous update.
    ///
    /// `dt` decides how many [`FIXED_STEP`]s this call takes, up to [`MAX_STEPS_PER_UPDATE`]; a
    /// settled layout takes none. Returns whether any node moved, so a caller can skip rebuilding
    /// geometry: false over a settled layout or for a frame that completes no step.
    pub fn update(&mut self, dt: f32) -> bool {
        if self.rest.settled || self.graph.node_count() == 0 {
            return false;
        }
        self.unspent_time = (self.unspent_time + dt).min(FIXED_STEP * MAX_STEPS_PER_UPDATE as f32);
        let mut stepped = false;
        while self.unspent_time >= FIXED_STEP {
            self.unspent_time -= FIXED_STEP;
            self.fixed_step();
            stepped = true;
            if self.rest.settled {
                // Settled: the next wake starts without a leftover part-step.
                self.unspent_time = 0.0;
                break;
            }
        }
        stepped
    }

    fn fixed_step(&mut self) {
        let dt = FIXED_STEP;
        let damping =
            self.parameters.damping_factor * self.rest.update(dt, self.parameters.settle_after);
        self.repel();
        self.attract();
        // A constrained direction is driven by `constrain` alone, so 2D stays exactly planar. A
        // slack layer's constraint is a band, which the forces fill.
        let pinned =
            self.parameters.dag_level_distance.is_some() && self.parameters.dag_level_slack <= 0.0;
        let planes = self.parameters.dag_layers == Layers::Planes;
        if pinned && !planes {
            self.discard_radial_force();
        }
        let free = [
            true,
            !(pinned && planes),
            self.parameters.dimensions == Dimensions::Three,
        ];
        self.integrate(dt, damping, free);
        let constrained = self.constrain(dt);
        // Cleared at the end, so a force applied between steps reaches the next step.
        self.nodes.clear_forces();

        // A constraint moves a node without velocity, so a collapsing graph must stay awake.
        let settle_speed = self.parameters.settle_speed;
        self.rest.settled =
            constrained && self.nodes.fastest_speed_sqrd() <= settle_speed * settle_speed;
        if self.rest.settled {
            self.nodes.halt();
        }
    }

    /// Adds a force to one node for the next [`ForceGraph::update`]; repeated calls accumulate.
    /// Unclamped by [`SimulationParameters::force_max`], and ignored by an anchored node.
    pub fn apply_force(&mut self, idx: DefaultNodeIdx, force: [f32; 3]) {
        if force == [0.0; 3] {
            return;
        }
        if self.graph.contains_node(idx) {
            self.nodes.add_force(idx.index(), force);
            self.rest.wake();
        }
    }

    /// Charge repulsion between the nodes: the pass that dominates the step.
    fn repel(&mut self) {
        if self.parameters.theta > 0.0 {
            self.repel_approx();
        } else {
            self.repel_exact();
        }
    }

    /// The squared [`SimulationParameters::repulsion_range`], infinite when unset.
    fn range_sq(&self) -> f32 {
        self.parameters
            .repulsion_range
            .map_or(f32::INFINITY, |range| range * range)
    }

    /// Every pair, summed exactly. O(n²), and the reference the approximate pass is tested
    /// against.
    fn repel_exact(&mut self) {
        let n = self.nodes.len();
        let force_max = self.parameters.force_max;
        let charge = self.parameters.force_charge;
        let range_sq = self.range_sq();
        let NodeStore {
            x,
            y,
            z,
            mass,
            ax,
            ay,
            az,
            ..
        } = &mut self.nodes;
        // Reslicing to one length folds the bounds checks away.
        let (x, y, z, mass) = (&x[..n], &y[..n], &z[..n], &mass[..n]);
        let (ax, ay, az) = (&mut ax[..n], &mut ay[..n], &mut az[..n]);
        // Every entry is a single node here.
        self.limits.clear();
        self.limits.resize(n, force_max);

        for i in 0..n {
            // A free slot has zero mass.
            let [fx, fy, fz] = repulsion_on(
                [x[i], y[i], z[i]],
                -charge * mass[i],
                range_sq,
                x,
                y,
                z,
                mass,
                &self.limits,
            );
            ax[i] += fx;
            ay[i] += fy;
            az[i] += fz;
        }
    }

    /// Barnes-Hut: per leaf, traverse the octree once to collect nearby bodies and distant cell
    /// aggregates, then run the kernel over that list for each node in the leaf.
    fn repel_approx(&mut self) {
        let n = self.nodes.len();
        let force_max = self.parameters.force_max;
        let charge = self.parameters.force_charge;
        let range_sq = self.range_sq();
        let theta = self.parameters.theta;
        let NodeStore {
            x,
            y,
            z,
            mass,
            ax,
            ay,
            az,
            ..
        } = &mut self.nodes;
        let (x, y, z, mass) = (&x[..n], &y[..n], &z[..n], &mass[..n]);
        let (ax, ay, az) = (&mut ax[..n], &mut ay[..n], &mut az[..n]);

        self.tree.build(Bodies { x, y, z, mass });
        let tree = &self.tree;
        let sorted = &tree.sorted;
        for leaf in &tree.leaves {
            tree.gather(leaf, theta, force_max, &mut self.interactions);
            let list = &self.interactions;
            for k in leaf.start as usize..leaf.end as usize {
                let [fx, fy, fz] = repulsion_on(
                    [sorted.x[k], sorted.y[k], sorted.z[k]],
                    -charge * sorted.mass[k],
                    range_sq,
                    &list.x,
                    &list.y,
                    &list.z,
                    &list.mass,
                    &list.limit,
                );
                let body = tree.order[k] as usize;
                ax[body] += fx;
                ay[body] += fy;
                az[body] += fz;
            }
        }
    }

    /// Spring attraction along the edges, proportional to the offset, so an edge within the ceiling
    /// needs no square root.
    ///
    /// Unclamped: the only force growing with distance decides edge length, and a clamped spring on
    /// one edge would lose to a large graph's repulsion.
    fn attract(&mut self) {
        let rate = self.parameters.force_spring * 0.5;
        let max = self.parameters.link_distance_max.unwrap_or(f32::INFINITY);
        let nodes = &mut self.nodes;
        for edge in self.graph.edge_references() {
            let (i, j) = (edge.source().index(), edge.target().index());
            let (dx, dy, dz) = (
                nodes.x[j] - nodes.x[i],
                nodes.y[j] - nodes.y[i],
                nodes.z[j] - nodes.z[i],
            );
            // Only an edge past the ceiling pays for the square root.
            let ceiling = max * edge.weight().reach;
            let stiffness = edge.weight().stiffness;
            let squared = dx * dx + dy * dy + dz * dz;
            let strength = if squared > ceiling * ceiling {
                let distance = squared.sqrt();
                rate * (stiffness + OVERSTRETCH_STIFFNESS * (distance - ceiling) / distance)
            } else {
                rate * stiffness
            };
            let (fx, fy, fz) = (dx * strength, dy * strength, dz * strength);
            nodes.ax[i] += fx;
            nodes.ay[i] += fy;
            nodes.az[i] += fz;
            nodes.ax[j] -= fx;
            nodes.ay[j] -= fy;
            nodes.az[j] -= fz;
        }
    }

    /// Removes the radial part of each accumulated force, leaving the motion around the shell. A
    /// node bound for the origin keeps none.
    fn discard_radial_force(&mut self) {
        let n = self.nodes.len();
        let NodeStore {
            x,
            y,
            z,
            ax,
            ay,
            az,
            level,
            ..
        } = &mut self.nodes;
        for i in 0..n {
            let radius = (x[i] * x[i] + y[i] * y[i] + z[i] * z[i]).sqrt();
            if level[i] <= 0.0 || radius == 0.0 {
                [ax[i], ay[i], az[i]] = [0.0; 3];
                continue;
            }
            let along = (ax[i] * x[i] + ay[i] * y[i] + az[i] * z[i]) / (radius * radius);
            ax[i] -= along * x[i];
            ay[i] -= along * y[i];
            az[i] -= along * z[i];
        }
    }

    /// Integrates the accumulated forces. `damping`, the share of velocity kept each step, is what
    /// the settling envelope leaves of [`SimulationParameters::damping_factor`].
    fn integrate(&mut self, dt: f32, damping: f32, free: [bool; 3]) {
        // The accumulators hold force: one `dt` makes a velocity change, the other the step.
        let dv = dt * dt * self.parameters.node_speed;
        let n = self.nodes.len();
        let mobility = &self.nodes.mobility[..n];
        // One axis at a time, so the working set fits the vector registers.
        for (axis, (pos, vel, acc)) in [
            (&mut self.nodes.x, &mut self.nodes.vx, &self.nodes.ax),
            (&mut self.nodes.y, &mut self.nodes.vy, &self.nodes.ay),
            (&mut self.nodes.z, &mut self.nodes.vz, &self.nodes.az),
        ]
        .into_iter()
        .enumerate()
        {
            if !free[axis] {
                continue;
            }
            integrate_axis(
                &mut pos[..n],
                &mut vel[..n],
                &acc[..n],
                mobility,
                dv,
                damping,
                dt,
            );
        }
    }

    /// Applies the axis constraints of the current mode after the forces move the nodes freely.
    /// Returns whether every constraint has reached the target, the other half of the settled test.
    fn constrain(&mut self, dt: f32) -> bool {
        let n = self.nodes.len();
        let mut reached = true;
        let retained = (-CONSTRAINT_RATE * dt).exp();
        let slack = self.parameters.dag_level_slack.max(0.0);

        if let (Some(distance), Layers::Planes) = (
            self.parameters.dag_level_distance,
            self.parameters.dag_layers,
        ) {
            self.targets.clear();
            self.targets
                .extend(self.nodes.level[..n].iter().map(|level| level * distance));
            reached &= constrain_axis(
                &mut self.nodes.y[..n],
                &mut self.nodes.vy[..n],
                &self.nodes.mobility[..n],
                &self.targets,
                slack,
                retained,
            );
        }

        if self.parameters.dimensions == Dimensions::Two {
            self.targets.clear();
            self.targets.resize(n, 0.0);
            reached &= constrain_axis(
                &mut self.nodes.z[..n],
                &mut self.nodes.vz[..n],
                &self.nodes.mobility[..n],
                &self.targets,
                0.0,
                retained,
            );
        }

        // After the plane, which scaling about the origin leaves planar.
        if let (Some(distance), Layers::Shells) = (
            self.parameters.dag_level_distance,
            self.parameters.dag_layers,
        ) {
            reached &= constrain_shells(&mut self.nodes, distance, slack, retained);
        }

        reached
    }

    pub fn node(&self, idx: DefaultNodeIdx) -> Option<NodeRef<'_, UserNodeData>> {
        Some(NodeRef {
            index: idx,
            nodes: &self.nodes,
            user_data: self.graph.node_weight(idx)?,
        })
    }

    pub fn visit_nodes<F: FnMut(NodeRef<'_, UserNodeData>)>(&self, mut cb: F) {
        for idx in self.graph.node_indices() {
            cb(NodeRef {
                index: idx,
                nodes: &self.nodes,
                user_data: &self.graph[idx],
            });
        }
    }

    pub fn visit_nodes_mut<F: FnMut(NodeMut<'_, UserNodeData>)>(&mut self, mut cb: F) {
        for idx in self.graph.node_indices().collect::<Vec<_>>() {
            cb(NodeMut {
                slot: idx.index(),
                nodes: &mut self.nodes,
                rest: &mut self.rest,
                user_data: &mut self.graph[idx],
            });
        }
    }

    /// Wakes the layout regardless, since edge data is read every step.
    pub fn visit_edges_mut<
        F: FnMut(DefaultNodeIdx, DefaultNodeIdx, &mut EdgeData<UserEdgeData>),
    >(
        &mut self,
        mut cb: F,
    ) {
        self.rest.wake();
        for idx in self.graph.edge_indices().collect::<Vec<_>>() {
            let Some((source, target)) = self.graph.edge_endpoints(idx) else {
                continue;
            };
            if let Some(edge) = self.graph.edge_weight_mut(idx) {
                cb(source, target, edge);
            }
        }
    }

    pub fn visit_edges<
        F: FnMut(NodeRef<'_, UserNodeData>, NodeRef<'_, UserNodeData>, &EdgeData<UserEdgeData>),
    >(
        &self,
        mut cb: F,
    ) {
        for edge_ref in self.graph.edge_references() {
            let (source, target) = (edge_ref.source(), edge_ref.target());
            cb(
                NodeRef {
                    index: source,
                    nodes: &self.nodes,
                    user_data: &self.graph[source],
                },
                NodeRef {
                    index: target,
                    nodes: &self.nodes,
                    user_data: &self.graph[target],
                },
                edge_ref.weight(),
            );
        }
    }
}

/// Cannot be constructed by the user.
pub struct NodeRef<'a, UserNodeData = ()> {
    pub user_data: &'a UserNodeData,
    index: DefaultNodeIdx,
    nodes: &'a NodeStore,
}

impl<UserNodeData> NodeRef<'_, UserNodeData> {
    pub fn x(&self) -> f32 {
        self.nodes.x[self.index.index()]
    }

    pub fn y(&self) -> f32 {
        self.nodes.y[self.index.index()]
    }

    pub fn z(&self) -> f32 {
        self.nodes.z[self.index.index()]
    }

    pub fn position(&self) -> [f32; 3] {
        [self.x(), self.y(), self.z()]
    }

    pub fn mass(&self) -> f32 {
        self.nodes.mass[self.index.index()]
    }

    pub fn is_anchor(&self) -> bool {
        self.nodes.mobility[self.index.index()] == 0.0
    }

    pub fn index(&self) -> DefaultNodeIdx {
        self.index
    }
}

/// Cannot be constructed by the user.
pub struct NodeMut<'a, UserNodeData = ()> {
    pub user_data: &'a mut UserNodeData,
    slot: usize,
    nodes: &'a mut NodeStore,
    /// Woken by every setter, each invalidating the rest the layout had reached.
    rest: &'a mut Rest,
}

impl<UserNodeData> NodeMut<'_, UserNodeData> {
    /// See [`NodeData::level`].
    pub fn level(&self) -> f32 {
        self.nodes.level[self.slot]
    }

    pub fn position(&self) -> [f32; 3] {
        [
            self.nodes.x[self.slot],
            self.nodes.y[self.slot],
            self.nodes.z[self.slot],
        ]
    }

    /// Moves the node, leaving the velocity alone.
    pub fn set_position(&mut self, [x, y, z]: [f32; 3]) {
        self.rest.wake();
        self.nodes.x[self.slot] = x;
        self.nodes.y[self.slot] = y;
        self.nodes.z[self.slot] = z;
    }

    /// Sets the velocity, leaving the position alone. Zero stops the node dead, as restarting a
    /// layout needs.
    pub fn set_velocity(&mut self, [vx, vy, vz]: [f32; 3]) {
        self.rest.wake();
        self.nodes.vx[self.slot] = vx;
        self.nodes.vy[self.slot] = vy;
        self.nodes.vz[self.slot] = vz;
    }

    /// See [`NodeData::level`].
    pub fn set_level(&mut self, level: f32) {
        self.rest.wake();
        self.nodes.level[self.slot] = level;
    }

    pub fn set_mass(&mut self, mass: f32) {
        self.rest.wake();
        self.nodes.mass[self.slot] = mass;
    }

    pub fn apply_force(&mut self, force: [f32; 3]) {
        self.rest.wake();
        self.nodes.add_force(self.slot, force);
    }

    pub fn set_anchor(&mut self, is_anchor: bool) {
        self.rest.wake();
        self.nodes.mobility[self.slot] = if is_anchor { 0.0 } else { 1.0 };
    }
}

/// Repulsion exerted on one node by a list of masses: every node in the graph, or an octree
/// interaction list. An entry at the target's own position contributes nothing.
///
/// `charge_target` folds in the target's mass and the charge sign. `limit` caps each entry per
/// component: `force_max` for a node, a multiple for a cell aggregate.
///
/// The lane loop is shaped so LLVM vectorizes without bounds checks or NaN correction inside;
/// `tests/codegen.rs` enforces both. Not `#[inline]`: one call per node is already amortized, and
/// one out-of-line copy keeps the vectorized loop findable in a disassembler.
#[expect(clippy::too_many_arguments)]
fn repulsion_on(
    target: [f32; 3],
    charge_target: f32,
    range_sq: f32,
    x: &[f32],
    y: &[f32],
    z: &[f32],
    mass: &[f32],
    limit: &[f32],
) -> [f32; 3] {
    #[cfg(target_arch = "x86_64")]
    if is_x86_feature_detected!("avx2") {
        // SAFETY: the CPU reports the feature the function is compiled for.
        return unsafe { repulsion_on_avx2(target, charge_target, range_sq, x, y, z, mass, limit) };
    }
    sum_repulsion(target, charge_target, range_sq, x, y, z, mass, limit)
}

/// The desktop build targets baseline x86-64, whose vectors are half as wide.
#[cfg(target_arch = "x86_64")]
#[expect(clippy::too_many_arguments)]
#[target_feature(enable = "avx2")]
fn repulsion_on_avx2(
    target: [f32; 3],
    charge_target: f32,
    range_sq: f32,
    x: &[f32],
    y: &[f32],
    z: &[f32],
    mass: &[f32],
    limit: &[f32],
) -> [f32; 3] {
    sum_repulsion(target, charge_target, range_sq, x, y, z, mass, limit)
}

#[expect(clippy::too_many_arguments)]
#[inline(always)]
fn sum_repulsion(
    target: [f32; 3],
    charge_target: f32,
    range_sq: f32,
    x: &[f32],
    y: &[f32],
    z: &[f32],
    mass: &[f32],
    limit: &[f32],
) -> [f32; 3] {
    let n = x
        .len()
        .min(y.len())
        .min(z.len())
        .min(mass.len())
        .min(limit.len());
    let (x, y, z, mass, limit) = (&x[..n], &y[..n], &z[..n], &mass[..n], &limit[..n]);
    let [tx, ty, tz] = target;
    // One accumulator per lane: LLVM will not reorder a single running sum to vectorize it.
    let mut fx = [0.0f32; LANES];
    let mut fy = [0.0f32; LANES];
    let mut fz = [0.0f32; LANES];

    // Chunked: LLVM leaves a bounds check in the loop for five slices indexed in step.
    let (xc, xr) = x.as_chunks::<LANES>();
    let (yc, yr) = y.as_chunks::<LANES>();
    let (zc, zr) = z.as_chunks::<LANES>();
    let (mc, mr) = mass.as_chunks::<LANES>();
    let (lc, lr) = limit.as_chunks::<LANES>();

    for ((((xs, ys), zs), ms), ls) in xc.iter().zip(yc).zip(zc).zip(mc).zip(lc) {
        for l in 0..LANES {
            let dx = xs[l] - tx;
            let dy = ys[l] - ty;
            let dz = zs[l] - tz;
            let d2 = dx * dx + dy * dy + dz * dz;
            // A select rather than a branch, so the lanes stay in step.
            let reach = if d2 < range_sq { charge_target } else { 0.0 };
            let strength = reach * ms[l] * inv_cube(d2);
            // Clamped per interaction, so one close neighbour cannot dominate the sum.
            fx[l] += clamp_symmetric(dx * strength, ls[l]);
            fy[l] += clamp_symmetric(dy * strength, ls[l]);
            fz[l] += clamp_symmetric(dz * strength, ls[l]);
        }
    }
    // At most `LANES - 1`, a bound the type carries, so this is peeled.
    for ((((&px, &py), &pz), &m), &l) in xr.iter().zip(yr).zip(zr).zip(mr).zip(lr) {
        let (dx, dy, dz) = (px - tx, py - ty, pz - tz);
        let d2 = dx * dx + dy * dy + dz * dz;
        let reach = if d2 < range_sq { charge_target } else { 0.0 };
        let strength = reach * m * inv_cube(d2);
        fx[0] += clamp_symmetric(dx * strength, l);
        fy[0] += clamp_symmetric(dy * strength, l);
        fz[0] += clamp_symmetric(dz * strength, l);
    }

    [reduce(fx), reduce(fy), reduce(fz)]
}

/// Repulsion falls off with the square of the distance, and the direction vector needs one more
/// division to normalize: `1 / (d2 + SOFTENING)^(3/2)`.
#[inline(always)]
fn inv_cube(d2: f32) -> f32 {
    let softened = d2 + SOFTENING;
    (softened * softened.sqrt()).recip()
}

/// `v` held to `+/-limit`, and a NaN `v` to `-limit`. `limit` is assumed not NaN, the one input
/// the two variants disagree on.
///
/// Two variants because the clamp that lowers to bare instructions differs by backend. `f32::max`
/// and `f32::min` are IEEE `maxNum`/`minNum`, returning the non-NaN operand; `maxps`/`minps` on x86
/// and `f32x4.pmax`/`pmin` on wasm do the opposite and need a NaN test and select per bound.
/// aarch64 has `maxNum` in hardware as `fmaxnm`/`fminnm`, where the comparisons need the second
/// instruction. `tests/codegen.rs` checks the intended variant.
#[inline(always)]
fn clamp_symmetric(v: f32, limit: f32) -> f32 {
    #[cfg(target_arch = "aarch64")]
    {
        clamp_ieee(v, limit)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        clamp_comparisons(v, limit)
    }
}

// Both compile everywhere so either is testable from one host.
#[allow(dead_code)]
#[inline(always)]
fn clamp_ieee(v: f32, limit: f32) -> f32 {
    v.max(-limit).min(limit)
}

/// Comparisons in the order the instructions already implement, so nothing is corrected after.
#[allow(dead_code)]
#[inline(always)]
fn clamp_comparisons(v: f32, limit: f32) -> f32 {
    let v = if v > -limit { v } else { -limit };
    if v < limit { v } else { limit }
}

fn reduce(lanes: [f32; LANES]) -> f32 {
    lanes.iter().sum()
}

/// Relaxes one axis geometrically toward a target, discarding the forces on that axis, so the gap
/// never overshoots. Returns whether every movable node is exactly on target.
fn constrain_axis(
    pos: &mut [f32],
    vel: &mut [f32],
    mobility: &[f32],
    targets: &[f32],
    slack: f32,
    retained: f32,
) -> bool {
    let n = pos
        .len()
        .min(vel.len())
        .min(mobility.len())
        .min(targets.len());
    let mut reached = true;
    for i in 0..n {
        if mobility[i] == 0.0 {
            continue;
        }
        let target = pos[i].clamp(targets[i] - slack, targets[i] + slack);
        if pos[i] == target {
            // Inside the band the forces own the axis; zeroing velocity would stop the stacking.
            continue;
        }
        pos[i] += (target - pos[i]) * (1.0 - retained);
        if (pos[i] - target).abs() < CONSTRAINT_SNAP {
            pos[i] = target;
        } else {
            reached = false;
        }
        vel[i] = 0.0;
    }
    reached
}

/// Relaxes each node's distance from the origin toward the node's shell, scaling the position
/// about the origin and discarding radial velocity, as [`constrain_axis`] does for one axis.
fn constrain_shells(nodes: &mut NodeStore, distance: f32, slack: f32, retained: f32) -> bool {
    let n = nodes.len();
    let NodeStore {
        x,
        y,
        z,
        vx,
        vy,
        vz,
        mobility,
        level,
        ..
    } = nodes;
    let mut reached = true;
    for i in 0..n {
        if mobility[i] == 0.0 {
            continue;
        }
        let shell = level[i] * distance;
        let radius = (x[i] * x[i] + y[i] * y[i] + z[i] * z[i]).sqrt();
        let target = radius.clamp((shell - slack).max(0.0), shell + slack);
        if radius == target {
            continue;
        }
        let mut relaxed = radius + (target - radius) * (1.0 - retained);
        // Scaling three coordinates places a far shell's radius only to a few parts in ten million.
        if (relaxed - target).abs() < CONSTRAINT_SNAP.max(target * 1e-6) {
            relaxed = target;
        } else {
            reached = false;
        }
        if radius == 0.0 {
            // No direction to scale along; any will do.
            [x[i], y[i], z[i]] = [relaxed, 0.0, 0.0];
            continue;
        }
        let scale = relaxed / radius;
        [x[i], y[i], z[i]] = [x[i] * scale, y[i] * scale, z[i] * scale];
        if relaxed == 0.0 {
            [vx[i], vy[i], vz[i]] = [0.0; 3];
            continue;
        }
        let along = (vx[i] * x[i] + vy[i] * y[i] + vz[i] * z[i]) / (relaxed * relaxed);
        vx[i] -= along * x[i];
        vy[i] -= along * y[i];
        vz[i] -= along * z[i];
    }
    reached
}

/// Damped explicit-Euler step for one axis. `mobility` zeroes both the velocity and the motion of
/// an anchored or free slot, which keeps the loop free of branches.
fn integrate_axis(
    pos: &mut [f32],
    vel: &mut [f32],
    acc: &[f32],
    mobility: &[f32],
    dv: f32,
    damping: f32,
    dt: f32,
) {
    let n = pos.len().min(vel.len()).min(acc.len()).min(mobility.len());
    let (pos, vel) = (&mut pos[..n], &mut vel[..n]);
    let (acc, mobility) = (&acc[..n], &mobility[..n]);
    for i in 0..n {
        let v = (vel[i] + acc[i] * dv) * damping * mobility[i];
        vel[i] = v;
        pos[i] += v * dt;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn radius([x, y, z]: [f32; 3]) -> f32 {
        (x * x + y * y + z * z).sqrt()
    }

    fn positions<N, E>(graph: &ForceGraph<N, E>) -> Vec<[f32; 3]> {
        let mut out = Vec::new();
        graph.visit_nodes(|node| out.push(node.position()));
        out
    }

    #[test]
    fn test_default() {
        let mut graph = <ForceGraph>::new(Default::default());
        let n1_idx = graph.add_node(NodeData {
            x: 0.1,
            y: 0.2,
            z: 0.3,
            ..Default::default()
        });
        let n2_idx = graph.add_node(NodeData {
            x: 0.3,
            y: 0.4,
            z: 0.5,
            ..Default::default()
        });
        graph.add_edge(n1_idx, n2_idx, Default::default());
    }

    #[test]
    fn test_user_data() {
        let mut graph = ForceGraph::new(Default::default());

        #[derive(Default)]
        struct UserNodeData {}
        #[derive(Default)]
        struct UserEdgeData {}

        let n1_idx = graph.add_node(NodeData {
            x: 0.1,
            y: 0.2,
            user_data: UserNodeData {},
            ..Default::default()
        });
        let n2_idx = graph.add_node(NodeData {
            x: 0.3,
            y: 0.4,
            user_data: UserNodeData {},
            ..Default::default()
        });

        graph.add_edge(
            n1_idx,
            n2_idx,
            EdgeData {
                reach: 1.0,
                stiffness: 1.0,
                user_data: UserEdgeData {},
            },
        );
    }

    #[test]
    fn repulsion_separates_on_every_axis() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dag_level_distance: None,
            ..Default::default()
        });
        graph.add_node(NodeData {
            x: -1.0,
            y: -1.0,
            z: -1.0,
            ..Default::default()
        });
        graph.add_node(NodeData {
            x: 1.0,
            y: 1.0,
            z: 1.0,
            ..Default::default()
        });

        graph.update(FIXED_STEP);
        let [a, b] = positions(&graph)[..] else {
            unreachable!()
        };
        for axis in 0..3 {
            assert!(a[axis] < -1.0, "axis {axis} not repelled: {a:?}");
            assert!(b[axis] > 1.0, "axis {axis} not repelled: {b:?}");
        }
    }

    #[test]
    fn spring_contracts_a_long_edge() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            force_charge: 0.0,
            ..Default::default()
        });
        let a = graph.add_node(NodeData {
            z: -500.0,
            ..Default::default()
        });
        let b = graph.add_node(NodeData {
            z: 500.0,
            ..Default::default()
        });
        graph.add_edge(a, b, Default::default());

        graph.update(FIXED_STEP);
        let [pa, pb] = positions(&graph)[..] else {
            unreachable!()
        };
        assert!(pa[2] > -500.0 && pb[2] < 500.0, "{pa:?} {pb:?}");
    }

    #[test]
    fn anchored_nodes_never_move() {
        let mut graph = <ForceGraph>::new(Default::default());
        let anchor = graph.add_node(NodeData {
            is_anchor: true,
            ..Default::default()
        });
        let free = graph.add_node(NodeData {
            x: 10.0,
            ..Default::default()
        });
        graph.add_edge(anchor, free, Default::default());

        for _ in 0..10 {
            graph.update(FIXED_STEP);
        }
        assert_eq!(positions(&graph)[0], [0.0; 3]);
    }

    #[test]
    fn coincident_nodes_stay_finite() {
        let mut graph = <ForceGraph>::new(Default::default());
        graph.add_node(NodeData::default());
        graph.add_node(NodeData::default());

        graph.update(FIXED_STEP);
        for position in positions(&graph) {
            assert!(position.iter().all(|c| c.is_finite()), "{position:?}");
        }
    }

    /// Runs both clamp variants on every host: CI covers one architecture, and phones get aarch64.
    ///
    /// A NaN `limit` is excluded: the comparisons propagate a NaN and IEEE `minNum` discards one.
    /// Zeroes compare by value: `fmaxnm`/`fminnm` pick a different zero than the comparisons, and
    /// `-0.0` moves a node as far as `0.0`. Only aarch64 shows this, under `just test-arm`.
    #[test]
    fn the_two_clamp_variants_agree() {
        const INTERESTING: [f32; 11] = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            f32::MIN_POSITIVE,
            -f32::MIN_POSITIVE,
            1e30,
            -1e30,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ];
        for v in INTERESTING {
            for limit in INTERESTING.into_iter().filter(|l| !l.is_nan()) {
                let (ieee, comparisons) = (clamp_ieee(v, limit), clamp_comparisons(v, limit));
                assert!(
                    ieee == comparisons || (ieee.is_nan() && comparisons.is_nan()),
                    "clamp({v:?}, {limit:?}): aarch64 gives {ieee:?}, everywhere else \
                     {comparisons:?}"
                );
            }
        }
    }

    /// A non-finite coordinate makes the falloff zero and the force `inf * 0`; the bound the clamp
    /// picks for a NaN keeps the rest of the graph finite.
    #[test]
    fn a_runaway_coordinate_does_not_infect_the_others() {
        let mut graph = <ForceGraph>::new(Default::default());
        graph.add_node(NodeData {
            x: f32::INFINITY,
            ..Default::default()
        });
        graph.add_node(NodeData {
            x: 10.0,
            y: 20.0,
            ..Default::default()
        });

        graph.update(FIXED_STEP);
        let neighbour = positions(&graph)[1];
        assert!(
            neighbour.iter().all(|c| c.is_finite()),
            "{neighbour:?} was reached through the node at infinity"
        );
    }

    /// The hot loops scan freed slots too, so a removal must not perturb the survivors.
    #[test]
    fn removed_nodes_exert_no_force() {
        let layout = |remove: bool| {
            let mut graph = <ForceGraph>::new(Default::default());
            let a = graph.add_node(NodeData {
                x: 100.0,
                ..Default::default()
            });
            let b = graph.add_node(NodeData {
                x: 200.0,
                y: 50.0,
                z: 25.0,
                ..Default::default()
            });
            graph.add_edge(a, b, Default::default());
            if remove {
                let doomed = graph.add_node(NodeData {
                    x: 150.0,
                    ..Default::default()
                });
                graph.add_edge(a, doomed, Default::default());
                graph.remove_node(doomed);
            }
            for _ in 0..20 {
                graph.update(FIXED_STEP);
            }
            positions(&graph)
        };
        assert_eq!(layout(false), layout(true));
    }
    /// Displacement after one step from rest is proportional to the force, so the displacement
    /// measures the approximation directly.
    fn one_step_displacements(theta: f32) -> Vec<[f32; 3]> {
        let mut seed = 0x5eed_1337u32;
        let mut rng = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        let mut graph = <ForceGraph>::new(SimulationParameters {
            theta,
            ..Default::default()
        });
        let nodes: Vec<_> = (0..1500)
            .map(|_| {
                graph.add_node(NodeData {
                    x: rng() * 1000.0,
                    y: rng() * 1000.0,
                    z: rng() * 1000.0,
                    ..Default::default()
                })
            })
            .collect();
        for i in 1..nodes.len() {
            graph.add_edge(nodes[i], nodes[i / 2], Default::default());
        }
        let before = positions(&graph);
        graph.update(FIXED_STEP);
        let after = positions(&graph);
        before
            .iter()
            .zip(after)
            .map(|(b, a)| [a[0] - b[0], a[1] - b[1], a[2] - b[2]])
            .collect()
    }

    fn worst_relative_error(exact: &[[f32; 3]], approx: &[[f32; 3]]) -> f32 {
        let magnitude = |v: &[f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let scale = exact.iter().map(magnitude).fold(0.0f32, f32::max);
        exact
            .iter()
            .zip(approx)
            .map(|(e, a)| magnitude(&[a[0] - e[0], a[1] - e[1], a[2] - e[2]]) / scale)
            .fold(0.0f32, f32::max)
    }

    /// Up to the order the lanes sum in: a vanishing angle descends to the leaves.
    #[test]
    fn tiny_theta_reproduces_exact_summation() {
        let exact = one_step_displacements(0.0);
        let approx = one_step_displacements(1e-4);
        assert!(
            worst_relative_error(&exact, &approx) < 1e-4,
            "{}",
            worst_relative_error(&exact, &approx)
        );
    }

    /// ~1% worst case, and the bound must hold at any size: an aggregate that mis-clamps the share
    /// of the force makes the error grow with node count.
    #[test]
    fn default_theta_stays_accurate() {
        let exact = one_step_displacements(0.0);
        let approx = one_step_displacements(SimulationParameters::default().theta);
        let error = worst_relative_error(&exact, &approx);
        assert!(error < 0.02, "{error}");
    }

    /// Exactly planar, with the other axes intact.
    #[test]
    fn two_dimensions_collapse_onto_the_plane() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dimensions: Dimensions::Two,
            dag_level_distance: None,
            ..Default::default()
        });
        for i in 0..8 {
            graph.add_node(NodeData {
                x: i as f32 * 40.0,
                y: (i % 3) as f32 * 40.0,
                z: 500.0 - i as f32 * 120.0,
                ..Default::default()
            });
        }

        for _ in 0..120 {
            graph.update(FIXED_STEP);
        }
        let positions = positions(&graph);
        assert!(positions.iter().all(|p| p[2] == 0.0), "{positions:?}");
        let spread = |axis: usize| {
            let values = positions.iter().map(|p| p[axis]);
            values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
        };
        assert!(spread(0) > 100.0 && spread(1) > 100.0, "{positions:?}");
    }

    /// A cold start hands `update` the longest frames on exactly the steps a freshly seeded layout
    /// has the largest forces, and that opening burst fixes the final spread.
    #[test]
    fn the_layout_does_not_depend_on_the_frame_rate() {
        let seeded = || {
            let mut graph = <ForceGraph>::new(SimulationParameters {
                dag_level_distance: None,
                ..Default::default()
            });
            for i in 0..48 {
                graph.add_node(NodeData {
                    // Crowded, as a rescatter leaves it.
                    x: ((i * 37) % 41) as f32,
                    y: ((i * 53) % 41) as f32,
                    z: ((i * 71) % 41) as f32,
                    ..Default::default()
                });
            }
            graph
        };
        let run = |dt: f32, frames: usize| {
            let mut graph = seeded();
            for _ in 0..frames {
                graph.update(dt);
            }
            positions(&graph)
        };
        assert_eq!(
            run(FIXED_STEP, 180),
            run(FIXED_STEP * MAX_STEPS_PER_UPDATE as f32, 60)
        );
    }

    /// Settle only on genuine rest, so a test of that path is not answered by the deadline.
    fn converge() -> SimulationParameters {
        SimulationParameters {
            settle_after: None,
            ..Default::default()
        }
    }

    fn settle<N, E>(graph: &mut ForceGraph<N, E>, limit: usize) -> usize {
        for step in 0..limit {
            graph.update(FIXED_STEP);
            if graph.is_settled() {
                return step + 1;
            }
        }
        panic!("did not settle in {limit} steps");
    }

    #[test]
    fn a_settled_layout_holds_still() {
        let mut graph = <ForceGraph>::new(converge());
        let a = graph.add_node(NodeData::default());
        let b = graph.add_node(NodeData {
            x: 200.0,
            ..Default::default()
        });
        graph.add_edge(a, b, Default::default());

        settle(&mut graph, 10_000);
        let resting = positions(&graph);
        for _ in 0..1000 {
            graph.update(FIXED_STEP);
        }
        assert_eq!(positions(&graph), resting);
    }

    #[test]
    fn a_force_wakes_a_settled_layout() {
        let mut graph = <ForceGraph>::new(converge());
        let grabbed = graph.add_node(NodeData::default());
        let other = graph.add_node(NodeData {
            x: 200.0,
            ..Default::default()
        });
        graph.add_edge(grabbed, other, Default::default());

        settle(&mut graph, 10_000);
        let resting = positions(&graph);
        graph.apply_force(grabbed, [400.0, 0.0, 0.0]);
        assert!(!graph.is_settled());

        graph.update(FIXED_STEP);
        assert!(positions(&graph)[0][0] > resting[0][0]);
    }

    /// This graph's repulsion outweighs the springs, so the graph expands without end; only
    /// withdrawing the damping brings rest.
    #[test]
    fn an_unconverged_layout_is_brought_to_rest() {
        let budget = 5.0f32;
        let dt = FIXED_STEP;
        let mut graph = <ForceGraph>::new(SimulationParameters {
            settle_after: Some(budget),
            ..Default::default()
        });
        for i in 0..64 {
            graph.add_node(NodeData {
                x: i as f32 * 10.0,
                y: (i % 8) as f32 * 10.0,
                z: (i % 5) as f32 * 10.0,
                ..Default::default()
            });
        }

        let steps = settle(&mut graph, 10_000);
        assert!(steps <= (budget / dt).ceil() as usize, "{steps}");
        let resting = positions(&graph);
        for _ in 0..600 {
            graph.update(dt);
        }
        assert_eq!(positions(&graph), resting);
    }

    /// Settling leaves the forces in place, so the damping returns gradually and one frame of
    /// contact does not release the whole layout.
    #[test]
    fn a_brief_touch_gives_a_brief_response() {
        let dt = FIXED_STEP;
        let travel = |frames_held: usize| {
            let mut graph = <ForceGraph>::new(Default::default());
            let mut nodes = Vec::new();
            for i in 0..64 {
                nodes.push(graph.add_node(NodeData {
                    x: i as f32 * 10.0,
                    y: (i % 8) as f32 * 10.0,
                    z: (i % 5) as f32 * 10.0,
                    ..Default::default()
                }));
            }
            settle(&mut graph, 10_000);
            let resting = positions(&graph);
            for frame in 0..600 {
                if frame < frames_held {
                    graph.apply_force(nodes[0], [20.0, 0.0, 0.0]);
                }
                graph.update(dt);
            }
            positions(&graph)
                .iter()
                .zip(&resting)
                .skip(1)
                .map(|(a, b)| {
                    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
                })
                .sum::<f32>()
        };

        let (touched, held) = (travel(1), travel(600));
        assert!(touched * 10.0 < held, "touched {touched} held {held}");
    }

    /// A node held for longer than the window keeps the layout in motion, and letting go grants a
    /// full window rather than what was left of one.
    #[test]
    fn the_settling_window_runs_from_the_last_disturbance() {
        let budget = 5.0f32;
        let dt = FIXED_STEP;
        let steps = (budget / dt).ceil() as usize;
        let mut graph = <ForceGraph>::new(SimulationParameters {
            settle_after: Some(budget),
            ..Default::default()
        });
        let held = graph.add_node(NodeData::default());
        graph.add_node(NodeData {
            x: 200.0,
            ..Default::default()
        });

        for _ in 0..steps * 2 {
            graph.apply_force(held, [50.0, 0.0, 0.0]);
            graph.update(dt);
            assert!(!graph.is_settled());
        }
        graph.apply_force(held, [50.0, 0.0, 0.0]);
        assert!(settle(&mut graph, 10_000) <= steps);
    }

    #[test]
    fn changing_a_parameter_wakes_a_settled_layout() {
        let mut graph = <ForceGraph>::new(converge());
        graph.add_node(NodeData::default());
        graph.add_node(NodeData {
            x: 200.0,
            ..Default::default()
        });

        settle(&mut graph, 10_000);
        graph.parameters_mut().force_charge *= 4.0;
        assert!(!graph.is_settled());
    }

    /// A constraint moves nodes without giving them velocity, so speed alone would freeze a graph
    /// mid-collapse.
    #[test]
    fn a_collapsing_layout_is_not_settled() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dimensions: Dimensions::Two,
            // No forces, so speed alone would call the layout settled at the first step.
            force_charge: 0.0,
            ..converge()
        });
        graph.add_node(NodeData {
            z: 1000.0,
            ..Default::default()
        });

        graph.update(FIXED_STEP);
        assert!(!graph.is_settled(), "settled while still off the plane");
        settle(&mut graph, 10_000);
        assert_eq!(positions(&graph)[0][2], 0.0);
    }

    /// Repulsion acts along the offset between two nodes, so nodes sharing a coordinate get no
    /// force along that axis: a caller switching back to three dimensions has to reseed z.
    #[test]
    fn a_flat_layout_gets_no_depth_back() {
        let mut graph = <ForceGraph>::new(Default::default());
        for i in 0..8 {
            graph.add_node(NodeData {
                x: i as f32 * 40.0,
                y: (i % 3) as f32 * 40.0,
                ..Default::default()
            });
        }

        for _ in 0..30 {
            graph.update(FIXED_STEP);
        }
        let positions = positions(&graph);
        assert!(positions.iter().all(|p| p[2] == 0.0), "{positions:?}");
    }

    /// Spread across the band, none leaving the band.
    #[test]
    fn slack_stacks_a_crowded_shell() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dag_level_distance: Some(1000.0),
            dag_level_slack: 200.0,
            dimensions: Dimensions::Two,
            ..Default::default()
        });
        for i in 0..8 {
            graph.add_node(NodeData {
                x: i as f32 * 10.0,
                // Nodes at one radius have no radial force to spread them.
                y: 1000.0 + (i % 4) as f32,
                level: 1.0,
                ..Default::default()
            });
        }

        for _ in 0..120 {
            graph.update(FIXED_STEP);
        }
        let radii: Vec<f32> = positions(&graph).iter().map(|&p| radius(p)).collect();
        // Soft edge: see [`SimulationParameters::dag_level_slack`].
        assert!(
            radii.iter().all(|r| (r - 1000.0).abs() <= 220.0),
            "{radii:?}"
        );
        let spread = radii.iter().fold(0.0f32, |a, r| a.max((r - 1000.0).abs()));
        assert!(spread > 1.0, "{radii:?}");
    }

    /// However hard the graph pulls the two ends apart. A plain linear spring stretches further, a
    /// larger reach holds the edge further out, and an exempt edge is not held.
    #[test]
    fn a_ceiling_bounds_how_far_an_edge_stretches() {
        const CEILING: f32 = 250.0;

        /// The longest edge of a hub with sixteen leaves. The leaves push each other off the hub,
        /// so every edge carries the repulsion of the whole star rather than of one neighbour.
        fn star(link_distance_max: Option<f32>, reach: f32, stiffness: f32) -> f32 {
            let mut graph = <ForceGraph>::new(SimulationParameters {
                link_distance_max,
                // Flat, so the leaves crowd each other harder than a sphere of them would.
                dimensions: Dimensions::Two,
                ..Default::default()
            });
            let hub = graph.add_node(Default::default());
            for i in 0..16 {
                // A star from one point has no offset for repulsion to act along.
                let leaf = graph.add_node(NodeData {
                    x: (i % 4) as f32,
                    y: (i / 4) as f32,
                    z: (i % 3) as f32,
                    ..Default::default()
                });
                graph.add_edge(
                    hub,
                    leaf,
                    EdgeData {
                        reach,
                        stiffness,
                        user_data: (),
                    },
                );
            }
            for _ in 0..900 {
                graph.update(FIXED_STEP);
            }
            let nodes = positions(&graph);
            let [hx, hy, hz] = nodes[0];
            nodes[1..].iter().fold(0.0f32, |longest, [x, y, z]| {
                longest.max(((x - hx).powi(2) + (y - hy).powi(2) + (z - hz).powi(2)).sqrt())
            })
        }

        let held = star(Some(CEILING), 1.0, 1.0);
        let free = star(None, 1.0, 1.0);
        // Soft: see [`OVERSTRETCH_STIFFNESS`].
        assert!(held < CEILING * 1.3, "{held}");
        assert!(free > held * 1.5, "{free} vs {held}");
        let roomier = star(Some(CEILING), 1.5, 1.0);
        assert!(roomier > held * 1.3, "{roomier} vs {held}");
        assert!(roomier < CEILING * 1.5 * 1.3, "{roomier}");
        assert_eq!(star(Some(CEILING), f32::INFINITY, 1.0), free);
        // Slack within the ceiling, still held past it.
        let slack = star(Some(CEILING), 1.0, 0.0);
        assert!(slack > held, "{slack} vs {held}");
        assert!(slack < CEILING * 1.3, "{slack}");
    }

    /// Whatever placement the forces prefer.
    #[test]
    fn shells_follow_the_level() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dag_level_distance: Some(100.0),
            ..Default::default()
        });
        let chain: Vec<_> = (0..4)
            .map(|i| {
                graph.add_node(NodeData {
                    x: i as f32 * 10.0,
                    // Off the shell radius, so the constraint must move the node.
                    y: 500.0,
                    level: i as f32,
                    ..Default::default()
                })
            })
            .collect();
        for pair in chain.windows(2) {
            graph.add_edge(pair[0], pair[1], Default::default());
        }

        for _ in 0..120 {
            graph.update(FIXED_STEP);
        }
        let radii: Vec<f32> = positions(&graph).iter().map(|&p| radius(p)).collect();
        for (level, r) in radii.iter().enumerate() {
            assert!((r - level as f32 * 100.0).abs() < 0.1, "{radii:?}");
        }
    }

    /// A settled layout has no damping left, and [`Rest::wake`] hands back only as much as the
    /// disturbance lasts -- right for a drag, useless for a rearrangement.
    #[test]
    fn reviving_lets_a_settled_layout_rearrange() {
        /// Bounded, so a layout that will not settle fails the test instead of hanging.
        fn rest(graph: &mut ForceGraph) {
            for _ in 0..2000 {
                if graph.is_settled() {
                    return;
                }
                graph.update(FIXED_STEP);
            }
            panic!("never settled");
        }
        // Several nodes per shell, so lifting the layering leaves the forces work. Roomy shells
        // keep the held strain low, so a brief wake travels little.
        fn layered() -> ForceGraph {
            let mut graph = <ForceGraph>::new(SimulationParameters {
                dag_level_distance: Some(3000.0),
                ..Default::default()
            });
            let nodes: Vec<_> = (0..24)
                .map(|i| {
                    graph.add_node(NodeData {
                        x: (i % 6) as f32 * 30.0,
                        z: (i / 6) as f32 * 30.0,
                        // Off the innermost shell, which is a single point.
                        level: (i / 6 + 1) as f32,
                        ..Default::default()
                    })
                })
                .collect();
            for pair in nodes.windows(2) {
                graph.add_edge(pair[0], pair[1], Default::default());
            }
            rest(&mut graph);
            graph
        }
        fn unlayer(revive: bool) -> f32 {
            let mut graph = layered();
            let before = positions(&graph);
            graph.parameters_mut().dag_level_distance = None;
            if revive {
                graph.revive();
            }
            rest(&mut graph);
            positions(&graph)
                .iter()
                .zip(&before)
                .map(|(a, b)| {
                    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
                })
                .sum()
        }
        let (woken, revived) = (unlayer(false), unlayer(true));
        assert!(
            revived > woken * 10.0,
            "woken travelled {woken}, revived {revived}"
        );
    }

    #[test]
    fn an_applied_force_is_consumed_by_one_step() {
        let mut graph = <ForceGraph>::new(Default::default());
        let node = graph.add_node(NodeData::default());
        graph.apply_force(node, [30.0, 0.0, 0.0]);

        graph.update(FIXED_STEP);
        let pushed = positions(&graph)[0][0];
        graph.update(FIXED_STEP);
        let coasted = positions(&graph)[0][0] - pushed;
        assert!(pushed > 0.0, "{pushed}");
        assert!(coasted > 0.0 && coasted < pushed, "{pushed} {coasted}");
    }

    /// The tree covers live slots only.
    #[test]
    fn free_slots_do_not_disturb_the_tree() {
        let build = |hole: bool| {
            let mut graph = <ForceGraph>::new(Default::default());
            let mut nodes = Vec::new();
            for i in 0..16 {
                if hole && i == 8 {
                    let doomed = graph.add_node(NodeData {
                        x: 5e5,
                        y: 5e5,
                        z: 5e5,
                        ..Default::default()
                    });
                    graph.remove_node(doomed);
                }
                nodes.push(graph.add_node(NodeData {
                    x: i as f32 * 30.0,
                    y: (i % 4) as f32 * 30.0,
                    z: (i % 3) as f32 * 30.0,
                    ..Default::default()
                }));
            }
            for i in 1..nodes.len() {
                graph.add_edge(nodes[i], nodes[i - 1], Default::default());
            }
            for _ in 0..10 {
                graph.update(FIXED_STEP);
            }
            positions(&graph)
        };
        assert_eq!(build(false), build(true));
    }
}
