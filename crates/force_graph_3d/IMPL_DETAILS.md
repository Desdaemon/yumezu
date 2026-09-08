# Implementation details

Where this crate is written for the compiler rather than for the problem, what each of those
choices is currently worth, and what to re-measure when the toolchain moves.

Everything here is a workaround for something LLVM does not do on its own today, and none of it is
permanent. If a later compiler stops needing one, delete it and take the plainer code:
`tests/codegen.rs` says which ones still apply, and the numbers below say what it costs if one
silently stops working.

Measured 2026-09-07 on rustc 1.98.1 / LLVM 22.1.8, one core of an AMD Ryzen 7 9800X3D. Node counts
are the whole graph; `theta = 0` is the exact O(n²) pass and `0.9` the *Barnes-Hut*[^bh] default.

## The repulsion kernel

`repulsion_on` dominates a step. It is plain scalar Rust over `[f32; LANES]` chunks, written so
LLVM's *SLP vectorizer*[^slp] widens it; the same Rust compiles for x86-64, aarch64 and wasm.

Three source-level idioms in it decide whether the SLP pass fires and what it emits around the
loop it produces.

**The clamp has one variant per architecture.** `f32::max` and `f32::min` are IEEE `maxNum` and
`minNum`, which return the operand that is not NaN when one side is NaN. The hardware minimum and
maximum do the opposite — [`maxps`][maxps]/[`minps`][minps] on x86, [`f32x4.pmax`/`pmin`][pmax] on
wasm — so on both LLVM emits a NaN test and a select after each one to correct it, spelled
[`cmpunord`][cmpps] plus a [blend][blendv] per bound on x86. aarch64 is the only architecture with
the IEEE rule in hardware, as [`fmaxnm`][fmaxnm]/[`fminnm`][fminnm], so there it is the
comparisons that need the second instruction. Neither variant is good on both, and
`clamp_symmetric` picks between
`clamp_ieee` and `clamp_comparisons` on `target_arch`. `f32::clamp` is not a third option — same
NaN rule as the methods, and a branch for `min > max` besides.

Cycles per iteration of the vectorized loop, which is eight interactions on every model below,
from `llvm-mca`[^mca] over the loop the compiler actually emitted. The chosen variant is in bold:

| model | comparisons | `f32::min`/`max` |
|---|---|---|
| skylake, SSE2 | **41.5** | 96.0 |
| skylake, AVX2 | **18.1** | 31.4 |
| znver3, SSE2 | **35.0** | 72.0 |
| znver3, AVX2 | **16.5** | 26.1 |
| alderlake, SSE2 | **21.8** | 52.6 |
| alderlake, AVX2 | **11.1** | 21.6 |
| cortex-a510, cortex-a520 | 106.0 | **101.0** |
| cortex-a55 | 140.0 | **130.0** |
| cortex-a710, cortex-a720, neoverse-n2 | 53.0 | **49.0** |
| cortex-x2, cortex-x3, neoverse-v2 | 25.5 | **23.5** |
| neoverse-n1 | 47.3 | **43.0** |
| cortex-x1, neoverse-v1 | 23.2 | **21.0** |
| cortex-x4, cortex-x925 | 22.1 | **19.0** |

On x86 the correction costs between a third and a half of the loop, and the SSE2 rows are the
x86-64 baseline feature level, which is what a build without a `target-feature` opt-in gets.
aarch64 reverses the ranking on all twelve of its
scheduling models, favouring `f32::min`/`max` by 4.7% to 13.7%, the newest big cores gaining most.

The two variants agree on every input except two. A NaN `limit` the comparisons propagate and
`minNum` discards; nothing in the crate produces one — `limit` is `force_max`, or a multiple of it
— and `clamp_symmetric` documents the argument as excluding it. And the two disagree on which
zero they return, but only when running on aarch64, because `fmaxnm` and `fminnm` do not pick the
same one the comparisons do; a force of `-0.0` moves a node exactly as far as one of `0.0`, so
`the_two_clamp_variants_agree` compares zeroes by value.

The zero sign disagreement is invisible from x86 — the same test passes there bit for bit — and
was found by `just test-arm`, which runs these tests on armv8 under qemu. Both variants are
compiled everywhere so one host can check them against each other, but a property that only
appears on the hardware needs the hardware. `tests/codegen.rs` covers the separate question of
whether each target got the variant meant for it.

`llvm-mca` has no wasm model, and a browser's own backend is what picks the final instructions
anyway; the x86 rows are the closest available proxy for what a browser's wasm backend emits.

Those are static estimates of one loop; the millisecond tables below are the same comparison run
as a whole simulation. Where they disagree, the milliseconds are what the machine did.

**The lane loop iterates `as_chunks`, not a running index.** Five slices indexed by a common
`base` leaves a bounds check inside the vectorized loop: LLVM does not prove `base + LANES <= n`
from a trip count computed as `n - n % LANES`, even when that trip count was just computed as
`n & !(LANES - 1)`. A `&[[f32; LANES]]` carries the count in its type instead, so there is nothing
to prove. The remainder that `as_chunks` hands back is statically shorter than `LANES`, which also
turns the scalar tail from a loop into a *peeled*[^peel] sequence.

**`repulsion_on` is not `#[inline]`.** It was, to stop the lane loop from regressing to scalar
code without it. That no longer happens — the out-of-line copy vectorizes identically on x86-64
and wasm — and the function is entered once per node, so the call is already amortized over that
node's interactions.

What each is worth, one change at a time, from `just bench`. Milliseconds per step of the whole
simulation; the swapped-clamp column is each architecture built with the variant meant for the
opposite architecture, and the lanes column is the running-index loop the bounds check survives
in. The phone is a moto g power 5G (2024), a Dimensity 7020, with each run pinned to one core of
a cluster.

Exact summation, 4000 nodes:

| | shipped | swapped clamp | indexed lanes |
|---|---|---|---|
| Ryzen 9800X3D, `target-cpu=x86-64` | 11.5 | 27.1 | 49.4 |
| Ryzen 9800X3D, `target-cpu=native` | 5.5 | 9.3 | 45.6 |
| Cortex-A55, 2.0 GHz | 143.1 | 161.0 | 692.0 |
| Cortex-A76, 2.2 GHz | 40.4 | 61.7 | 177.8 |

Barnes-Hut at the default opening angle, 16000 nodes, which is what the application runs:

| | shipped | swapped clamp | indexed lanes |
|---|---|---|---|
| Ryzen 9800X3D, `target-cpu=x86-64` | 8.4 | 15.0 | 24.7 |
| Ryzen 9800X3D, `target-cpu=native` | 5.8 | 7.4 | 22.3 |
| Cortex-A55, 2.0 GHz | 127.4 | 131.9 | 364.9 |
| Cortex-A76, 2.2 GHz | 28.0 | 37.7 | 86.4 |

The bounds check costs the most of the three by a wide margin, 3x to 8x, and is the one nothing in
the Rust suggests: the indexed loop reads identically and differs only in what LLVM can prove
about it. It costs most where the vectors are widest, because what it costs is the vectorization
rather than the check.

The clamp is 1.3x to 2.4x on the exact pass, and the two aarch64 rows are why there are two
variants at all. The Cortex-A76 figure, 1.5x, is far from the 10% `llvm-mca` estimates for the
aarch64 cores it does model — it has no A76 model, and that is the core the difference matters
most on. The A55 row's run-to-run variance is several percent, so treat it as roughly a tenth
rather than as 12.6%.

Barnes-Hut cuts interactions per node to O(log n), so the kernel is a smaller share of a step and
every difference above shrinks with it: the octree walk around the kernel is unchanged either
way. That is the honest number for the application, and it is still 1.35x on a phone's big core.

## The optimization level of the shipping build

`[profile.min]` in the workspace manifest is `opt-level = 3` rather than a size level, and that
is worth more than every source-level change above put together.

The lane loop is a fixed-trip inner loop, so it has to be *unrolled*[^unroll] to `LANES`
before the SLP vectorizer can see anything to widen. Both `"s"` and `"z"` turn the unroller off.
Counting `simd128` instructions in a small wasm module that does nothing but step a graph:

| profile | instructions |
|---|---|
| `opt-level = "z"` | 9 |
| `opt-level = "s"` | 16 |
| `opt-level = 3` | 251 |

Confining the speed level to this one crate with `[profile.min.package.force_graph_3d]` does not
work under *LTO*[^lto]. Fat and thin both defer the vectorizers to the post-link pipeline, which
runs at whatever the top-level `opt-level` says; the same module scores 20 either way. Only with
LTO off does a per-package override take effect. For the same reason, `--emit=llvm-ir` on an LTO
build shows pre-link bitcode that has not been vectorized at any optimization level, so
`tests/codegen.rs` builds without LTO to inspect the emitted kernel and checks the manifest
separately.

The whole application, built for wasm32 with `+simd128`. "Kernel" is whether `repulsion_on` in
the emitted wasm holds `f32x4` instructions or scalar ones.

| profile | wasm | brotli[^brotli] | kernel |
|---|---|---|---|
| `"s"`, fat LTO | 7.68 MB | 2.260 MB | scalar |
| `3`, fat LTO | 9.17 MB | 2.444 MB | vector |
| `3`, no LTO | 8.79 MB | 2.435 MB | vector |
| `"s"`, no LTO, package override | 8.31 MB | 2.299 MB | vector |
| `"z"`, no LTO, package override | 8.01 MB | 2.242 MB | vector |

The last row is the smallest after brotli and still gets the vectorized kernel. What it gives up
is cross-crate inlining for everything outside this crate, which is mostly rendering and does not
show in a size measurement; that is why the shipped profile is the second row and not the last.
Fat LTO is not worth its cost on size either way — 374 KB more raw wasm than the same profile
without it, 9 KB less after brotli.

## Considered and rejected

**A reciprocal square root estimate.** `(d2 + SOFTENING).sqrt().recip()` is the two
highest-latency vector operations the machine has, chained, in a loop that is otherwise multiplies
and adds. x86 has [`rsqrtps`][rsqrtps] and aarch64 has [`frsqrte`][frsqrte], both around 12 bits;
one *Newton-Raphson step*[^nr] brings either to within a bit or two of exact, far inside the ~1%
the Barnes-Hut approximation already costs.

Substituted into the emitted loop and measured, it is mostly not worth adopting. Cycles per
iteration against the current code:

| model | current | estimate, 1 step | estimate, 2 steps |
|---|---|---|---|
| skylake, AVX2 | 18.1 | 18.1 | — |
| znver3, AVX2 | 16.5 | 15.0 | — |
| alderlake, AVX2 | 11.1 | 11.6 | — |
| cortex-a510 | 101.0 | 137.0 | 177.0 |
| cortex-a55 | 130.0 | 167.0 | 207.0 |
| cortex-a710 | 49.0 | 38.5 | 48.5 |
| neoverse-n1 | 43.0 | 38.5 | 48.5 |
| cortex-x1 | 21.0 | 23.0 | 33.0 |
| cortex-x2 | 23.5 | 23.0 | 33.0 |
| cortex-x4 | 19.0 | 25.0 | 33.0 |

Latency is not the bottleneck here; *resource pressure*[^mca] is. The divide and square root are
pipelined on everything modern, with enough independent work either side to cover their latency,
so what limits the loop is issue bandwidth rather than how long any one operation takes.
`llvm-mca` charges each instruction to a *processor resource unit*[^mca], one micro-op per unit
per cycle, each model naming its units after the hardware it models — and the two sequences do not
compete for the same units at all.

Taking each pair on its own on Skylake, [`vsqrtps`][sqrtps] and [`vdivps`][divps] charge 11 cycles
to `SKLFPDivider` and 2 to a vector-multiply unit, where the estimate charges the divider nothing
and about 2.5 cycles to each of the two units that have a multiplier behind them. That is 11
cycles against 2 in isolation and no gain in the loop, because those two units are what the loop
is already limited by while the divider it frees had slack: the substitution moves work off an
idle unit onto the busy one, and its *block reciprocal throughput*[^mca] rises, 12.5 to 14.5.

The aarch64 split is the same accounting read the other way. On `neoverse-n1`, `fsqrt` and `fdiv`
are 7 cycles each on `N1UnitV0`, where `frsqrte` is 2 and [`frsqrts`][frsqrts] and the
multiplies around 1 apiece spread over `N1UnitV0` and `N1UnitV1` — the estimate empties the pipe
that was holding the loop up, which is the gain on that class of core. On `cortex-a55` and
`cortex-a510`, the two *in-order*[^inorder] cores here, both `frsqrte` and `frsqrts` are charged 9
cycles to `CortexA55UnitFPDIV`, the very unit `fsqrt` uses, so one Newton-Raphson step occupies it
twice instead of once and adds two multiplies on the FP ALU besides: 18 cycles against the 19 of
the pair it replaces, and a loss once those multiplies land in a loop already full of them. Their
NEON *datapath*[^datapath] is 64-bit, so each of those multiplies occupies it for two cycles
rather than one. Two Newton-Raphson steps are a loss everywhere, and the machines that would most
want this are the ones it hurts most.

Two more things to weigh, neither of them about cycle counts. wasm has no reciprocal-estimate
instruction at all — its SIMD is required to be bit-deterministic — so the browser build, the one
with the tightest frame budget, cannot have this whatever the flags say: `-ffast-math` emits the
same square root and divide as the default build, instruction for instruction.

On the native targets the opposite holds, and it is no longer something to wait for. The
substitution is gated on one *fast-math flag*[^fastmath], `arcp`, on the divide — clang makes it
with `-freciprocal-math` alone, where `-fapprox-func` alone changes nothing — and
`f32::algebraic_div` sets that flag from safe stable Rust on a single expression, which is enough
for `rsqrtps` and one Newton-Raphson step to come out under rustc 1.98.1. So there are no
intrinsics to write and nothing in the toolchain standing in the way; what argues against the
substitution is the table above.

**A wider `LANES` on AVX-512.** `LANES` is 8, which fills a 256-bit vector register. The machine
this was measured on has `avx512f`, but nothing the application ships is built for it.

## Checking by hand

`tests/codegen.rs` covers the three properties above automatically, and `just bench <nodes>
<theta> <steps>` times each of them against the variant it was chosen over, here and on a phone
over adb; that is where the millisecond tables came from. It builds through `tools/variant.py`,
which writes the alternatives into a scratch copy of the crate, so none of them has to live in the
kernel behind a `cfg`. An alternative whose anchor text no longer matches is an error there rather
than a silent no-op: the kernel moved, and whatever the number measured wants remeasuring.

To look at the code directly:

```sh
# x86-64 assembly, Intel syntax, for a chosen microarchitecture level
RUSTFLAGS="-C target-cpu=x86-64-v3" cargo rustc --release -p force_graph_3d \
  -- --emit asm -C llvm-args=--x86-asm-syntax=intel

# cycles for the vectorized loop, given the block cut out of one of those .s files. Every number
# in this file came from this. `-mcpu=cortex-a76` silently falls back to the Cortex-A57 model and
# reports a non-pipelined divider that no A76 has, so it is not among them.
llvm-mca -mcpu=skylake -iterations=1000 -bottleneck-analysis loop.s

# which unit an instruction is charged to, for the two- to four-instruction sequences compared in
# "Considered and rejected". Those per-unit figures are the pairs alone, not the loop around them.
llvm-mca -mcpu=skylake -iterations=100 pair.s | sed -n '/Resource pressure by instruction/,$p'

# the emitted wasm of the real application
just dist && wasm-objdump -d dist/*.wasm | grep -E 'f32x4|v128\.'
```

An `rlib` on its own instantiates nothing, because the kernel is only reachable through a generic
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
