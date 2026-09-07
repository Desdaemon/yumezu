#!/usr/bin/env bash
# Times the kernel's spellings against each other, on this machine and on a phone if one is
# attached. `IMPL_DETAILS.md` carries the results.
#
# `llvm-mca` costs a loop from a scheduling model. This runs the simulation instead, which is the
# only way to see the memory system, the octree walk around the kernel, and the several shipping
# cores the model has nothing for. `tools/variant.py` writes each variant to a scratch copy of the
# crate, so nothing here touches the working tree.
#
# Args are the benchmark's: `<nodes> <theta> <steps>`. Wants ANDROID_HOME and a device on adb for
# the phone half, and skips it without them.
set -euo pipefail

readonly TRIPLE=aarch64-linux-android
readonly MIN_SDK=24
readonly REMOTE=/data/local/tmp
readonly VARIANTS=(shipped clamp indexed)

cd "$(dirname "$0")/.."
readonly ROOT=$PWD
readonly SCRATCH=${TMPDIR:-/tmp}/force_graph_3d-bench

# The desktop build sets no `target-cpu`, so SSE2 is the floor it actually ships; this machine's
# own is whatever it has, and the gap between the two columns is what the wider vectors are worth.
readonly HOST_CPUS=(x86-64 native)

for variant in "${VARIANTS[@]}"; do
    python3 tools/variant.py "$variant" "$SCRATCH/$variant"
done

echo "host: $(lscpu | sed -n 's/^Model name: *//p' | head -1)"
for cpu in "${HOST_CPUS[@]}"; do
    echo "  -C target-cpu=$cpu"
    for variant in "${VARIANTS[@]}"; do
        cd "$SCRATCH/$variant"
        RUSTFLAGS="-C target-cpu=$cpu" cargo build --release --quiet --example bench
        printf '    %-9s %s\n' "$variant" "$(./target/release/examples/bench "$@")"
        cd "$ROOT"
    done
done

sdk=${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}
ndk=$sdk/ndk/$(ls "$sdk/ndk" 2>/dev/null | sort -V | tail -1)
clang=$ndk/toolchains/llvm/prebuilt/linux-x86_64/bin/$TRIPLE$MIN_SDK-clang
if [[ ! -x $clang ]] || ! adb get-state >/dev/null 2>&1; then
    echo "no device or no ndk under $sdk; skipping the phone"
    exit 0
fi
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=$clang

for variant in "${VARIANTS[@]}"; do
    cd "$SCRATCH/$variant"
    cargo build --release --quiet --example bench --target $TRIPLE
    adb push -q "target/$TRIPLE/release/examples/bench" "$REMOTE/bench-$variant" >/dev/null
    adb shell chmod 755 "$REMOTE/bench-$variant"
    cd "$ROOT"
done

# One core from each cluster, read off the device rather than assumed, because a phone's cores are
# not alike and the scheduler will move a short run between them.
mapfile -t freqs < <(adb shell 'cat /sys/devices/system/cpu/cpu*/cpufreq/cpuinfo_max_freq' | tr -d '\r')
slow=0 fast=0
for i in "${!freqs[@]}"; do
    ((freqs[i] < freqs[slow])) && slow=$i
    ((freqs[i] > freqs[fast])) && fast=$i
done

echo "device: $(adb shell getprop ro.product.model | tr -d '\r')"
for core in $slow $fast; do
    echo "  cpu$core, ${freqs[core]} kHz"
    for variant in "${VARIANTS[@]}"; do
        # Best of three: a phone throttles, and the fastest run is the one least contaminated by
        # whatever else the device decided to do.
        best=$(for _ in 1 2 3; do
            adb shell "taskset $(printf '%x' $((1 << core))) $REMOTE/bench-$variant $*" | tr -d '\r'
        done | sort -t' ' -k5 -n | head -1)
        printf '    %-9s %s\n' "$variant" "$best"
    done
done
adb shell rm -f "$REMOTE"/bench-{shipped,clamp,indexed}
