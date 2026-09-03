//! Flat octree behind the Barnes-Hut repulsion pass.
//!
//! Cells live in parallel arrays and refer to each other by index, so a build reuses the
//! allocations from the previous step and a traversal walks indices instead of pointers.

use crate::LANES;

/// Bodies per leaf.
///
/// This is the group a single tree walk serves, so it trades walk count against interaction-list
/// length. Measured on a 4000-node graph at the default opening angle: 8 bodies costs 3.5 ms per
/// step, 32 costs 2.2 ms, 64 costs 2.5 ms. A multiple of [`LANES`] keeps a leaf's bodies filling
/// whole iterations of the repulsion kernel.
const LEAF_CAPACITY: usize = LANES * 4;
/// Coincident bodies would subdivide forever. At this depth the leaf keeps them all instead.
const MAX_DEPTH: u32 = 20;
/// Absent child.
const NIL: u32 = u32::MAX;

#[derive(Default)]
pub(crate) struct Octree {
    /// Center of mass of the cell.
    com_x: Vec<f32>,
    com_y: Vec<f32>,
    com_z: Vec<f32>,
    /// Total mass of the cell.
    mass: Vec<f32>,
    /// Number of bodies in the cell, which is how many pair interactions it stands in for.
    count: Vec<f32>,
    /// Full width of the cell, for the `width / distance < theta` acceptance test.
    width: Vec<f32>,
    children: Vec<[u32; 8]>,
    /// `(start, count)` into `order`, for every cell: a cell owns one contiguous run of bodies.
    /// The traversal needs this for internal cells too, to recognize the ones that enclose the
    /// node it is gathering for.
    bodies: Vec<(u32, u32)>,
    /// Whether the cell stopped subdividing, and so owns its bodies directly.
    is_leaf: Vec<bool>,
    /// Live body slots, ordered so that every leaf owns one contiguous run.
    order: Vec<u32>,
    /// Reused by the octant partition.
    partition: Vec<u32>,
}

impl Octree {
    /// Rebuilds the tree over every slot with mass. A massless slot neither exerts nor feels a
    /// force, so leaving it out also keeps the root cube tight around the live nodes.
    pub fn build(&mut self, x: &[f32], y: &[f32], z: &[f32], mass: &[f32]) {
        self.com_x.clear();
        self.com_y.clear();
        self.com_z.clear();
        self.mass.clear();
        self.count.clear();
        self.width.clear();
        self.children.clear();
        self.bodies.clear();
        self.is_leaf.clear();
        self.order.clear();
        for (slot, &m) in mass.iter().enumerate() {
            if m > 0.0 {
                self.order.push(slot as u32);
            }
        }
        if self.order.is_empty() {
            return;
        }

        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for &body in &self.order {
            let body = body as usize;
            for (axis, value) in [x[body], y[body], z[body]].into_iter().enumerate() {
                lo[axis] = lo[axis].min(value);
                hi[axis] = hi[axis].max(value);
            }
        }
        let center = [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ];
        // A cube, so one width per cell describes it. Never zero: a degenerate cube would make
        // the acceptance test accept every cell at any distance.
        let width = (hi[0] - lo[0])
            .max(hi[1] - lo[1])
            .max(hi[2] - lo[2])
            .max(f32::MIN_POSITIVE);

        // Moved out so the recursion can hold a mutable sub-slice of `order` while it pushes
        // cells into the arrays.
        let mut order = std::mem::take(&mut self.order);
        let mut partition = std::mem::take(&mut self.partition);
        self.build_cell(
            &mut order,
            &mut partition,
            0,
            center,
            width,
            0,
            x,
            y,
            z,
            mass,
        );
        self.order = order;
        self.partition = partition;
    }

    #[allow(clippy::too_many_arguments)]
    fn build_cell(
        &mut self,
        order: &mut [u32],
        partition: &mut Vec<u32>,
        offset: u32,
        center: [f32; 3],
        width: f32,
        depth: u32,
        x: &[f32],
        y: &[f32],
        z: &[f32],
        mass: &[f32],
    ) -> u32 {
        let cell = self.width.len();
        self.width.push(width);
        self.children.push([NIL; 8]);
        self.bodies.push((offset, order.len() as u32));
        self.is_leaf.push(false);

        let (mut total, mut cx, mut cy, mut cz) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for &body in order.iter() {
            let body = body as usize;
            let m = mass[body];
            total += m;
            cx += x[body] * m;
            cy += y[body] * m;
            cz += z[body] * m;
        }
        let inv_total = if total > 0.0 { total.recip() } else { 0.0 };
        self.com_x.push(cx * inv_total);
        self.com_y.push(cy * inv_total);
        self.com_z.push(cz * inv_total);
        self.mass.push(total);
        self.count.push(order.len() as f32);

        if order.len() <= LEAF_CAPACITY || depth >= MAX_DEPTH {
            self.is_leaf[cell] = true;
            return cell as u32;
        }

        let bounds = partition_octants(order, partition, center, x, y, z);
        let quarter = width * 0.25;
        for octant in 0..8 {
            let (start, end) = (bounds[octant] as usize, bounds[octant + 1] as usize);
            if start == end {
                continue;
            }
            let offset_of = |axis: usize| {
                if octant & (1 << axis) != 0 {
                    quarter
                } else {
                    -quarter
                }
            };
            let child = self.build_cell(
                &mut order[start..end],
                partition,
                offset + start as u32,
                [
                    center[0] + offset_of(0),
                    center[1] + offset_of(1),
                    center[2] + offset_of(2),
                ],
                width * 0.5,
                depth + 1,
                x,
                y,
                z,
                mass,
            );
            self.children[cell][octant] = child;
        }
        cell as u32
    }

    pub fn cell_count(&self) -> usize {
        self.width.len()
    }

    /// The bodies of `cell`, if `cell` is a leaf.
    pub fn leaf_bodies(&self, cell: usize) -> Option<&[u32]> {
        let (start, count) = self.bodies[cell];
        self.is_leaf[cell].then(|| &self.order[start as usize..(start + count) as usize])
    }

    /// Collects the bodies and cell aggregates that everything inside the box `lo..hi` interacts
    /// with, together with the force each entry may contribute.
    ///
    /// A cell stands in for its bodies once its width subtends less than `theta` from the whole
    /// box, measured from the nearest corner, so the list is valid for every node in the box.
    /// One walk then serves a leaf's worth of nodes instead of one node.
    ///
    /// `query` is the leaf the box was measured from. Its own bodies must reach the kernel as
    /// bodies, never folded into an aggregate: only an exactly zero offset makes a node's pull
    /// on itself zero, and an aggregate standing in for the node itself sits a rounding error
    /// away from it, which the softened inverse square turns into a full-strength kick.
    #[allow(clippy::too_many_arguments)]
    pub fn gather(
        &self,
        query: usize,
        lo: [f32; 3],
        hi: [f32; 3],
        theta: f32,
        force_max: f32,
        out: &mut Interactions,
        x: &[f32],
        y: &[f32],
        z: &[f32],
        mass: &[f32],
    ) {
        out.clear();
        if self.width.is_empty() {
            return;
        }
        let theta_sqrd = theta * theta;
        let (query_start, _) = self.bodies[query];
        out.stack.push(0);
        while let Some(cell) = out.stack.pop() {
            let cell = cell as usize;
            let com = [self.com_x[cell], self.com_y[cell], self.com_z[cell]];
            // Distance from the center of mass to the box: zero on any axis where it lies
            // between the bounds.
            let mut distance_sqrd = 0.0;
            for axis in 0..3 {
                let gap = (lo[axis] - com[axis]).max(com[axis] - hi[axis]).max(0.0);
                distance_sqrd += gap * gap;
            }
            let width = self.width[cell];
            // The acceptance test cannot recognize an enclosing cell on its own: it measures
            // from the center of mass, which can lie outside the box while the cell encloses
            // it, and a cell of coincident bodies has no width to fail the test with.
            let (start, count) = self.bodies[cell];
            let encloses_query = start <= query_start && query_start < start + count;
            if !encloses_query && width * width < theta_sqrd * distance_sqrd {
                // The aggregate replaces `count` pairs, each of which the exact pass would have
                // clamped to `force_max` on its own, so it carries that many pairs' worth of
                // headroom. Clamping it as if it were a single pair is what makes an aggregate
                // under-report a crowded cell.
                out.push(
                    com[0],
                    com[1],
                    com[2],
                    self.mass[cell],
                    force_max * self.count[cell],
                );
                continue;
            }
            match self.leaf_bodies(cell) {
                Some(bodies) => {
                    for &body in bodies {
                        let body = body as usize;
                        out.push(x[body], y[body], z[body], mass[body], force_max);
                    }
                }
                None => {
                    for &child in &self.children[cell] {
                        if child != NIL {
                            out.stack.push(child);
                        }
                    }
                }
            }
        }
    }
}

/// Sorts `order` into the eight octants of `center` and returns the nine boundaries between them.
fn partition_octants(
    order: &mut [u32],
    scratch: &mut Vec<u32>,
    center: [f32; 3],
    x: &[f32],
    y: &[f32],
    z: &[f32],
) -> [u32; 9] {
    let octant_of = |body: u32| {
        let body = body as usize;
        (x[body] >= center[0]) as usize
            | ((y[body] >= center[1]) as usize) << 1
            | ((z[body] >= center[2]) as usize) << 2
    };

    let mut counts = [0u32; 8];
    for &body in order.iter() {
        counts[octant_of(body)] += 1;
    }
    let mut bounds = [0u32; 9];
    for octant in 0..8 {
        bounds[octant + 1] = bounds[octant] + counts[octant];
    }

    scratch.clear();
    scratch.resize(order.len(), 0);
    let mut cursor = bounds;
    for &body in order.iter() {
        let octant = octant_of(body);
        scratch[cursor[octant] as usize] = body;
        cursor[octant] += 1;
    }
    order.copy_from_slice(scratch);
    bounds
}

/// The bodies and cell aggregates one node interacts with, laid out for the repulsion kernel.
#[derive(Default)]
pub(crate) struct Interactions {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub z: Vec<f32>,
    pub mass: Vec<f32>,
    /// Largest force, per component, that the entry may contribute.
    pub limit: Vec<f32>,
    /// Traversal stack, kept here so a walk allocates nothing.
    stack: Vec<u32>,
}

impl Interactions {
    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
        self.z.clear();
        self.mass.clear();
        self.limit.clear();
        self.stack.clear();
    }

    fn push(&mut self, x: f32, y: f32, z: f32, mass: f32, limit: f32) {
        self.x.push(x);
        self.y.push(y);
        self.z.push(z);
        self.mass.push(mass);
        self.limit.push(limit);
    }
}
