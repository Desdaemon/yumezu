# Implementation details

Where this crate is shaped by the compiler rather than by the problem, what each of those shapes
is currently worth, and what to re-measure when the toolchain moves.

Everything here is a workaround for something LLVM does not do on its own today, and none of it is
permanent. If a later compiler stops needing one, delete it and take the plainer code:
`tests/codegen.rs` says which ones still earn their keep, and the numbers below say what is at
stake if one silently stops working.

Measured 2026-09-07 on rustc 1.98.1 / LLVM 22.1.8, one core of an AMD Ryzen 7 9800X3D. Node
counts are the whole graph; `theta = 0` is the exact O(n²) pass and `0.9` the Barnes-Hut default.

## The repulsion kernel

`repulsion_on` dominates a step. It is plain scalar Rust over `[f32; LANES]` chunks, written so
LLVM's SLP vectorizer widens it; the same source runs on x86-64, aarch64 and wasm.

Three spellings in it are load-bearing.

**The clamp has one body per architecture.** `f32::max` and `f32::min` are IEEE `maxNum` and
`minNum`, which return the *other* operand when one side is NaN. `maxps`, `minps` and
`f32x4.pmax`/`pmin` do the opposite, so on x86 and wasm LLVM emits a `cmpunord` and a `blend`
after each one to correct it. aarch64 is the only architecture with the IEEE rule in hardware, as
`fmaxnm`/`fminnm`, so there it is the comparisons that need the second instruction. Neither
spelling is good on both, and `clamp_symmetric` picks between `clamp_ieee` and
`clamp_comparisons` on `target_arch`. `f32::clamp` is not a third option — same NaN rule as the
methods, and a branch for `min > max` besides.

Cycles per iteration of the vector body, which is eight interactions on every model below, from
`llvm-mca` over the loop the compiler actually emitted. The chosen body is in bold:

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
floor every desktop build has to clear. aarch64 goes the other way on all twelve of its
scheduling models, by 4.7% to 13.7%, the newest big cores gaining most.

The two bodies agree on every input except two. A NaN `limit` the comparisons propagate and
`minNum` discards; nothing in the crate produces one — `limit` is `force_max`, or a multiple of it
— and `clamp_symmetric` documents the argument as excluding it. And the two disagree on which
zero they return, but only when running on aarch64, because `fmaxnm` and `fminnm` do not pick the
same one the comparisons do; a force of `-0.0` moves a node exactly as far as one of `0.0`, so
`the_two_clamp_bodies_agree` compares zeroes by value.

The zero disagreement is invisible from x86 — the same test passes there bit for bit — and was
found by `just test-arm`, which runs these tests on armv8 under qemu. Both bodies are compiled
everywhere so one host can check them against each other, but a property that only appears on the
hardware needs the hardware. `tests/codegen.rs` covers the separate question of whether each target
got the body meant for it.

`llvm-mca` has no wasm model, and a browser's own backend is what picks the final instructions
anyway; the x86 rows are the closest available reading of what a desktop browser ends up running.

Those are static estimates of one loop; the millisecond tables below are the same comparison run
as a whole simulation. Where they disagree, the milliseconds are what happened.

**The lane loop iterates `as_chunks`, not a running index.** Five slices indexed by a common
`base` leaves a bounds check inside the vector body: LLVM does not prove `base + LANES <= n` from
`body = n - n % LANES`, even having just computed `body` as `n & !(LANES - 1)`. A
`&[[f32; LANES]]` carries the count in its type instead, so there is nothing to prove. The
remainder that `as_chunks` hands back is statically shorter than `LANES`, which also turns the
scalar tail from a loop into a peeled sequence.

**`repulsion_on` is not `#[inline]`.** It was, against the lane loop going scalar without it. That
is no longer true — the out-of-line copy vectorizes identically on x86-64 and wasm — and the
function is entered once per node, so the call is already amortized over that node's interactions.

What each is worth, one change at a time, from `just bench`. Milliseconds per step of the whole
simulation; the clamp column is each architecture built with the body meant for the other one, and
the lanes column is the running-index loop the bounds check survives in. The phone is a moto g
power 5G (2024), a Dimensity 7020, with each run pinned to one core of a cluster.

Exact summation, 4000 nodes:

| | shipped | other clamp | indexed lanes |
|---|---|---|---|
| Ryzen 9800X3D, `target-cpu=x86-64` | 11.5 | 27.1 | 49.4 |
| Ryzen 9800X3D, `target-cpu=native` | 5.5 | 9.3 | 45.6 |
| Cortex-A55, 2.0 GHz | 143.1 | 161.0 | 692.0 |
| Cortex-A76, 2.2 GHz | 40.4 | 61.7 | 177.8 |

Barnes-Hut at the default angle, 16000 nodes, which is the shape the application runs:

| | shipped | other clamp | indexed lanes |
|---|---|---|---|
| Ryzen 9800X3D, `target-cpu=x86-64` | 8.4 | 15.0 | 24.7 |
| Ryzen 9800X3D, `target-cpu=native` | 5.8 | 7.4 | 22.3 |
| Cortex-A55, 2.0 GHz | 127.4 | 131.9 | 364.9 |
| Cortex-A76, 2.2 GHz | 28.0 | 37.7 | 86.4 |

The bounds check is by a distance the largest of the three, 3x to 8x, and the one nothing about
the source suggests: the indexed loop reads identically and differs only in what LLVM can prove
about it. It costs most where the vectors are widest, because what it costs is the vectorization
rather than the check.

The clamp is 1.3x to 2.4x on the exact pass, and the two aarch64 rows are why there are two bodies
at all. The Cortex-A76 figure, 1.5x, is far from the 10% `llvm-mca` estimates for the aarch64 cores
it does model — it has no A76 model, and that is the core the difference matters most on. The A55
row moves by several points between runs, so read it as around a tenth rather than as 12.6%.

Barnes-Hut dilutes all of it, because the octree walk around the kernel is untouched either way.
That is the honest number for the application, and it is still 1.35x on a phone's big core.

## The optimization level of the shipping build

`[profile.min]` in the workspace manifest is `opt-level = 3` rather than a size level, and that
is worth more than every source-level change above put together.

The lane loop is a fixed-trip inner loop, so it has to be unrolled to `LANES` before the SLP
vectorizer can see anything to widen. Both `"s"` and `"z"` turn the unroller off. Counting
`simd128` instructions in a small wasm module that does nothing but step a graph:

| profile | instructions |
|---|---|
| `opt-level = "z"` | 9 |
| `opt-level = "s"` | 16 |
| `opt-level = 3` | 251 |

Confining the speed level to this one crate with `[profile.min.package.force_graph_3d]` does not
work under LTO. Fat and thin both defer the vectorizers to the post-link pipeline, which runs at
whatever the top-level `opt-level` says; the same module scores 20 either way. Only with LTO off
does a per-package override take effect. For the same reason, `--emit=llvm-ir` on an LTO build
shows pre-link bitcode that has not been vectorized at any optimization level, so `tests/codegen.rs`
builds without LTO to look at the shape of the kernel and checks the manifest separately.

The whole application, built for wasm32 with `+simd128`. "Kernel" is whether `repulsion_on` in
the emitted wasm holds `f32x4` instructions or scalar ones.

| profile | wasm | brotli | kernel |
|---|---|---|---|
| `"s"`, fat LTO | 7.68 MB | 2.260 MB | scalar |
| `3`, fat LTO | 9.17 MB | 2.444 MB | vector |
| `3`, no LTO | 8.79 MB | 2.435 MB | vector |
| `"s"`, no LTO, package override | 8.31 MB | 2.299 MB | vector |
| `"z"`, no LTO, package override | 8.01 MB | 2.242 MB | vector |

The last row is smaller after brotli than any of the others and still gets the vectorized kernel.
What it gives up is cross-crate inlining for everything else in the binary, which is mostly
rendering and does not show in a size measurement; that is why the shipped profile is the second
row and not the last. Fat LTO is not paying for itself on size either way — 374 KB more raw wasm
than the same profile without it, 9 KB less after brotli.

## Left on the table

**A reciprocal square root estimate.** `(d2 + SOFTENING).sqrt().recip()` is the two
highest-latency vector operations the machine has, chained, in a loop that is otherwise multiplies
and adds. x86 has `rsqrtps` and aarch64 has `frsqrte`, both around 12 bits; one Newton-Raphson step
brings either to within a bit or two of exact, far inside the ~1% the Barnes-Hut approximation
already costs.

Substituted into the emitted loop and measured, it is mostly not worth having. Cycles per
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

Latency is not what binds here. On Skylake the loop is limited by ports 0 and 1, which the
estimate's extra multiplies land on too, so its block throughput goes *up*, 12.5 to 14.5, while the
divider it frees was never full. The divide and square root are pipelined on everything modern,
with enough independent work either side to cover the latency. Two Newton-Raphson steps are a loss
everywhere. One is a gain on exactly one class of aarch64 core, and a loss on the newest ones and
on both in-order cores, where trading two instructions for four costs more than the operations
saved — `cortex-a510` and `cortex-a55` have 64-bit NEON and the least headroom, so the machines
that would most want this are the ones it hurts most.

Two more things to weigh. wasm has no such instruction at all — its SIMD is required to be
bit-deterministic — so the browser build, the one under the most pressure, cannot have it. And LLVM
already knows how to make this substitution: it is gated behind fast-math flags that Rust has no
stable way to apply to a single expression. If that ever lands, the plain source becomes the fast
source, and hand-written intrinsics would be in the way.

**A wider `LANES` on AVX-512.** `LANES` is 8, which fills a `ymm`. The machine this was measured
on has `avx512f`, but nothing the application ships is built for it.

## Checking by hand

`tests/codegen.rs` covers the three properties above automatically, and `just bench <nodes>
<theta> <steps>` times each of them against the spelling it was chosen over, here and on a phone
over adb; that is where the millisecond tables came from. It builds through `tools/variant.py`,
which writes the alternatives into a scratch copy of the crate, so none of them has to live in the
kernel behind a `cfg`. An alternative whose anchor text no longer matches is an error there rather
than a silent no-op: the kernel moved, and whatever the number measured wants remeasuring.

To look at the code directly:

```sh
# x86-64 assembly, Intel syntax, for a chosen microarchitecture level
RUSTFLAGS="-C target-cpu=x86-64-v3" cargo rustc --release -p force_graph_3d \
  -- --emit asm -C llvm-args=--x86-asm-syntax=intel

# cycles for the vector body, given the block cut out of one of those .s files. Every number in
# this file came from this. `-mcpu=cortex-a76` silently falls back to the Cortex-A57 model and
# reports a non-pipelined divider that no A76 has, so it is not among them.
llvm-mca -mcpu=skylake -iterations=1000 -bottleneck-analysis loop.s

# the emitted wasm of the real application
just dist && wasm-objdump -d dist/*.wasm | grep -E 'f32x4|v128\.'
```

An `rlib` on its own instantiates nothing, because the kernel is only reachable through a generic
type: give the crate a non-generic entry point that steps a graph, or read the IR of a consumer.
