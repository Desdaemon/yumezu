# Implementation details

Where this crate is written for the compiler, what each such choice is worth, and what to
re-measure when the toolchain moves.

Each choice works around something LLVM does not do unaided today. When a later compiler stops
needing a workaround, delete the workaround for the plainer code: `tests/codegen.rs` says which
still apply, and the figures below say what each costs if one silently stops working.

## Measurements

The figures come in two sets, taken on different compilers and against different kernels. They are
not comparable with each other, and every table below says which set it belongs to.

| set | taken | toolchain | `inv_cube` | tool |
|---|---|---|---|---|
| milliseconds per simulation step | 2026-11-15 | rustc 1.100.0 | as in `src/lib.rs` | `just bench` |
| cycles per vectorized loop iteration | 2026-09-07 | rustc 1.98.1 / LLVM 22.1.8 | one more multiply per interaction | `llvm-mca`[^mca] |

The desktop figures are one core of an AMD Ryzen 7 9800X3D. The phone is a moto g power 5G (2024), a
Dimensity 7020 of six Cortex-A55 at 2.0 GHz and two Cortex-A78 at 2.2 GHz, with each measurement
pinned to one core of a cluster. Node counts are the whole graph; `theta = 0` is the exact O(n²)
pass and `0.7` the *Barnes-Hut*[^bh] default. The cycle counts are `llvm-mca` over the loop the
compiler emitted, and one iteration of that loop is eight interactions on every model.

## The repulsion kernel

`repulsion_on` dominates a step: safe scalar Rust over `[f32; LANES]` chunks, written so
LLVM's *SLP vectorizer*[^slp] widens it, and the same source compiles for x86-64, aarch64 and wasm
simd128. Three choices in it decide whether the SLP pass fires and what it emits around the loop it
produces: a clamp per architecture, a lane loop over `as_chunks`, and the kernel kept out of line.
A fourth picks the vector width on x86 at runtime.

Node state is one `Vec<f32>` per component, so the scan is flat and branch-free. Every pair is
computed twice, once per node, because the scattered write that Newton's third law needs blocks
vectorization. Forces are gathered from one snapshot and integrated afterwards (Jacobi), so results
do not depend on node order.

### The clamp has one variant per architecture

`clamp_symmetric` picks `clamp_comparisons` on every `target_arch` except aarch64, which gets
`clamp_ieee`. Each is the spelling that lowers to bare vector min/max on the matching architecture.

`f32::max` and `f32::min` are IEEE `maxNum` and `minNum`, which return the operand that is not NaN
when one side is NaN. The hardware minimum and maximum do the opposite on x86, as
[`maxps`][maxps]/[`minps`][minps], and on wasm, as [`f32x4.pmax`/`pmin`][pmax], so on both LLVM
follows each bound with a NaN test and a select; on x86 that is [`cmpunord`][cmpps] plus a
[blend][blendv] per bound. aarch64 alone has the IEEE rule in hardware, as
[`fmaxnm`][fmaxnm]/[`fminnm`][fminnm], so there the comparisons are what need the second
instruction. `f32::clamp` is no third option: it returns a NaN `v` as NaN, where the kernel needs
a number, and panics on a `min > max` bound.

On x86 the NaN correction costs a third to three fifths of the loop. The SSE2 columns are the
x86-64 baseline feature level, which a build gets without a `target-feature` opt-in. Cycles per
loop iteration, 2026-09-07 set, shipped `clamp_comparisons` against swapped `clamp_ieee`:

| x86 model | SSE2 shipped | SSE2 swapped | AVX2 shipped | AVX2 swapped |
|---|---:|---:|---:|---:|
| skylake | 41.5 | 96.0 | 18.1 | 31.4 |
| znver3 | 35.0 | 72.0 | 16.5 | 26.1 |
| alderlake | 21.8 | 52.6 | 11.1 | 21.6 |

aarch64 reverses the ranking on every aarch64 scheduling model, favouring `f32::min`/`max` by
4.7% to 14.0%, the newest big cores gaining most. Same set, shipped `clamp_ieee` against swapped
`clamp_comparisons`:

| aarch64 model | shipped | swapped |
|---|---:|---:|
| cortex-a55 | 130.0 | 140.0 |
| cortex-a510, cortex-a520 | 101.0 | 106.0 |
| cortex-a710, cortex-a720, neoverse-n2 | 49.0 | 53.0 |
| neoverse-n1 | 43.0 | 47.3 |
| cortex-x2, cortex-x3, neoverse-v2 | 23.5 | 25.5 |
| cortex-x1, neoverse-v1 | 21.0 | 23.2 |
| cortex-x4, cortex-x925 | 19.0 | 22.1 |

`llvm-mca` has no wasm model, and a browser's own backend picks the final instructions; the x86 rows
are the closest available proxy for what the browser emits. These are static estimates of one loop.
Where they disagree with the millisecond tables, the milliseconds are what the machine did.

The two variants agree on every input except two:

- A NaN `limit`, which the comparisons propagate and `minNum` discards. Nothing in the crate
  produces one: `limit` is `force_max` or a multiple, and `clamp_symmetric` documents the
  argument as excluding NaN.
- The sign of a zero result, on aarch64 only, where `fmaxnm` and `fminnm` do not pick the zero the
  comparisons pick. A force of `-0.0` moves a node exactly as far as `0.0`, so
  `the_two_clamp_variants_agree` compares zeroes by value.

On x86 the same test passes bit for bit; `just test-arm`, which runs the tests on armv8 under qemu,
found the zero-sign difference. Both variants compile everywhere so one host can test them against
each other, but a property that only appears on the hardware needs the hardware.
`tests/codegen.rs` asserts separately that each target got the variant meant for it.

### The lane loop iterates `as_chunks`

Five slices indexed by a common running `base` leave a bounds check in the loop, and the loop
scalar: LLVM does not prove `base + LANES <= n` from a trip count of `n - n % LANES`, nor from one
of `n & !(LANES - 1)`. A `&[[f32; LANES]]` carries the count in its type, so there is nothing to
prove. The remainder `as_chunks` returns is statically shorter than `LANES`, so LLVM may
*peel*[^peel] the scalar tail, and on wasm it does.

### `repulsion_on` is not `#[inline]`

Out of line, the kernel is compiled in this crate at `opt-level = 3`. An `#[inline]` function is
compiled in the crate that calls it, and the kernel's caller is a generic method instantiated in
the application, which the shipping profile builds for size: the copy there comes out scalar. The
kernel is entered once per node, so the call is amortized over that node's interactions.
`tests/codegen.rs` builds its consumer for speed and does not see this.

### x86 dispatches to AVX2 at runtime

The desktop build targets baseline x86-64, whose SSE2 vectors hold four lanes. `repulsion_on` asks
`is_x86_feature_detected!("avx2")`, a cached load, and on a yes enters `repulsion_on_avx2`, the
same kernel compiled with `#[target_feature(enable = "avx2")]` at eight lanes. That halves the exact
pass and takes a third off Barnes-Hut, and the dispatched build times within 2% of
`target-cpu=x86-64-v3`. FMA is not enabled: Rust never contracts a multiply and an add unasked,
so FMA adds nothing here.

### What each choice is worth

The bounds check costs the most, 3x to 8x, and nothing in the Rust hints at the cost: the indexed
loop reads the same and differs only in what LLVM can prove. The cost peaks where the vectors are
widest, because the loss is the vectorization itself.

The swapped clamp costs up to 2x on the exact pass, and the two aarch64 rows are why there are two
variants. The Cortex-A78's 1.4x is far from the 10% `llvm-mca` estimates for aarch64, for a cause
not yet found: the two loops differ only by twelve `fcmgt` and twelve `bif` standing where twelve
`fmaxnm`/`fminnm` were, and the gap holds whichever variant runs first. The Cortex-A55's 3% is
inside the variance between measurements.

Milliseconds per step of the whole simulation, 2026-11-15 set, from `just bench`, one change at a
time. The SSE2 rows are the path a CPU without AVX2 takes; this host has AVX2, so `just bench`
reaches that path only with `repulsion_on_avx2` deleted from the scratch copy. Swapped clamp builds
each architecture with the variant meant for the other; indexed lanes is the running-index loop
that keeps the bounds check. Each ratio divides by the shipped column.

Exact summation, 4000 nodes:

| target | shipped (ms) | swapped clamp (ms) | ratio | indexed lanes (ms) | ratio |
|---|---:|---:|---:|---:|---:|
| 9800X3D, SSE2 | 10.4 | 20.6 | 1.98 | 41.3 | 3.97 |
| 9800X3D, AVX2 | 5.1 | 6.8 | 1.33 | 41.2 | 8.08 |
| Cortex-A55, 2.0 GHz | 135.3 | 139.9 | 1.03 | 658.1 | 4.86 |
| Cortex-A78, 2.2 GHz | 37.6 | 53.9 | 1.43 | 163.8 | 4.36 |

Barnes-Hut cuts interactions per node to O(log n), so the kernel is a smaller share of a step and
every ratio shrinks: the octree traversal around the kernel is the same in all three variants. This
is the workload the application runs, and on the phone's big core the swapped clamp still costs
1.4x.

Barnes-Hut at the default opening angle, 16000 nodes:

| target | shipped (ms) | swapped clamp (ms) | ratio | indexed lanes (ms) | ratio |
|---|---:|---:|---:|---:|---:|
| 9800X3D, SSE2 | 7.8 | 13.0 | 1.67 | 23.6 | 3.03 |
| 9800X3D, AVX2 | 5.0 | 5.9 | 1.18 | 23.6 | 4.72 |
| Cortex-A55, 2.0 GHz | 101.1 | 101.7 | 1.01 | 380.2 | 3.76 |
| Cortex-A78, 2.2 GHz | 26.3 | 35.7 | 1.36 | 91.7 | 3.49 |

## The optimization level of the shipping build

`[profile.min]` in the workspace manifest is `opt-level = "z"` with LTO off, and
`[profile.min.package.force_graph_3d]` raises this crate alone to `opt-level = 3`. That override
is worth more than every source-level choice above put together.

The lane loop is a fixed-trip inner loop, so it has to be *unrolled*[^unroll] to `LANES` before
the SLP vectorizer has anything to widen, and both `"s"` and `"z"` turn the unroller off. `simd128`
instructions in a small wasm module that only steps a graph:

| `opt-level` | `simd128` instructions |
|---|---:|
| `"z"` | 9 |
| `"s"` | 16 |
| `3` | 251 |

The per-package override takes effect only with LTO off. Fat and thin *LTO*[^lto] both defer the
vectorizers to the post-link pipeline, which runs at the top-level `opt-level`, and under either
the same module counts 20. For the same reason, `--emit=llvm-ir` on an LTO build shows pre-link
bitcode that no optimization level has vectorized, so `tests/codegen.rs` builds without LTO to
inspect the emitted kernel and reads the manifest separately.

The last row below ships: the smallest after brotli, with the vectorized kernel. That build gives
up cross-crate inlining for everything outside this crate, which is mostly rendering and unmeasured.
At `opt-level = 3`, fat LTO adds 380 KB of raw wasm and 9 KB after brotli. The whole application,
built for wasm32 with `+simd128`; the kernel column says whether `repulsion_on` in the emitted wasm
holds `f32x4` instructions or scalar ones:

| top-level `opt-level` | LTO | override to `3` | wasm (MB) | brotli[^brotli] (MB) | kernel |
|---|---|---|---:|---:|---|
| `"s"` | fat | no | 7.68 | 2.260 | scalar |
| `3` | fat | n/a | 9.17 | 2.444 | vector |
| `3` | off | n/a | 8.79 | 2.435 | vector |
| `"s"` | off | yes | 8.31 | 2.299 | vector |
| `"z"` | off | yes | 8.01 | 2.242 | vector |

## Considered and rejected

### A reciprocal square root estimate

Measured in the emitted loop, the estimate is mostly not worth adopting, hurts the slowest cores
most, and has no wasm instruction.

`inv_cube` chains a square root and a divide, the two highest-latency vector operations the machine
has, in a loop otherwise of multiplies and adds. x86 has [`rsqrtps`][rsqrtps] and aarch64 has
[`frsqrte`][frsqrte], both around 12 bits; one *Newton-Raphson step*[^nr] brings either to within a
bit or two of exact, far inside the ~1% the Barnes-Hut approximation already costs.

Cycles per loop iteration, 2026-09-07 set; a dash marks no figure:

| model | square root and divide | estimate, 1 step | estimate, 2 steps |
|---|---:|---:|---:|
| skylake, AVX2 | 18.1 | 18.1 | — |
| znver3, AVX2 | 16.5 | 15.0 | — |
| alderlake, AVX2 | 11.1 | 11.6 | — |
| cortex-a55 | 130.0 | 167.0 | 207.0 |
| cortex-a510 | 101.0 | 137.0 | 177.0 |
| cortex-a710 | 49.0 | 38.5 | 48.5 |
| neoverse-n1 | 43.0 | 38.5 | 48.5 |
| cortex-x2 | 23.5 | 23.0 | 33.0 |
| cortex-x1 | 21.0 | 23.0 | 33.0 |
| cortex-x4 | 19.0 | 25.0 | 33.0 |

What limits this loop is *resource pressure*[^mca]. The divide and square root are pipelined on
everything modern, with enough independent work either side to cover their latency, so issue
bandwidth sets the pace. `llvm-mca` charges each instruction to a *processor resource unit*[^mca],
one micro-op per unit per cycle, each model naming its units after the hardware it models, and the
two sequences load different units.

On Skylake, taking each pair alone, [`vsqrtps`][sqrtps] and [`vdivps`][divps] charge 11 cycles to
`SKLFPDivider` and 2 to a vector-multiply unit; the estimate charges the divider nothing and about
2.5 cycles to each of the two units with a multiplier behind them. That is 11 cycles against 2 in
isolation and no gain in the loop: those two units already limit the loop while the divider had
slack, so the substitution moves work off an idle unit onto busy ones, and the *block reciprocal
throughput*[^mca] rises from 12.5 to 14.5.

On `neoverse-n1`, `fsqrt` and `fdiv` are 7 cycles each on `N1UnitV0`, while `frsqrte` is 2 and
[`frsqrts`][frsqrts] and the surrounding multiplies about 1 apiece, spread over `N1UnitV0` and
`N1UnitV1`: the estimate empties the pipe that held the loop up, which is the gain on that class of
core. On the two *in-order*[^inorder] cores, `cortex-a55` and `cortex-a510`, `frsqrte` and `frsqrts`
are each charged 9 cycles to `CortexA55UnitFPDIV`, the unit `fsqrt` uses, so one Newton-Raphson step
occupies that unit twice and adds two multiplies on the FP ALU: 18 cycles against the 19 of the
replaced pair, and a loss once those multiplies land in a loop already full of them. Their NEON
*datapath*[^datapath] is 64-bit, so each multiply occupies the datapath for two cycles. Two
Newton-Raphson steps break even at best, and the machines that would most want the estimate are the
ones the estimate hurts most.

wasm has no reciprocal-estimate instruction: wasm SIMD is required to be bit-deterministic.
The browser build, the one with the tightest frame budget, cannot have the estimate whatever the
flags say: `-ffast-math` emits the same square root and divide as the default build, instruction
for instruction.

On the native targets nothing in the toolchain stands in the way. The substitution is gated on one
*fast-math flag*[^fastmath], `arcp`, on the divide: clang sets `arcp` with `-freciprocal-math`
alone, and `-fapprox-func` alone changes nothing. `f32::algebraic_div` sets that flag from safe
stable Rust on a single expression, which is enough for `rsqrtps` and one Newton-Raphson step under
rustc 1.98.1, with no intrinsics. The table above decides against the estimate.

### The charge folded into the divide

On wasm the SLP vectorizer declines `charge_target * m / (r * r.sqrt())` and the kernel comes out
scalar, 3.5x slower under wasmtime. Elsewhere the fold saves one more multiply than `inv_cube` does
and is 9% to 17% faster on x86-64 and the Cortex-A78.

### A wider `LANES` on AVX-512

`LANES` is 8, which fills a 256-bit vector register. On the 9800X3D, `target-cpu=x86-64-v4` and
`native` time within 2% of `x86-64-v3`: AVX2 holds the whole gain over the baseline.

## Checking by hand

`tests/codegen.rs` asserts the clamp variant and the vectorized lane loop on each target, and
`just bench <nodes> <theta> <steps>` times those two against the variant each was chosen over, here
and on a phone over adb; the millisecond tables come from `just bench`. The bench builds through
`tools/variant.py`, which writes the alternatives into a scratch copy of the crate, so none of them
lives in the kernel behind a `cfg`. An alternative whose anchor text no longer matches is an error:
the kernel moved, and the alternative's figure needs remeasuring.

To read the code directly:

```sh
# x86-64 assembly, Intel syntax, for a chosen microarchitecture level
RUSTFLAGS="-C target-cpu=x86-64-v3" cargo rustc --release -p force_graph_3d \
  -- --emit asm -C llvm-args=--x86-asm-syntax=intel

# cycles for the vectorized loop, given the block cut out of one of those .s files; every cycle
# count in this file came from this. LLVM 23 models the phone's big core as `-mcpu=cortex-a78`,
# which has not been run over this loop.
llvm-mca -mcpu=skylake -iterations=1000 -bottleneck-analysis loop.s

# which unit an instruction is charged to, for the two- to four-instruction sequences compared in
# "Considered and rejected"; those per-unit figures are the pairs alone, without the loop around them
llvm-mca -mcpu=skylake -iterations=100 pair.s | sed -n '/Resource pressure by instruction/,$p'

# the emitted wasm of the real application
just dist && wasm-objdump -d dist/*.wasm | grep -E 'f32x4|v128\.'
```

An `rlib` on its own instantiates nothing, because the kernel is reachable only through a generic
type: give the crate a non-generic entry point that steps a graph, or read the IR of a consumer.

[^bh]: <https://en.wikipedia.org/wiki/Barnes%E2%80%93Hut_simulation>
[^slp]: <https://llvm.org/docs/Vectorizers.html#the-slp-vectorizer>
[^mca]: <https://llvm.org/docs/CommandGuide/llvm-mca.html>
[^peel]: <https://en.wikipedia.org/wiki/Loop_splitting>
[^unroll]: <https://en.wikipedia.org/wiki/Loop_unrolling>
[^lto]: <https://en.wikipedia.org/wiki/Interprocedural_optimization>
[^brotli]: <https://en.wikipedia.org/wiki/Brotli>
[^nr]: <https://en.wikipedia.org/wiki/Newton%27s_method>
[^inorder]: <https://en.wikipedia.org/wiki/Out-of-order_execution>
[^datapath]: <https://en.wikipedia.org/wiki/Datapath>
[^fastmath]: <https://llvm.org/docs/LangRef.html#fast-math-flags>

[maxps]: https://www.felixcloutier.com/x86/maxps
[minps]: https://www.felixcloutier.com/x86/minps
[cmpps]: https://www.felixcloutier.com/x86/cmpps
[blendv]: https://www.felixcloutier.com/x86/blendvps
[rsqrtps]: https://www.felixcloutier.com/x86/rsqrtps
[sqrtps]: https://www.felixcloutier.com/x86/sqrtps
[divps]: https://www.felixcloutier.com/x86/divps
[pmax]: https://github.com/WebAssembly/simd/blob/main/proposals/simd/SIMD.md
[fmaxnm]: https://www.scs.stanford.edu/~zyedidia/arm64/fmaxnm_advsimd.html
[fminnm]: https://www.scs.stanford.edu/~zyedidia/arm64/fminnm_advsimd.html
[frsqrte]: https://www.scs.stanford.edu/~zyedidia/arm64/frsqrte_advsimd.html
[frsqrts]: https://www.scs.stanford.edu/~zyedidia/arm64/frsqrts_advsimd.html
