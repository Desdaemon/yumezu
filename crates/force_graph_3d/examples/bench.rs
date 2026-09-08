//! Times `ForceGraph::update` on the machine the kernel is meant for.
//!
//! `bench <nodes> <theta> <steps>`, printing nanoseconds per step. `theta` of zero sums every pair
//! and is the shape the lane loop dominates; the default angle adds the octree walk around it and
//! is what the application runs.
//!
//! `tools/bench-arm.sh` runs this on an Android device over adb.

use force_graph_3d::*;
use std::time::Instant;

fn main() {
    let arg = |n: usize, default: &str| -> String {
        std::env::args().nth(n).unwrap_or_else(|| default.into())
    };
    let nodes: usize = arg(1, "4000").parse().expect("node count");
    let theta: f32 = arg(2, "0").parse().expect("theta");
    let steps: usize = arg(3, "60").parse().expect("step count");

    let mut graph = <ForceGraph>::new(SimulationParameters {
        theta,
        // The layout must not settle partway through, or the later steps measure the early exit
        // rather than the kernel.
        settle_speed: 0.0,
        settle_after: None,
        ..Default::default()
    });

    // Deterministic, so two runs measure the same arrangement of work: positions decide how much
    // of the octree a step walks, which is most of what varies between graphs.
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / 2048.0 - 1.0
    };
    let indices: Vec<_> = (0..nodes)
        .map(|_| {
            graph.add_node(NodeData {
                x: next() * 500.0,
                y: next() * 500.0,
                z: next() * 500.0,
                mass: 1.0,
                ..Default::default()
            })
        })
        .collect();
    for pair in indices.windows(2) {
        graph.add_edge(pair[0], pair[1], Default::default());
    }

    // One step ahead of the clock: the first allocates the kernel's scratch buffers.
    graph.update(1.0 / 60.0);
    let start = Instant::now();
    for _ in 0..steps {
        graph.update(1.0 / 60.0);
    }
    let per_step = start.elapsed().as_nanos() / steps as u128;
    println!("{nodes} nodes  theta {theta}  {per_step} ns/step");
}
