//! Flat octree behind the Barnes-Hut repulsion pass.
//!
//! Cells live in parallel arrays and refer to each other by index, so a build reuses the
//! allocations from the previous step and a traversal walks indices instead of pointers.

use crate::LANES;

/// The group one tree walk serves: walk count against interaction-list length. A 4000-node graph
/// at the default opening angle costs 3.5 ms per step at 8 bodies, 2.2 ms at 32, 2.5 ms at 64. A
/// multiple of [`LANES`], so a leaf's bodies fill whole iterations of the repulsion kernel.
const LEAF_CAPACITY: usize = LANES * 4;
/// Coincident bodies would subdivide forever. At this depth the leaf keeps them all instead.
const MAX_DEPTH: u32 = 20;
const NO_CHILD: u32 = u32::MAX;

/// The bodies' positions and masses, one slice per field.
#[derive(Clone, Copy)]
pub(crate) struct Bodies<'a> {
    pub x: &'a [f32],
    pub y: &'a [f32],
    pub z: &'a [f32],
    pub mass: &'a [f32],
}

/// The bodies copied into [`Octree::order`]'s order, so a leaf's run is contiguous.
#[derive(Default)]
pub(crate) struct SortedBodies {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub z: Vec<f32>,
    pub mass: Vec<f32>,
}

impl SortedBodies {
    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
        self.z.clear();
        self.mass.clear();
    }
}

/// A cell that stopped subdividing, with the box its bodies span.
pub(crate) struct Leaf {
    pub start: u32,
    pub end: u32,
    pub lo: [f32; 3],
    pub hi: [f32; 3],
}

#[derive(Default)]
pub(crate) struct Octree {
    com_x: Vec<f32>,
    com_y: Vec<f32>,
    com_z: Vec<f32>,
    mass: Vec<f32>,
    /// Full width of the cell, for the `width / distance < theta` acceptance test.
    width: Vec<f32>,
    /// All [`NO_CHILD`] for a leaf.
    children: Vec<[u32; 8]>,
    /// `(start, count)` into `order`: a cell owns one contiguous run of bodies. Kept for internal
    /// cells too, so the traversal can recognize the ones enclosing the node it gathers for.
    bodies: Vec<(u32, u32)>,
    pub leaves: Vec<Leaf>,
    /// Live body slots, ordered so that every leaf owns one contiguous run.
    pub order: Vec<u32>,
    pub sorted: SortedBodies,
    partition_scratch: Vec<u32>,
    octant_scratch: Vec<u8>,
}

impl Octree {
    /// Over every slot with mass: a massless slot neither exerts nor feels a force, so leaving it
    /// out also keeps the root cube tight around the live nodes.
    pub fn build(&mut self, bodies: Bodies) {
        self.com_x.clear();
        self.com_y.clear();
        self.com_z.clear();
        self.mass.clear();
        self.width.clear();
        self.children.clear();
        self.bodies.clear();
        self.leaves.clear();
        self.order.clear();
        self.sorted.clear();
        for (slot, &m) in bodies.mass.iter().enumerate() {
            if m > 0.0 {
                self.order.push(slot as u32);
            }
        }
        if self.order.is_empty() {
            return;
        }
        self.sorted.x.resize(self.order.len(), 0.0);
        self.sorted.y.resize(self.order.len(), 0.0);
        self.sorted.z.resize(self.order.len(), 0.0);
        self.sorted.mass.resize(self.order.len(), 0.0);

        let (lo, hi) = bounding_box(&self.order, bodies);
        let root = Cube {
            center: [
                (lo[0] + hi[0]) * 0.5,
                (lo[1] + hi[1]) * 0.5,
                (lo[2] + hi[2]) * 0.5,
            ],
            // Never zero: a degenerate cube would pass the acceptance test at any distance.
            width: (hi[0] - lo[0])
                .max(hi[1] - lo[1])
                .max(hi[2] - lo[2])
                .max(f32::MIN_POSITIVE),
        };

        // Taken so the recursion can borrow `order` mutably while pushing cells.
        let mut order = std::mem::take(&mut self.order);
        let mut scratch = Scratch {
            partition: std::mem::take(&mut self.partition_scratch),
            octants: std::mem::take(&mut self.octant_scratch),
        };
        self.build_cell(&mut order, &mut scratch, 0, root, 0, bodies);
        self.order = order;
        self.partition_scratch = scratch.partition;
        self.octant_scratch = scratch.octants;
    }

    /// Aggregates bottom-up: a leaf sums its bodies, an internal cell its children.
    fn build_cell(
        &mut self,
        order: &mut [u32],
        scratch: &mut Scratch,
        offset: u32,
        cube: Cube,
        depth: u32,
        bodies: Bodies,
    ) -> u32 {
        let cell = self.width.len();
        self.width.push(cube.width);
        self.children.push([NO_CHILD; 8]);
        self.bodies.push((offset, order.len() as u32));
        self.com_x.push(0.0);
        self.com_y.push(0.0);
        self.com_z.push(0.0);
        self.mass.push(0.0);

        if order.len() <= LEAF_CAPACITY || depth >= MAX_DEPTH {
            self.build_leaf(cell, order, offset, bodies);
            return cell as u32;
        }

        let bounds = partition_octants(order, scratch, cube.center, bodies);
        let (mut total, mut cx, mut cy, mut cz) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for octant in 0..8 {
            let (start, end) = (bounds[octant] as usize, bounds[octant + 1] as usize);
            if start == end {
                continue;
            }
            let child = self.build_cell(
                &mut order[start..end],
                scratch,
                offset + start as u32,
                cube.octant(octant),
                depth + 1,
                bodies,
            );
            self.children[cell][octant] = child;
            let (child, m) = (child as usize, self.mass[child as usize]);
            total += m;
            cx += self.com_x[child] * m;
            cy += self.com_y[child] * m;
            cz += self.com_z[child] * m;
        }
        self.set_aggregate(cell, total, [cx, cy, cz]);
        cell as u32
    }

    /// A leaf's run of `order` is final, so this is where its bodies are copied into place.
    fn build_leaf(&mut self, cell: usize, order: &[u32], offset: u32, bodies: Bodies) {
        let (start, end) = (offset as usize, offset as usize + order.len());
        let (mut total, mut cx, mut cy, mut cz) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for (k, &body) in (start..end).zip(order) {
            let body = body as usize;
            let (x, y, z, m) = (
                bodies.x[body],
                bodies.y[body],
                bodies.z[body],
                bodies.mass[body],
            );
            (
                self.sorted.x[k],
                self.sorted.y[k],
                self.sorted.z[k],
                self.sorted.mass[k],
            ) = (x, y, z, m);
            total += m;
            cx += x * m;
            cy += y * m;
            cz += z * m;
            for (axis, value) in [x, y, z].into_iter().enumerate() {
                lo[axis] = lo[axis].min(value);
                hi[axis] = hi[axis].max(value);
            }
        }
        self.set_aggregate(cell, total, [cx, cy, cz]);
        self.leaves.push(Leaf {
            start: start as u32,
            end: end as u32,
            lo,
            hi,
        });
    }

    /// `weighted` is the mass-weighted sum of positions.
    fn set_aggregate(&mut self, cell: usize, total: f32, weighted: [f32; 3]) {
        let inv_total = if total > 0.0 { total.recip() } else { 0.0 };
        self.com_x[cell] = weighted[0] * inv_total;
        self.com_y[cell] = weighted[1] * inv_total;
        self.com_z[cell] = weighted[2] * inv_total;
        self.mass[cell] = total;
    }

    /// Collects the bodies and cell aggregates everything inside the box `lo..hi` interacts with,
    /// together with the force each entry may contribute.
    ///
    /// A cell stands in for its bodies once its width subtends less than `theta` from the whole
    /// box, measured from the nearest corner, so the list is valid for every node in the box and
    /// one walk serves a leaf's worth of nodes.
    ///
    /// `query` is the leaf the box was measured from. Its own bodies must reach the kernel as
    /// bodies, never folded into an aggregate: only an exactly zero offset makes a node's pull on
    /// itself zero, and an aggregate standing in for it sits a rounding error away, which the
    /// softened inverse square turns into a full-strength kick.
    pub fn gather(&self, query: &Leaf, theta: f32, force_max: f32, out: &mut Interactions) {
        out.clear();
        if self.width.is_empty() {
            return;
        }
        let (lo, hi) = (query.lo, query.hi);
        let theta_sqrd = theta * theta;
        let query_start = query.start;
        out.traversal_stack.push(0);
        while let Some(cell) = out.traversal_stack.pop() {
            let cell = cell as usize;
            let com = [self.com_x[cell], self.com_y[cell], self.com_z[cell]];
            let mut distance_sqrd = 0.0;
            for axis in 0..3 {
                let gap = (lo[axis] - com[axis]).max(com[axis] - hi[axis]).max(0.0);
                distance_sqrd += gap * gap;
            }
            let width = self.width[cell];
            // The center of mass may lie outside an enclosing box; coincident bodies have no width.
            let (start, count) = self.bodies[cell];
            let encloses_query = start <= query_start && query_start < start + count;
            if !encloses_query && width * width < theta_sqrd * distance_sqrd {
                // Stands for `count` pairs, each clamped to `force_max`.
                out.push(com, self.mass[cell], force_max * count as f32);
                continue;
            }
            let children = &self.children[cell];
            if *children == [NO_CHILD; 8] {
                out.extend(
                    &self.sorted,
                    start as usize..(start + count) as usize,
                    force_max,
                );
                continue;
            }
            out.traversal_stack
                .extend(children.iter().filter(|&&child| child != NO_CHILD));
        }
    }
}

#[derive(Clone, Copy)]
struct Cube {
    center: [f32; 3],
    width: f32,
}

impl Cube {
    /// The child cube in `octant`, whose bit `axis` is set on the positive side of that axis.
    fn octant(self, octant: usize) -> Self {
        let quarter = self.width * 0.25;
        let shift = |axis: usize| match octant & (1 << axis) {
            0 => -quarter,
            _ => quarter,
        };
        Self {
            center: [
                self.center[0] + shift(0),
                self.center[1] + shift(1),
                self.center[2] + shift(2),
            ],
            width: self.width * 0.5,
        }
    }
}

struct Scratch {
    partition: Vec<u32>,
    octants: Vec<u8>,
}

fn bounding_box(order: &[u32], bodies: Bodies) -> ([f32; 3], [f32; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for &body in order {
        let body = body as usize;
        for (axis, value) in [bodies.x[body], bodies.y[body], bodies.z[body]]
            .into_iter()
            .enumerate()
        {
            lo[axis] = lo[axis].min(value);
            hi[axis] = hi[axis].max(value);
        }
    }
    (lo, hi)
}

/// Sorts `order` into the eight octants of `center` and returns the nine boundaries between them.
fn partition_octants(
    order: &mut [u32],
    scratch: &mut Scratch,
    center: [f32; 3],
    bodies: Bodies,
) -> [u32; 9] {
    scratch.octants.clear();
    scratch.octants.extend(order.iter().map(|&body| {
        let body = body as usize;
        (bodies.x[body] >= center[0]) as u8
            | ((bodies.y[body] >= center[1]) as u8) << 1
            | ((bodies.z[body] >= center[2]) as u8) << 2
    }));

    let mut counts = [0u32; 8];
    for &octant in &scratch.octants {
        counts[octant as usize] += 1;
    }
    let mut bounds = [0u32; 9];
    for octant in 0..8 {
        bounds[octant + 1] = bounds[octant] + counts[octant];
    }

    let partition = &mut scratch.partition;
    partition.clear();
    partition.resize(order.len(), 0);
    let mut cursor = bounds;
    for (&body, &octant) in order.iter().zip(&scratch.octants) {
        let octant = octant as usize;
        partition[cursor[octant] as usize] = body;
        cursor[octant] += 1;
    }
    order.copy_from_slice(partition);
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
    traversal_stack: Vec<u32>,
}

impl Interactions {
    fn clear(&mut self) {
        self.x.clear();
        self.y.clear();
        self.z.clear();
        self.mass.clear();
        self.limit.clear();
        self.traversal_stack.clear();
    }

    fn push(&mut self, [x, y, z]: [f32; 3], mass: f32, limit: f32) {
        self.x.push(x);
        self.y.push(y);
        self.z.push(z);
        self.mass.push(mass);
        self.limit.push(limit);
    }

    /// Every body in `run` of `sorted`, each one a pair clamped to `force_max`.
    fn extend(&mut self, sorted: &SortedBodies, run: std::ops::Range<usize>, force_max: f32) {
        self.x.extend_from_slice(&sorted.x[run.clone()]);
        self.y.extend_from_slice(&sorted.y[run.clone()]);
        self.z.extend_from_slice(&sorted.z[run.clone()]);
        self.mass.extend_from_slice(&sorted.mass[run.clone()]);
        self.limit.resize(self.limit.len() + run.len(), force_max);
    }
}
