"""Writes a copy of the force-graph crate with one of the kernel's spellings replaced.

`IMPL_DETAILS.md` claims a cost for each of them. Rather than carry the rejected alternatives in
the shipped kernel behind a `cfg`, they live here and are applied to a throwaway copy of the
crate, which is what `tools/bench.sh` then builds. The same thing `tests/codegen.rs` does to get
an isolated build.

    variant.py <shipped|clamp|indexed> <destination>

An anchor that no longer matches is an error rather than a silent no-op: the kernel moved, and
whichever number this was measuring needs remeasuring anyway.
"""

import pathlib
import shutil
import sys

# The lane loop written the way LLVM leaves a bounds check in the middle of it: five slices walked
# by a common running index, which it will not prove `base + LANES <= n` for.
INDEXED_FROM = """    // Chunked rather than indexed by a running base: a `&[[f32; LANES]]` carries the lane count
    // in its type, so nothing in the body needs a bounds check that LLVM then has to prove
    // redundant. It does not manage that proof for five slices indexed in step, and the check it
    // leaves behind sits inside the vector body.
    let (xc, xr) = x.as_chunks::<LANES>();"""

INDEXED_TO = """    let body = n - n % LANES;
    let mut base = 0;
    while base < body {
        for l in 0..LANES {
            let dx = x[base + l] - tx;
            let dy = y[base + l] - ty;
            let dz = z[base + l] - tz;
            let strength = charge_target * mass[base + l] * inv_cube(dx * dx + dy * dy + dz * dz);
            fx[l] += clamp_symmetric(dx * strength, limit[base + l]);
            fy[l] += clamp_symmetric(dy * strength, limit[base + l]);
            fz[l] += clamp_symmetric(dz * strength, limit[base + l]);
        }
        base += LANES;
    }
    for i in body..n {
        let (dx, dy, dz) = (x[i] - tx, y[i] - ty, z[i] - tz);
        let strength = charge_target * mass[i] * inv_cube(dx * dx + dy * dy + dz * dz);
        fx[0] += clamp_symmetric(dx * strength, limit[i]);
        fy[0] += clamp_symmetric(dy * strength, limit[i]);
        fz[0] += clamp_symmetric(dz * strength, limit[i]);
    }
    let _ = (&x, &y, &z, &mass, &limit);"""


def indexed(source: str) -> str:
    start = source.index(INDEXED_FROM)
    end = source.index("    [reduce(fx), reduce(fy), reduce(fz)]", start)
    return source[:start] + INDEXED_TO + "\n\n" + source[end:]


CLAMP_FROM = """    #[cfg(target_arch = "aarch64")]
    {
        clamp_ieee(v, limit)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        clamp_comparisons(v, limit)
    }"""

CLAMP_TO = """    #[cfg(target_arch = "aarch64")]
    {
        clamp_comparisons(v, limit)
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        clamp_ieee(v, limit)
    }"""


def clamp(source: str) -> str:
    """Each architecture gets the body meant for the other one."""
    assert CLAMP_FROM in source, "the clamp dispatch moved"
    return source.replace(CLAMP_FROM, CLAMP_TO)


VARIANTS = {"shipped": lambda source: source, "clamp": clamp, "indexed": indexed}

variant, destination = sys.argv[1], pathlib.Path(sys.argv[2])
crate = pathlib.Path(__file__).resolve().parent.parent / "crates/force_graph_3d"

shutil.rmtree(destination, ignore_errors=True)
shutil.copytree(crate, destination, ignore=shutil.ignore_patterns("target"))
# Standalone, so that building it does not want the application's own dependencies.
manifest = destination / "Cargo.toml"
manifest.write_text("[workspace]\n" + manifest.read_text())

lib = destination / "src/lib.rs"
lib.write_text(VARIANTS[variant](lib.read_text()))
