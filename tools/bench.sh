#!/usr/bin/env bash
# Times the kernel's variants here and on an adb phone. `IMPL_DETAILS.md` carries the results.
# Args: `<nodes> <theta> <steps>`. Skips the phone without ANDROID_HOME and a device.
set -euo pipefail

readonly TRIPLE=aarch64-linux-android
readonly MIN_SDK=24
readonly REMOTE=/data/local/tmp
readonly VARIANTS=(shipped clamp indexed)

cd "$(dirname "$0")/.."
readonly ROOT=$PWD
readonly SCRATCH=${TMPDIR:-/tmp}/force_graph_3d-bench

# The desktop build ships with no `target-cpu`, so SSE2.
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

# A phone's clusters differ and the scheduler migrates short runs between them.
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
        # Best of three: a phone throttles.
        best=$(for _ in 1 2 3; do
            adb shell "taskset $(printf '%x' $((1 << core))) $REMOTE/bench-$variant $*" | tr -d '\r'
        done | sort -t' ' -k5 -n | head -1)
        printf '    %-9s %s\n' "$variant" "$best"
    done
done
adb shell rm -f "$REMOTE"/bench-{shipped,clamp,indexed}
