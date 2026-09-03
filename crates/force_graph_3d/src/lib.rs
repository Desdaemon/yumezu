//! A 3D force-directed graph simulation.
//!
//! Derived from the 2D [`force_graph`](https://github.com/t-mw/force-graph-rs) crate, which in
//! turn implements the algorithm from [Graphoon](https://github.com/rm-code/Graphoon/).
//!
//! # Layout
//!
//! Node state lives in parallel `Vec<f32>` arrays (struct-of-arrays), one per component, rather
//! than inside the graph nodes. The O(n²) repulsion pass is therefore a flat, branch-free scan
//! over contiguous slices that LLVM can auto-vectorize. Two consequences worth knowing:
//!
//! - The pass computes every pair twice instead of applying Newton's third law once. That doubles
//!   the arithmetic but removes the scattered write to the second node, which is what blocks
//!   vectorization. A 4- or 8-wide inner loop wins that trade back several times over.
//! - Forces are gathered from one consistent snapshot and integrated afterwards (Jacobi), where
//!   the original integrated each node inside the pair loop (Gauss-Seidel). Results no longer
//!   depend on node order.
//!
//! # Repulsion cost
//!
//! Charge repulsion is all-to-all, so summing it exactly is O(n²). By default a Barnes-Hut
//! octree replaces distant groups of nodes with their center of mass, which brings the pass down
//! to O(n log n); [`SimulationParameters::theta`] controls the tradeoff, and `0.0` restores exact
//! summation. Measured on one core, 4000 nodes: 10.2 ms per step exact, 2.2 ms at the default
//! angle, for a worst-case force error near 1%.
//!
//! # Settling
//!
//! A layout that has come to rest stops stepping until something disturbs it, so an idle graph
//! costs nothing per frame. [`ForceGraph::is_settled`] reports it, every method that can
//! invalidate it wakes it, and [`SimulationParameters::settle_speed`] sets how still is still
//! enough. Not every graph reaches equilibrium, so
//! [`SimulationParameters::settle_after`] settles one that has not by a deadline.
//!
//! # Modes
//!
//! [`SimulationParameters::dimensions`] and [`SimulationParameters::dag_level_distance`] can
//! both be changed between any two steps. Neither alters the force passes: each one constrains
//! one axis after the forces have run, pulling it toward a plane or a depth layer, so a switch
//! reads as the graph settling into the new arrangement.
//!
//! On wasm the vector loops need `-C target-feature=+simd128`; see `.cargo/config.toml`.
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

use octree::{Interactions, Octree};
use petgraph::{
    stable_graph::{NodeIndex, StableUnGraph},
    visit::{EdgeRef, IntoEdgeReferences},
};

pub type DefaultNodeIdx = NodeIndex<petgraph::stable_graph::DefaultIx>;

/// Added to every squared distance so that a pair at zero distance yields a finite force instead
/// of a NaN. The pair's direction vector is zero there, so the resulting force is still zero -
/// this only replaces the branch the scalar version needed.
const SOFTENING: f32 = 1e-6;

/// Number of lanes the repulsion loop accumulates into.
///
/// Eight `f32` lanes fill an AVX2 register and are a whole multiple of the four that wasm
/// `simd128`, SSE and NEON provide, so the same loop vectorizes on all of them.
const LANES: usize = 8;

/// Seconds of continued disturbance needed to bring the damping all the way back.
///
/// Waking cannot simply restore the damping: settling by withdrawing it leaves the forces
/// untouched, so a layout stopped short of equilibrium is only being held still, and handing the
/// damping back in one step releases every bit of that at once. A nudge then lurches the whole
/// graph. Recovering over a window instead makes the response proportional to how long the
/// layout is actually being disturbed, and this is short enough that a drag becomes responsive
/// well inside the time it takes to make one.
const WAKE_TIME: f32 = 0.5;

/// Rate at which a constrained axis closes the gap to its target, per second.
///
/// A constraint moves a node by a fixed fraction of the remaining distance each step, so it
/// converges without overshoot at any frame time, unlike a spring stiff enough to look instant.
/// At this rate the visible part of a mode change plays out over roughly half a second.
const CONSTRAINT_RATE: f32 = 8.0;

/// Distance below which a constrained axis is set to its target exactly.
///
/// Geometric decay only approaches the target, and [`Dimensions::Two`] is expected to yield
/// coordinates that are actually planar. Simulation units are pixel-sized, so this is invisible.
const CONSTRAINT_SNAP: f32 = 1e-2;

/// Which axes the layout may use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Dimensions {
    /// Layout on the `z = 0` plane. Repulsion and springs still run in 3D, so switching at
    /// runtime collapses the graph onto the plane rather than flattening it in one step.
    Two,
    #[default]
    Three,
}

/// Parameters to control the simulation of the force graph.
#[derive(Clone, Debug)]
pub struct SimulationParameters {
    pub force_charge: f32,
    pub force_spring: f32,
    pub force_max: f32,
    pub node_speed: f32,
    pub damping_factor: f32,
    /// Barnes-Hut opening angle.
    ///
    /// A group of distant nodes is replaced by its center of mass once the group's width
    /// subtends less than this angle, which turns the O(n²) repulsion pass into O(n log n).
    /// Larger values approximate more aggressively; `0.0` disables the approximation and sums
    /// every pair exactly.
    pub theta: f32,
    /// Whether the layout is free to use the z axis.
    pub dimensions: Dimensions,
    /// Speed below which the layout counts as settled, in position units per second.
    ///
    /// A settled layout stops stepping until something disturbs it, which is both cheaper and
    /// steadier than integrating motion too small to see. The default is a tenth of a position
    /// unit per frame at 60 Hz. `0.0` settles only a layout that is exactly at rest.
    pub settle_speed: f32,
    /// Simulated seconds over which the layout is brought to a standstill, or `None` to leave
    /// the damping alone and wait for [`SimulationParameters::settle_speed`] however long that
    /// takes.
    ///
    /// Whether these forces reach equilibrium at all depends on the graph: a sparse one large
    /// enough for repulsion to outweigh its springs expands without ever converging. Rather
    /// than cut such a layout off mid-flight, [`SimulationParameters::damping_factor`] is
    /// ramped to zero across this window, which bleeds off the motion and lets the layout coast
    /// to the rest it would not have reached on its own. Disturbing the layout hands the damping
    /// back over [`WAKE_TIME`] rather than at once, so the response is proportional to how long
    /// the layout is actually being handled.
    pub settle_after: Option<f32>,
    /// Layered ("DAG") mode: the spacing between depth layers along y, or `None` to let the
    /// forces place nodes freely.
    ///
    /// Each node is pinned to the layer named by its [`NodeData::level`], so the y axis reads as
    /// depth and the forces only spread the nodes within a layer.
    pub dag_level_distance: Option<f32>,
    /// Layered mode: how far a node may sit from its layer along y, in position units, or `0.0`
    /// to pin it to the layer exactly.
    ///
    /// Inside the band the axis is free, so the forces stack a crowded layer across its thickness
    /// instead of squeezing the whole of it onto one line. Meant for [`Dimensions::Two`], where a
    /// layer is a line rather than a plane and has no third axis to take the overflow. Read only
    /// while [`SimulationParameters::dag_level_distance`] is set, and best kept well under it, or
    /// neighbouring layers meet and stop reading as layers.
    ///
    /// The edge of the band is soft: it is the same geometric decay that pins a layer without
    /// slack, at [`CONSTRAINT_RATE`], so a node the forces push outward hard enough settles a
    /// little past the band rather than exactly on it.
    pub dag_level_slack: f32,
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
            dag_level_distance: Some(800.0),
            dag_level_slack: 0.0,
        }
    }
}

/// Stores data associated with a node that can be modified by the user.
pub struct NodeData<UserNodeData = ()> {
    /// The horizontal position of the node.
    pub x: f32,
    /// The vertical position of the node.
    pub y: f32,
    /// The depth position of the node.
    pub z: f32,
    /// Which depth layer the node belongs to, in layered mode.
    ///
    /// Read only while [`SimulationParameters::dag_level_distance`] is set, which turns it into
    /// a y coordinate. What a layer counts is the caller's to decide.
    pub level: f32,
    /// The mass of the node.
    ///
    /// Increasing the mass of a node increases the force with which it repels other nearby nodes.
    pub mass: f32,
    /// Whether the node is fixed to its current position.
    pub is_anchor: bool,
    /// Arbitrary user data.
    ///
    /// Defaults to `()` if not specified.
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

/// Stores data associated with an edge that can be modified by the user.
pub struct EdgeData<UserEdgeData = ()> {
    /// Arbitrary user data.
    ///
    /// Defaults to `()` if not specified.
    pub user_data: UserEdgeData,
}

impl<UserEdgeData> Default for EdgeData<UserEdgeData>
where
    UserEdgeData: Default,
{
    fn default() -> Self {
        EdgeData {
            user_data: Default::default(),
        }
    }
}

/// Node state, one array per component.
///
/// Every array is indexed by [`DefaultNodeIdx::index`], so a slot outlives the node that owned it
/// until [`StableUnGraph`] hands the index out again. A free slot carries `mass == 0.0` and
/// `mobility == 0.0`, which makes it exert no force and receive no motion, so the hot loops can
/// scan the whole array without testing whether a slot is live.
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
    /// Depth layer per node, as handed over in [`NodeData::level`]. Only layered mode reads it.
    level: Vec<f32>,
}

impl NodeStore {
    fn len(&self) -> usize {
        self.x.len()
    }

    /// Writes a node into `slot`, growing the arrays if the slot is past the end.
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

    /// Brings every node to rest, so that a settled layout resumes from rest rather than from
    /// the residual drift it settled with.
    fn halt(&mut self) {
        self.vx.fill(0.0);
        self.vy.fill(0.0);
        self.vz.fill(0.0);
    }

    /// Speed of the fastest node, squared.
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

/// Whether the layout is at rest, and how much of the damping is currently in play.
///
/// One value rather than separate fields, because waking is one operation on all of it: every
/// method that can disturb the layout has to record that it did, and none of them should have to
/// know how the damping is being managed.
struct Rest {
    settled: bool,
    /// Fraction of [`SimulationParameters::damping_factor`] in effect: withdrawn across the
    /// settling window while the layout is left alone, restored while it is being disturbed.
    liveliness: f32,
    /// Whether anything disturbed the layout since the last step.
    disturbed: bool,
}

impl Default for Rest {
    fn default() -> Self {
        // A new graph is fully in motion; nothing has had a chance to settle it yet.
        Rest {
            settled: false,
            liveliness: 1.0,
            disturbed: false,
        }
    }
}

impl Rest {
    fn wake(&mut self) {
        self.settled = false;
        self.disturbed = true;
    }

    /// Hands the damping back at once and starts the settling window over.
    fn revive(&mut self) {
        *self = Rest::default();
    }

    /// Advances the damping envelope by one step and returns the fraction now in effect.
    ///
    /// Rises while the layout is being disturbed and falls when it is not, so a brief touch
    /// gives a brief response and only sustained handling brings the layout fully back to life.
    fn advance(&mut self, dt: f32, settle_after: Option<f32>) -> f32 {
        let Some(window) = settle_after else {
            self.liveliness = 1.0;
            return 1.0;
        };
        let step = if std::mem::take(&mut self.disturbed) {
            dt / WAKE_TIME
        } else {
            -dt / window.max(f32::MIN_POSITIVE)
        };
        self.liveliness = (self.liveliness + step).clamp(0.0, 1.0);
        // Cubed, so the damping holds near full for most of the window and is withdrawn at the
        // end of it: the layout spends its time laying out rather than braking.
        let withdrawn = 1.0 - self.liveliness;
        1.0 - withdrawn * withdrawn * withdrawn
    }
}

/// The main force graph structure.
pub struct ForceGraph<UserNodeData = (), UserEdgeData = ()> {
    parameters: SimulationParameters,
    /// Every path that can disturb the layout wakes this, which is why
    /// [`ForceGraph::parameters`] is behind an accessor.
    rest: Rest,
    graph: StableUnGraph<UserNodeData, EdgeData<UserEdgeData>>,
    nodes: NodeStore,
    /// Rebuilt every step by the approximate pass; kept to reuse its allocations.
    tree: Octree,
    interactions: Interactions,
    /// `force_max` per node, the uniform interaction limit the exact pass hands the kernel.
    limits: Vec<f32>,
    /// Target coordinate per node for whichever axis a constraint is closing on. Refilled per
    /// constraint; kept to reuse its allocation.
    targets: Vec<f32>,
}

impl<UserNodeData, UserEdgeData> ForceGraph<UserNodeData, UserEdgeData> {
    /// Constructs a new force graph.
    ///
    /// Use the following syntax to create a graph with default parameters:
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
        }
    }

    pub fn parameters(&self) -> &SimulationParameters {
        &self.parameters
    }

    /// Borrows the parameters for modification, and wakes the layout.
    ///
    /// Changing a parameter invalidates the rest the layout had come to, and the graph has no
    /// other way to notice; this is why the parameters are not a public field.
    pub fn parameters_mut(&mut self) -> &mut SimulationParameters {
        self.rest.wake();
        &mut self.parameters
    }

    /// Starts the settling window over, as though the layout had just been built.
    ///
    /// [`SimulationParameters::settle_after`] withdraws the damping to bring a layout to rest,
    /// and a woken layout only gets it back over [`WAKE_TIME`] for as long as it is actually
    /// being disturbed - which is right for a drag, and far too little for a layout rearranged
    /// wholesale. A caller that has changed the arrangement itself, rather than nudged it,
    /// should say so here: there is no equilibrium left to release gently, and the layout needs
    /// a full window to find its new shape at the speed a fresh one would.
    pub fn revive(&mut self) {
        self.rest.revive();
    }

    /// Whether the layout has come to rest, leaving [`ForceGraph::update`] with nothing to do.
    ///
    /// A caller that rebuilds geometry from the node positions can skip that work while this
    /// holds. It is cleared by anything that disturbs the layout: a force, a moved or altered
    /// node, a new or removed node or edge, and a parameter change.
    pub fn is_settled(&self) -> bool {
        self.rest.settled
    }

    /// Provides access to the raw graph structure if required.
    ///
    /// The graph holds the topology and the user data; node positions live in the arrays behind
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

    /// Adds a new node and returns an index that can be used to reference the node.
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

    /// Removes a node by index.
    pub fn remove_node(&mut self, idx: DefaultNodeIdx) {
        if self.graph.remove_node(idx).is_some() {
            self.nodes.release(idx.index());
            self.rest.wake();
        }
    }

    /// Adds or updates an edge connecting two nodes by index.
    pub fn add_edge(
        &mut self,
        n1_idx: DefaultNodeIdx,
        n2_idx: DefaultNodeIdx,
        edge: EdgeData<UserEdgeData>,
    ) {
        self.graph.update_edge(n1_idx, n2_idx, edge);
        self.rest.wake();
    }

    /// Removes all nodes from the force graph.
    pub fn clear(&mut self) {
        self.graph.clear();
        self.nodes.clear();
        self.rest.wake();
    }

    /// Applies the next step of the force graph simulation.
    ///
    /// The number of seconds that have elapsed since the previous update must be calculated and
    /// provided by the user as `dt`.
    /// A settled layout - see [`ForceGraph::is_settled`] - does nothing until something wakes
    /// it, so calling this every frame costs nothing once the graph has come to rest.
    pub fn update(&mut self, dt: f32) {
        if self.rest.settled || self.graph.node_count() == 0 {
            return;
        }
        let damping =
            self.parameters.damping_factor * self.rest.advance(dt, self.parameters.settle_after);
        self.repel();
        self.attract();
        // A constrained axis is driven by `constrain` alone. Integrating it as well would let
        // the forces push the node off the constraint every step, and the constraint would only
        // ever pull most of that back: the axis would hover near its target instead of reaching
        // it, and two-dimensional mode would never be exactly planar. A slack layer is the
        // exception: its constraint is a band rather than a point, and the forces are what fill
        // the band, so the axis has to be integrated for the slack to be worth anything.
        let free = [
            true,
            self.parameters.dag_level_distance.is_none() || self.parameters.dag_level_slack > 0.0,
            self.parameters.dimensions == Dimensions::Three,
        ];
        self.integrate(dt, damping, free);
        let constrained = self.constrain(dt);
        // Cleared at the end, not the start, so a force applied between two steps survives
        // until the step that consumes it.
        self.nodes.clear_forces();

        // A constraint moves a node without giving it any velocity, so the speed test cannot
        // see that motion: a graph still collapsing onto the plane or into its layers has to
        // stay awake however slowly it is moving.
        let settle_speed = self.parameters.settle_speed;
        self.rest.settled =
            constrained && self.nodes.fastest_speed_sqrd() <= settle_speed * settle_speed;
        if self.rest.settled {
            self.nodes.halt();
        }
    }

    /// Adds a force to one node, to be applied by the next [`ForceGraph::update`].
    ///
    /// Repeated calls accumulate, and one update consumes the total. The force is added to the
    /// charge and spring forces without passing through
    /// [`SimulationParameters::force_max`], so a caller dragging a node is not competing with
    /// the clamp; an anchored node ignores it, as it ignores every other force.
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

    /// Every pair, summed exactly. O(n²), and the reference the approximate pass is tested
    /// against.
    fn repel_exact(&mut self) {
        let n = self.nodes.len();
        let force_max = self.parameters.force_max;
        let charge = self.parameters.force_charge;
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
        // Reslicing to one common length lets the bounds checks fold away.
        let (x, y, z, mass) = (&x[..n], &y[..n], &z[..n], &mass[..n]);
        let (ax, ay, az) = (&mut ax[..n], &mut ay[..n], &mut az[..n]);
        // Every entry is a single node here, so every entry gets one pair's worth of headroom.
        self.limits.clear();
        self.limits.resize(n, force_max);

        for i in 0..n {
            // A free slot has zero mass, so it contributes nothing in either direction.
            let [fx, fy, fz] = repulsion_on(
                [x[i], y[i], z[i]],
                -charge * mass[i],
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

    /// Barnes-Hut: walk the octree once per leaf to collect an interaction list of nearby bodies
    /// and distant cell aggregates, then run the same kernel over that list for each of the
    /// leaf's nodes.
    fn repel_approx(&mut self) {
        let n = self.nodes.len();
        let force_max = self.parameters.force_max;
        let charge = self.parameters.force_charge;
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

        self.tree.build(x, y, z, mass);
        for cell in 0..self.tree.cell_count() {
            let Some(bodies) = self.tree.leaf_bodies(cell) else {
                continue;
            };
            let mut lo = [f32::INFINITY; 3];
            let mut hi = [f32::NEG_INFINITY; 3];
            for &body in bodies {
                let body = body as usize;
                for (axis, value) in [x[body], y[body], z[body]].into_iter().enumerate() {
                    lo[axis] = lo[axis].min(value);
                    hi[axis] = hi[axis].max(value);
                }
            }
            self.tree.gather(
                cell,
                lo,
                hi,
                theta,
                force_max,
                &mut self.interactions,
                x,
                y,
                z,
                mass,
            );

            let list = &self.interactions;
            for &body in bodies {
                let body = body as usize;
                let [fx, fy, fz] = repulsion_on(
                    [x[body], y[body], z[body]],
                    -charge * mass[body],
                    &list.x,
                    &list.y,
                    &list.z,
                    &list.mass,
                    &list.limit,
                );
                ax[body] += fx;
                ay[body] += fy;
                az[body] += fz;
            }
        }
    }

    /// Spring attraction along the edges.
    ///
    /// The spring force is `spring * distance * 0.5` along the unit vector between the nodes,
    /// which is just the offset vector scaled: no square root needed.
    fn attract(&mut self) {
        let strength = self.parameters.force_spring * 0.5;
        let force_max = self.parameters.force_max;
        let nodes = &mut self.nodes;
        for edge in self.graph.edge_references() {
            let (i, j) = (edge.source().index(), edge.target().index());
            let fx = ((nodes.x[j] - nodes.x[i]) * strength)
                .max(-force_max)
                .min(force_max);
            let fy = ((nodes.y[j] - nodes.y[i]) * strength)
                .max(-force_max)
                .min(force_max);
            let fz = ((nodes.z[j] - nodes.z[i]) * strength)
                .max(-force_max)
                .min(force_max);
            nodes.ax[i] += fx;
            nodes.ay[i] += fy;
            nodes.az[i] += fz;
            nodes.ax[j] -= fx;
            nodes.ay[j] -= fy;
            nodes.az[j] -= fz;
        }
    }

    /// Integrates the accumulated forces. `damping` is what the settling envelope has left of
    /// [`SimulationParameters::damping_factor`]: the forces push regardless, so what stops the
    /// layout is how much of the velocity they build it gets to keep.
    fn integrate(&mut self, dt: f32, damping: f32, free: [bool; 3]) {
        // The accumulators hold force; one `dt` turns it into a velocity change and the other
        // is the step it acts over.
        let dv = dt * dt * self.parameters.node_speed;
        let n = self.nodes.len();
        let mobility = &self.nodes.mobility[..n];
        // One axis at a time: fewer live streams per loop, so the vector registers hold the whole
        // working set.
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

    /// Applies the axis constraints the current mode asks for, after the forces have moved the
    /// nodes freely.
    ///
    /// Both modes are the same operation on a different axis and target, so a runtime switch
    /// only changes which constraint runs; the forces themselves stay 3D throughout.
    /// Returns whether every constraint has reached its target, which is the other half of
    /// deciding that the layout has settled.
    fn constrain(&mut self, dt: f32) -> bool {
        let n = self.nodes.len();
        let mut reached = true;
        // Fraction of the gap a constraint leaves behind this step.
        let retained = (-CONSTRAINT_RATE * dt).exp();

        if let Some(distance) = self.parameters.dag_level_distance {
            self.targets.clear();
            self.targets
                .extend(self.nodes.level[..n].iter().map(|level| level * distance));
            reached &= constrain_axis(
                &mut self.nodes.y[..n],
                &mut self.nodes.vy[..n],
                &self.nodes.mobility[..n],
                &self.targets,
                self.parameters.dag_level_slack.max(0.0),
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

        reached
    }

    /// Borrows one node by index, or `None` if it has been removed.
    pub fn node(&self, idx: DefaultNodeIdx) -> Option<NodeRef<'_, UserNodeData>> {
        Some(NodeRef {
            index: idx,
            nodes: &self.nodes,
            user_data: self.graph.node_weight(idx)?,
        })
    }

    /// Processes each node with a user-defined callback `cb`.
    pub fn visit_nodes<F: FnMut(NodeRef<'_, UserNodeData>)>(&self, mut cb: F) {
        for idx in self.graph.node_indices() {
            cb(NodeRef {
                index: idx,
                nodes: &self.nodes,
                user_data: &self.graph[idx],
            });
        }
    }

    /// Mutates each node with a user-defined callback `cb`.
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

    /// Processes each edge and its associated nodes with a user-defined callback `cb`.
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

/// Borrows one node of a [`ForceGraph`]. Can not be constructed by the user.
pub struct NodeRef<'a, UserNodeData = ()> {
    /// The node data provided by the user.
    pub user_data: &'a UserNodeData,
    index: DefaultNodeIdx,
    nodes: &'a NodeStore,
}

impl<UserNodeData> NodeRef<'_, UserNodeData> {
    /// The horizontal position of the node.
    pub fn x(&self) -> f32 {
        self.nodes.x[self.index.index()]
    }

    /// The vertical position of the node.
    pub fn y(&self) -> f32 {
        self.nodes.y[self.index.index()]
    }

    /// The depth position of the node.
    pub fn z(&self) -> f32 {
        self.nodes.z[self.index.index()]
    }

    /// The position of the node.
    pub fn position(&self) -> [f32; 3] {
        [self.x(), self.y(), self.z()]
    }

    pub fn mass(&self) -> f32 {
        self.nodes.mass[self.index.index()]
    }

    pub fn is_anchor(&self) -> bool {
        self.nodes.mobility[self.index.index()] == 0.0
    }

    /// The index used to reference the node in the [ForceGraph].
    pub fn index(&self) -> DefaultNodeIdx {
        self.index
    }
}

/// Mutably borrows one node of a [`ForceGraph`]. Can not be constructed by the user.
pub struct NodeMut<'a, UserNodeData = ()> {
    /// The node data provided by the user.
    pub user_data: &'a mut UserNodeData,
    slot: usize,
    nodes: &'a mut NodeStore,
    /// Woken by every setter: each one invalidates the rest the layout had come to.
    rest: &'a mut Rest,
}

impl<UserNodeData> NodeMut<'_, UserNodeData> {
    pub fn position(&self) -> [f32; 3] {
        [
            self.nodes.x[self.slot],
            self.nodes.y[self.slot],
            self.nodes.z[self.slot],
        ]
    }

    /// Moves the node, leaving its velocity alone.
    pub fn set_position(&mut self, [x, y, z]: [f32; 3]) {
        self.rest.wake();
        self.nodes.x[self.slot] = x;
        self.nodes.y[self.slot] = y;
        self.nodes.z[self.slot] = z;
    }

    /// Sets the velocity of the node, leaving its position alone.
    ///
    /// Zeroing it stops the node dead, which is what restarting a layout needs: a node moved to
    /// a new position keeps whatever momentum it had at the old one otherwise.
    pub fn set_velocity(&mut self, [vx, vy, vz]: [f32; 3]) {
        self.rest.wake();
        self.nodes.vx[self.slot] = vx;
        self.nodes.vy[self.slot] = vy;
        self.nodes.vz[self.slot] = vz;
    }

    pub fn set_mass(&mut self, mass: f32) {
        self.rest.wake();
        self.nodes.mass[self.slot] = mass;
    }

    /// Adds a force to the node, to be applied by the next [`ForceGraph::update`].
    ///
    /// See [`ForceGraph::apply_force`].
    pub fn apply_force(&mut self, force: [f32; 3]) {
        self.rest.wake();
        self.nodes.add_force(self.slot, force);
    }

    pub fn set_anchor(&mut self, is_anchor: bool) {
        self.rest.wake();
        self.nodes.mobility[self.slot] = if is_anchor { 0.0 } else { 1.0 };
    }
}

/// Repulsion exerted on one node by a list of masses.
///
/// `charge_target` folds in the target's own mass and the sign of the charge. `limit` caps what
/// each entry may contribute per component: `force_max` for a single node, and a multiple of it
/// for a cell aggregate that stands in for several. The list may be every node in the graph, or
/// the interaction list of an octree walk; the kernel does not care, and a list entry at the
/// target's own position contributes nothing.
/// Inlined on purpose: at a call site the slice lengths and their disjointness are visible, and
/// without that LLVM leaves the lane loop scalar. Checked by inspecting the emitted wasm IR.
#[inline]
fn repulsion_on(
    target: [f32; 3],
    charge_target: f32,
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
    // One accumulator per lane. A single `f32` running total would be a reduction that LLVM may
    // not reorder, and it refuses to vectorize the loop rather than change the summation order;
    // per-lane totals summed at the end make the order explicit.
    let mut fx = [0.0f32; LANES];
    let mut fy = [0.0f32; LANES];
    let mut fz = [0.0f32; LANES];

    let body = n - n % LANES;
    for base in (0..body).step_by(LANES) {
        // Fixed-size views: the length is known at compile time, so no bounds check and no
        // scalar epilogue inside the chunk.
        let (xs, ys, zs, ms, ls) = (
            lane(x, base),
            lane(y, base),
            lane(z, base),
            lane(mass, base),
            lane(limit, base),
        );
        for l in 0..LANES {
            let dx = xs[l] - tx;
            let dy = ys[l] - ty;
            let dz = zs[l] - tz;
            // Repulsion falls off with the square of the distance, and the direction vector
            // needs one more division to normalize: three inverse distances.
            let inv = (dx * dx + dy * dy + dz * dz + SOFTENING).sqrt().recip();
            let strength = charge_target * ms[l] * inv * inv * inv;
            // Clamped per interaction, as the scalar version clamped per pair, so one close
            // neighbour cannot dominate the sum. `min`/`max` stay branch-free.
            fx[l] += (dx * strength).max(-ls[l]).min(ls[l]);
            fy[l] += (dy * strength).max(-ls[l]).min(ls[l]);
            fz[l] += (dz * strength).max(-ls[l]).min(ls[l]);
        }
    }
    for j in body..n {
        let dx = x[j] - tx;
        let dy = y[j] - ty;
        let dz = z[j] - tz;
        let inv = (dx * dx + dy * dy + dz * dz + SOFTENING).sqrt().recip();
        let strength = charge_target * mass[j] * inv * inv * inv;
        let (lo, hi) = (-limit[j], limit[j]);
        fx[0] += (dx * strength).max(lo).min(hi);
        fy[0] += (dy * strength).max(lo).min(hi);
        fz[0] += (dz * strength).max(lo).min(hi);
    }

    [reduce(fx), reduce(fy), reduce(fz)]
}

/// Borrows `LANES` values starting at `base` as a fixed-size array.
fn lane(values: &[f32], base: usize) -> &[f32; LANES] {
    values[base..base + LANES].try_into().unwrap()
}

fn reduce(lanes: [f32; LANES]) -> f32 {
    lanes.iter().sum()
}

/// Relaxes one axis toward a target coordinate, discarding whatever the forces did to it.
///
/// The gap closes geometrically rather than through a spring: it converges at a rate set only by
/// the elapsed time, and it cannot overshoot however large the step or the distance.
/// Returns whether every node it may move is now exactly on its target.
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
        // An anchor - and a free slot - holds its coordinate against the constraint too.
        if mobility[i] == 0.0 {
            continue;
        }
        // The band the target names. Without slack it is the target itself, so a node is inside
        // it only once it has arrived.
        let target = pos[i].clamp(targets[i] - slack, targets[i] + slack);
        if pos[i] == target {
            // Inside the band the axis belongs to the forces: leave the velocity alone, or the
            // constraint would bleed off the motion that does the stacking.
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
                user_data: UserEdgeData {},
            },
        );
    }

    /// Repulsion has to push along all three axes, not just the two the original supported.
    #[test]
    fn repulsion_separates_on_every_axis() {
        // Every axis has to be free for the forces to be visible on it.
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

        graph.update(0.01);
        let [a, b] = positions(&graph)[..] else {
            unreachable!()
        };
        for axis in 0..3 {
            assert!(a[axis] < -1.0, "axis {axis} not repelled: {a:?}");
            assert!(b[axis] > 1.0, "axis {axis} not repelled: {b:?}");
        }
    }

    /// A spring pulls its two nodes together once repulsion is out of the way.
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

        graph.update(0.01);
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
            graph.update(0.01);
        }
        assert_eq!(positions(&graph)[0], [0.0; 3]);
    }

    /// Coincident nodes used to need a zero-distance branch; softening replaces it and must not
    /// leak a NaN into the positions.
    #[test]
    fn coincident_nodes_stay_finite() {
        let mut graph = <ForceGraph>::new(Default::default());
        graph.add_node(NodeData::default());
        graph.add_node(NodeData::default());

        graph.update(0.01);
        for position in positions(&graph) {
            assert!(position.iter().all(|c| c.is_finite()), "{position:?}");
        }
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
                graph.update(0.01);
            }
            positions(&graph)
        };
        assert_eq!(layout(false), layout(true));
    }
    /// Displacement after one step from rest is proportional to the force on the node, so it
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
        graph.update(0.016);
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

    /// A vanishing opening angle descends to the leaves, so the walk must reproduce the exact
    /// pass up to the order the lanes sum in.
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

    /// The default opening angle has to stay accurate enough that the layout is
    /// indistinguishable. Measured at ~1% worst case, and the bound must hold at any size: an
    /// aggregate that mis-clamps its share of the force makes this grow with node count.
    #[test]
    fn default_theta_stays_accurate() {
        let exact = one_step_displacements(0.0);
        let approx = one_step_displacements(SimulationParameters::default().theta);
        let error = worst_relative_error(&exact, &approx);
        assert!(error < 0.02, "{error}");
    }

    /// Switching to two dimensions has to end with coordinates that are exactly planar, not
    /// merely small, and it must not flatten the other axes with them.
    #[test]
    fn two_dimensions_collapse_onto_the_plane() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dimensions: Dimensions::Two,
            // Only the plane constraint is under test: layering would pin y as well.
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
            graph.update(0.016);
        }
        let positions = positions(&graph);
        assert!(positions.iter().all(|p| p[2] == 0.0), "{positions:?}");
        let spread = |axis: usize| {
            let values = positions.iter().map(|p| p[axis]);
            values.clone().fold(f32::MIN, f32::max) - values.fold(f32::MAX, f32::min)
        };
        assert!(spread(0) > 100.0 && spread(1) > 100.0, "{positions:?}");
    }

    /// Parameters that settle only once the layout has genuinely come to rest, so that a test
    /// of that path is not answered by the deadline instead.
    fn converge() -> SimulationParameters {
        SimulationParameters {
            settle_after: None,
            ..Default::default()
        }
    }

    /// Runs the layout until it settles, and reports how many steps that took.
    fn settle<N, E>(graph: &mut ForceGraph<N, E>, limit: usize) -> usize {
        for step in 0..limit {
            graph.update(1.0 / 60.0);
            if graph.is_settled() {
                return step + 1;
            }
        }
        panic!("did not settle in {limit} steps");
    }

    /// A settled layout has to hold still: the point of settling is that the caller can keep
    /// calling `update` for free.
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
            graph.update(1.0 / 60.0);
        }
        assert_eq!(positions(&graph), resting);
    }

    /// Grabbing a node has to bring the layout back to life, and the node has to move.
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

        graph.update(1.0 / 60.0);
        assert!(positions(&graph)[0][0] > resting[0][0]);
    }

    /// A graph whose repulsion outweighs its springs expands for as long as it is stepped, so
    /// withdrawing the damping is the only thing that will ever settle it.
    #[test]
    fn an_unconverged_layout_is_brought_to_rest() {
        let budget = 5.0f32;
        let dt = 1.0 / 60.0;
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

        // Within the window rather than at the end of it: the layout stops as soon as what is
        // left of the damping can no longer carry it, which is before the damping reaches zero.
        let steps = settle(&mut graph, 10_000);
        assert!(steps <= (budget / dt).ceil() as usize, "{steps}");
        let resting = positions(&graph);
        for _ in 0..600 {
            graph.update(dt);
        }
        assert_eq!(positions(&graph), resting);
    }

    /// Settling by withdrawing the damping leaves the forces in place, so handing it all back
    /// at the first touch would let one frame of contact release the whole layout. The response
    /// has to be proportional to how long the layout is actually handled.
    #[test]
    fn a_brief_touch_gives_a_brief_response() {
        let dt = 1.0 / 60.0;
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
            // Total distance the layout moved, the grabbed node excluded: what is being measured
            // is how much of the stored expansion the touch released.
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

    /// The window runs from the last disturbance, so a node held for longer than the window
    /// keeps the layout in motion, and letting go grants a full window rather than what was
    /// left of one.
    #[test]
    fn the_settling_window_runs_from_the_last_disturbance() {
        let budget = 5.0f32;
        let dt = 1.0 / 60.0;
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

        // Held for twice the budget: a force every step means it never gets to settle.
        for _ in 0..steps * 2 {
            graph.apply_force(held, [50.0, 0.0, 0.0]);
            graph.update(dt);
            assert!(!graph.is_settled());
        }
        // Let go, then disturb it once more so the window starts from zero here rather than
        // from the step above.
        graph.apply_force(held, [50.0, 0.0, 0.0]);
        assert!(settle(&mut graph, 10_000) <= steps);
    }

    /// Changing a parameter invalidates the rest the layout had reached, and the graph cannot
    /// see the change any other way.
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

    /// A constraint moves nodes without giving them any velocity, so a graph mid-collapse must
    /// not be mistaken for a settled one and frozen halfway.
    #[test]
    fn a_collapsing_layout_is_not_settled() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dimensions: Dimensions::Two,
            // Nothing but the constraint moves this graph, so speed alone would settle it at
            // the first step.
            force_charge: 0.0,
            ..converge()
        });
        graph.add_node(NodeData {
            z: 1000.0,
            ..Default::default()
        });

        graph.update(1.0 / 60.0);
        assert!(!graph.is_settled(), "settled while still off the plane");
        settle(&mut graph, 10_000);
        assert_eq!(positions(&graph)[0][2], 0.0);
    }

    /// Repulsion acts along the offset between two nodes, so nodes sharing a coordinate get no
    /// force along that axis at all. A caller switching back to three dimensions has to reseed
    /// the axis rather than expect the forces to reinflate a flat layout.
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
            graph.update(0.016);
        }
        let positions = positions(&graph);
        assert!(positions.iter().all(|p| p[2] == 0.0), "{positions:?}");
    }

    /// Slack gives a layer thickness: the nodes on it spread across the band rather than pile
    /// onto its line, and none of them leaves the band.
    #[test]
    fn slack_stacks_a_crowded_layer() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dag_level_distance: Some(1000.0),
            dag_level_slack: 200.0,
            dimensions: Dimensions::Two,
            ..Default::default()
        });
        for i in 0..8 {
            graph.add_node(NodeData {
                x: i as f32 * 10.0,
                // Repulsion acts along the offset between two nodes, so a column of nodes sharing
                // a y has no y force to spread it: the band is filled from a seed, not from
                // nothing. The app scatters its layout for the same reason.
                y: 1000.0 + (i % 4) as f32,
                level: 1.0,
                ..Default::default()
            });
        }

        for _ in 0..120 {
            graph.update(0.016);
        }
        let ys: Vec<f32> = positions(&graph).iter().map(|p| p[1]).collect();
        // The band is a soft edge, not a wall: a node under load sits a little past it. See
        // [`SimulationParameters::dag_level_slack`].
        assert!(ys.iter().all(|y| (y - 1000.0).abs() <= 220.0), "{ys:?}");
        let spread = ys.iter().fold(0.0f32, |a, y| a.max((y - 1000.0).abs()));
        assert!(spread > 1.0, "{ys:?}");
    }

    /// Layered mode pins each node to the layer its level names, however the forces would rather
    /// place it.
    #[test]
    fn layers_follow_the_level() {
        let mut graph = <ForceGraph>::new(SimulationParameters {
            dag_level_distance: Some(100.0),
            ..Default::default()
        });
        let chain: Vec<_> = (0..4)
            .map(|i| {
                graph.add_node(NodeData {
                    x: i as f32 * 10.0,
                    // Deliberately not the y the layer asks for: the constraint has to place it.
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
            graph.update(0.016);
        }
        let layers: Vec<f32> = positions(&graph).iter().map(|p| p[1]).collect();
        assert_eq!(layers, vec![0.0, 100.0, 200.0, 300.0]);
    }

    /// Withdrawing the damping is what brings a layout to rest, so a settled layout has none
    /// left and [`Rest::wake`] hands back only as much as the disturbance lasts. That is right
    /// for a drag and useless for a rearrangement: asked to give up its layers that way, the
    /// layout barely moves. [`ForceGraph::revive`] is what a caller changing the arrangement
    /// itself needs.
    #[test]
    fn reviving_lets_a_settled_layout_rearrange() {
        /// Runs to rest, bounded so a layout that will not settle fails the test instead of
        /// hanging it.
        fn rest(graph: &mut ForceGraph) {
            for _ in 0..2000 {
                if graph.is_settled() {
                    return;
                }
                graph.update(0.016);
            }
            panic!("never settled");
        }
        // Several nodes to a layer, so that lifting the layering leaves the forces something
        // to do: they spread the layers apart along the axis the constraint was holding.
        fn layered() -> ForceGraph {
            let mut graph = <ForceGraph>::new(SimulationParameters {
                dag_level_distance: Some(100.0),
                ..Default::default()
            });
            let nodes: Vec<_> = (0..24)
                .map(|i| {
                    graph.add_node(NodeData {
                        x: (i % 6) as f32 * 30.0,
                        z: (i / 6) as f32 * 30.0,
                        level: (i / 6) as f32,
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
        /// How far the nodes travel, in total, once the layering is lifted.
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

    /// A force handed to one node moves that node, and one step consumes it: the second step
    /// only carries the leftover velocity.
    #[test]
    fn an_applied_force_is_consumed_by_one_step() {
        let mut graph = <ForceGraph>::new(Default::default());
        let node = graph.add_node(NodeData::default());
        graph.apply_force(node, [30.0, 0.0, 0.0]);

        graph.update(0.016);
        let pushed = positions(&graph)[0][0];
        graph.update(0.016);
        let coasted = positions(&graph)[0][0] - pushed;
        assert!(pushed > 0.0, "{pushed}");
        assert!(coasted > 0.0 && coasted < pushed, "{pushed} {coasted}");
    }

    /// The tree covers live slots only, so a hole in the middle of the arrays must not shift the
    /// bodies around it.
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
                graph.update(0.016);
            }
            positions(&graph)
        };
        assert_eq!(build(false), build(true));
    }
}
