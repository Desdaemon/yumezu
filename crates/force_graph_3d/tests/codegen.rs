//! Asserts that the repulsion kernel's LLVM IR stays vectorized across refactors and toolchain
//! changes. A failure costs speed, not correctness; a fix may drop a stale workaround.

// Drives a compiler, so host-only; `just test-wasm` runs the behaviour tests.
#![cfg(not(target_family = "wasm"))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The kernel compiled for one target and set of flags, as LLVM IR.
///
/// The kernel is generic, so an `rlib` alone instantiates nothing; this builds a throwaway consumer
/// that steps a graph and reads the IR of the compilation unit holding the definition.
fn kernel_ir(target: Option<&str>, rustflags: &str, kernel: &str) -> Option<String> {
    let dir = scratch().join(match target {
        Some(target) => format!("codegen-{target}"),
        None => "codegen-host".into(),
    });
    let crate_dir = dir.join("driver");
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    let force_graph = Path::new(env!("CARGO_MANIFEST_DIR")).display().to_string();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        format!(
            r#"
[workspace]
[package]
name = "driver"
version = "0.0.0"
edition = "2024"
[lib]
crate-type = ["rlib"]
[dependencies]
force_graph_3d = {{ path = "{force_graph}" }}
# Without LTO: under it the per-crate IR is pre-link bitcode the vectorizers have not run on.
[profile.release]
opt-level = 3
lto = false
codegen-units = 1
"#
        ),
    )
    .unwrap();
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        r#"
use force_graph_3d::*;
#[unsafe(no_mangle)]
pub extern "C" fn drive(graph: &mut ForceGraph, dt: f32) -> bool { graph.update(dt) }
"#,
    )
    .unwrap();

    // Cargo re-emits nothing for a crate considered fresh.
    let mut clean = Command::new(env!("CARGO"));
    clean
        .current_dir(&crate_dir)
        .args(["clean", "--release", "--quiet", "-p", "force_graph_3d"])
        .env("CARGO_TARGET_DIR", dir.join("target"));
    if let Some(target) = target {
        clean.args(["--target", target]);
    }
    let _ = clean.status();

    let mut cargo = Command::new(env!("CARGO"));
    cargo
        .current_dir(&crate_dir)
        .args(["rustc", "--release", "--quiet"])
        .env("CARGO_TARGET_DIR", dir.join("target"))
        // Reaches every crate, so the dependency emits IR too.
        .env("RUSTFLAGS", format!("{rustflags} --emit=llvm-ir"));
    if let Some(target) = target {
        cargo.args(["--target", target]);
    }
    let output = cargo.output().expect("cargo");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // A missing target skips locally and fails on CI.
        let missing = target
            .is_some_and(|t| stderr.contains(&format!("the `{t}` target may not be installed")));
        if missing && std::env::var_os("CI").is_none() {
            eprintln!("skipped: {} is not installed", target.unwrap());
            return None;
        }
        panic!("{stderr}");
    }

    // Newest first: the scratch target directory keeps stale IR from earlier revisions.
    let mut emitted: Vec<PathBuf> = files_under(&dir.join("target"))
        .filter(|p| p.extension().is_some_and(|e| e == "ll"))
        .collect();
    emitted.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let define = emitted
        .iter()
        .rev()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .find_map(|ir| function(&ir, kernel))
        .unwrap_or_else(|| {
            panic!(
                "no `define` for `{kernel}` in any emitted IR: it was inlined into every caller, \
                 and this probe needs rewriting to look at the callers instead"
            )
        });
    Some(define)
}

/// A build directory outside the workspace target directory, whose lock the outer `cargo test` may
/// still hold.
fn scratch() -> PathBuf {
    std::env::temp_dir().join("force_graph_3d-codegen")
}

fn files_under(root: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Box::new(std::iter::empty());
    };
    Box::new(entries.filter_map(Result::ok).flat_map(|e| {
        let path = e.path();
        if path.is_dir() {
            files_under(&path)
        } else {
            Box::new(std::iter::once(path))
        }
    }))
}

/// Mangled names prefix each path segment with the segment length, so `repulsion_on` misses
/// `repulsion_on_avx2`.
fn function(ir: &str, name: &str) -> Option<String> {
    let segment = format!("{}{name}", name.len());
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains(&segment))?;
    let block: Vec<&str> = ir.lines().skip(start).take_while(|l| *l != "}").collect();
    Some(block.join("\n"))
}

/// A bounds check left in the middle of the loop would make `as_chunks` pointless.
#[track_caller]
fn assert_vectorized(ir: &str, width: usize) {
    let vector = format!("<{width} x float>");
    assert!(
        ir.contains(&vector),
        "the lane loop is scalar: no {vector} in the emitted IR"
    );
    assert!(
        !ir.contains("slice_index_fail"),
        "a bounds check is back in the lane loop"
    );
}

/// Each `clamp_symmetric` variant lowers to bare instructions only on the matching architecture; a
/// `cfg` reaching the wrong target is silent and costs about a tenth of the loop.
#[track_caller]
fn assert_clamp_is_ieee(ir: &str, expected: bool) {
    // `f32::max`/`f32::min` lower to `maximumnum`/`minimumnum`; `maxnum`/`minnum` are equivalent.
    let ieee = ["maximumnum", "minimumnum", "maxnum", "minnum"]
        .iter()
        .any(|name| ir.contains(name));
    assert_eq!(
        ieee,
        expected,
        "{}",
        if expected {
            "the clamp is comparisons here, which aarch64 needs two instructions for where \
             `fmaxnm`/`fminnm` would do"
        } else {
            "the clamp is IEEE min/max here, which costs a NaN test and a select per bound"
        }
    );
}

/// The browser build, which holds a frame rate on the weakest hardware the application runs on.
/// `simd128` is off by default and set in `.cargo/config.toml`.
#[test]
fn the_lane_loop_vectorizes_on_wasm() {
    let Some(ir) = kernel_ir(
        Some("wasm32-unknown-unknown"),
        "-C target-feature=+simd128",
        "repulsion_on",
    ) else {
        return;
    };
    // wasm `v128` is four lanes wide whatever `LANES` is.
    assert_vectorized(&ir, 4);
    assert_clamp_is_ieee(&ir, false);
}

/// The Android build. NEON is baseline for this triple, so there is no target feature to set, and
/// the probe crate is an rlib that never reaches a linker, so the NDK is not needed.
#[test]
fn the_lane_loop_vectorizes_on_arm() {
    let Some(ir) = kernel_ir(Some("aarch64-linux-android"), "", "repulsion_on") else {
        return;
    };
    assert_vectorized(&ir, 4);
    assert_clamp_is_ieee(&ir, true);
}

/// The desktop and Android builds, at the baseline the toolchain picks by default.
#[test]
fn the_lane_loop_vectorizes_on_the_host() {
    let ir = kernel_ir(None, "", "repulsion_on").expect("the host target is always installed");
    // SSE2 and NEON give four lanes; AVX2 gives eight.
    assert!(
        ir.contains("<4 x float>") || ir.contains("<8 x float>"),
        "the lane loop is scalar: no vector type in the emitted IR"
    );
    assert_vectorized(&ir, if ir.contains("<8 x float>") { 8 } else { 4 });
    assert_clamp_is_ieee(&ir, cfg!(target_arch = "aarch64"));
}

/// The copy the desktop build dispatches to on a CPU with AVX2.
#[cfg(target_arch = "x86_64")]
#[test]
fn the_lane_loop_vectorizes_with_avx2() {
    let ir = kernel_ir(None, "", "repulsion_on_avx2").expect("the host target is always installed");
    assert_vectorized(&ir, 8);
    assert_clamp_is_ieee(&ir, false);
}

/// Vectorizing the lane loop needs the unroller to widen the loop to `LANES`, and both size levels
/// disable the unroller. So the `min` profile needs a speed level, or this package does with LTO
/// off: both LTO modes run the vectorizers after linking, at the top-level setting.
#[test]
fn the_shipping_profile_builds_this_crate_for_speed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml");
    let manifest = std::fs::read_to_string(&root).expect("workspace manifest");
    let profile = |header: &str| -> Option<String> {
        Some(
            manifest
                .split(&format!("{header}\n"))
                .nth(1)?
                .lines()
                .take_while(|l| !l.trim_start().starts_with('['))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    };
    let setting = |section: &Option<String>, key: &str| -> Option<String> {
        section.as_ref()?.lines().find_map(|l| {
            let (name, value) = l.split_once('=')?;
            (name.trim() == key).then(|| value.trim().trim_matches('"').to_string())
        })
    };
    let for_speed = |level: Option<String>| matches!(level.as_deref(), Some("2" | "3"));

    let min = profile("[profile.min]");
    let package = profile("[profile.min.package.force_graph_3d]");
    let whole_profile = for_speed(setting(&min, "opt-level"));
    let this_package = for_speed(setting(&package, "opt-level"))
        && setting(&min, "lto").as_deref() == Some("false");

    assert!(
        whole_profile || this_package,
        "{} compiles force_graph_3d for size, which makes the hot loops one lane wide. Either \
         give [profile.min] a speed opt-level, or give [profile.min.package.force_graph_3d] one \
         and set lto = false so that it is not overridden after linking.",
        root.display()
    );
}
