//! Guards the shape of the code LLVM emits for the repulsion kernel.
//!
//! The kernel is ordinary safe Rust that happens to be spelled the way the auto-vectorizer wants
//! it. Nothing in the language enforces that, so a refactor, a profile change or a new compiler
//! can quietly drop it back to one lane at a time; the layout still looks right and the tests
//! still pass, only slower. Each check below builds the crate the way something really ships it
//! and asserts on the LLVM IR that comes out. See `IMPL_DETAILS.md`.
//!
//! Every failure here is about speed, not correctness, and the fix may well be to delete the
//! workaround the check is guarding once the compiler no longer needs it.

// Drives a compiler and reads what it wrote, so it is a host-only test; `just test-wasm` runs
// the behaviour tests under a wasm runtime, not this one.
#![cfg(not(target_family = "wasm"))]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The kernel compiled for one target and set of flags, as LLVM IR.
///
/// The crate reaches the kernel only through a generic type, so an `rlib` on its own instantiates
/// nothing. This builds a throwaway consumer that steps a graph, the way the application does,
/// and takes the IR of whichever compilation unit the definition landed in.
fn kernel_ir(target: Option<&str>, rustflags: &str) -> Option<String> {
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
# Deliberately not the shipping profile's `lto = true`. Under fat LTO the per-crate IR is
# pre-link bitcode that the vectorizers have not run on yet, whatever the optimization level, so
# there would be nothing here to look at. What these checks are for is the way the kernel is
# written, which is the same either way; that the shipping profile still lets the vectorizers
# run is the separate check below.
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

    // Cargo would otherwise call the crate fresh and re-emit nothing, leaving whatever IR the
    // last run of this test happened to produce.
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
        // Reaches every crate in the graph, so the dependency emits IR too.
        .env("RUSTFLAGS", format!("{rustflags} --emit=llvm-ir"));
    if let Some(target) = target {
        cargo.args(["--target", target]);
    }
    let output = cargo.output().expect("cargo");
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // A target the developer has not installed is not a failure of this crate. On CI it is:
        // skipping there would mean this check quietly stops checking anything.
        let missing = target
            .is_some_and(|t| stderr.contains(&format!("the `{t}` target may not be installed")));
        if missing && std::env::var_os("CI").is_none() {
            eprintln!("skipped: {} is not installed", target.unwrap());
            return None;
        }
        panic!("{stderr}");
    }

    // Newest first: the scratch target directory outlives one run and cargo never cleans it, so
    // IR from an earlier build of a different shape can still be lying there. A build that
    // changed nothing rewrites nothing, which leaves the newest file the current one either way.
    let mut emitted: Vec<PathBuf> = walk(&dir.join("target"))
        .filter(|p| p.extension().is_some_and(|e| e == "ll"))
        .collect();
    emitted.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
    let define = emitted
        .iter()
        .rev()
        .filter_map(|f| std::fs::read_to_string(f).ok())
        .find_map(|ir| function(&ir, "repulsion_on"))
        .expect(
            "no `define` for `repulsion_on` in any emitted IR: it was inlined into every caller, \
             and this probe needs rewriting to look at the callers instead",
        );
    Some(define)
}

/// Somewhere to build the throwaway crate that is not the workspace target directory: the outer
/// `cargo test` may still hold its lock.
fn scratch() -> PathBuf {
    std::env::temp_dir().join("force_graph_3d-codegen")
}

fn walk(root: &Path) -> Box<dyn Iterator<Item = PathBuf>> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Box::new(std::iter::empty());
    };
    Box::new(entries.filter_map(Result::ok).flat_map(|e| {
        let path = e.path();
        if path.is_dir() {
            walk(&path)
        } else {
            Box::new(std::iter::once(path))
        }
    }))
}

/// The body of the one `define` whose mangled name contains `name`.
fn function(ir: &str, name: &str) -> Option<String> {
    let start = ir
        .lines()
        .position(|l| l.starts_with("define") && l.contains(name))?;
    let body: Vec<&str> = ir
        .lines()
        .skip(start)
        .take_while(|l| *l != "}")
        .collect();
    Some(body.join("\n"))
}

/// The lane loop has to come out as vector instructions, with no bounds check left in the middle
/// of it for `as_chunks` to have been pointless.
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

/// `clamp_symmetric` has one body per architecture and they are not interchangeable: each is the
/// one that lowers to bare instructions there and the other is the one that needs correcting
/// after. A `cfg` reaching the wrong target is silent and costs about a tenth of the loop, so
/// which body arrived is checked rather than assumed.
#[track_caller]
fn assert_clamp_is_ieee(ir: &str, expected: bool) {
    // `maximumnum`/`minimumnum` are what `f32::max`/`f32::min` lower to; the older `maxnum` and
    // `minnum` spellings mean the same thing and would be just as wrong on the targets that do
    // not want them.
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
            "the clamp is IEEE min/max here, which costs a compare and a blend per bound"
        }
    );
}

/// The browser build, which is the one that has to hold a frame rate on the weakest hardware the
/// application runs on. `simd128` is off by default and set in `.cargo/config.toml`.
#[test]
fn the_lane_loop_vectorizes_on_wasm() {
    let Some(ir) = kernel_ir(
        Some("wasm32-unknown-unknown"),
        "-C target-feature=+simd128",
    ) else {
        return;
    };
    // wasm `v128` is four lanes wide whatever `LANES` is; the loop is unrolled to fill it.
    assert_vectorized(&ir, 4);
    assert_clamp_is_ieee(&ir, false);
}

/// The Android build. `android/build.sh` ships this triple, and NEON is baseline for it, so there
/// is no target feature to set. Only the IR is read, so this needs the target installed but not
/// the NDK: the probe crate is an rlib and never reaches a linker.
#[test]
fn the_lane_loop_vectorizes_on_arm() {
    let Some(ir) = kernel_ir(Some("aarch64-linux-android"), "") else {
        return;
    };
    assert_vectorized(&ir, 4);
    assert_clamp_is_ieee(&ir, true);
}

/// The desktop and Android builds, at the baseline the toolchain picks by default.
#[test]
fn the_lane_loop_vectorizes_on_the_host() {
    let ir = kernel_ir(None, "").expect("the host target is always installed");
    // Four lanes is what the baseline of every target the application ships on provides: SSE2 on
    // x86-64, NEON on aarch64. A machine with AVX2 gets eight, and `<4 x float>` would then be
    // missing, so this asks for whichever the build chose.
    assert!(
        ir.contains("<4 x float>") || ir.contains("<8 x float>"),
        "the lane loop is scalar: no vector type in the emitted IR"
    );
    assert_vectorized(&ir, if ir.contains("<8 x float>") { 8 } else { 4 });
    // Whichever body this host's own architecture selects, which is the aarch64 one when the
    // machine running the tests is itself aarch64.
    assert_clamp_is_ieee(&ir, cfg!(target_arch = "aarch64"));
}

/// The other half of what makes the browser build fast, and the half that is invisible from
/// inside this crate. Vectorizing the lane loop needs the unroller to widen it to `LANES` first,
/// and both size-optimizing levels turn the unroller off, so something in the workspace manifest
/// has to compile this crate for speed. There are two arrangements that do, and this accepts
/// either: a speed level for the whole `min` profile, or one for this package alone, which only
/// takes effect with LTO off because both LTO modes run the vectorizers after linking at the
/// top-level setting.
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
    let this_package =
        for_speed(setting(&package, "opt-level")) && setting(&min, "lto").as_deref() == Some("false");

    assert!(
        whole_profile || this_package,
        "{} compiles force_graph_3d for size, which makes the hot loops one lane wide. Either \
         give [profile.min] a speed opt-level, or give [profile.min.package.force_graph_3d] one \
         and set lto = false so that it is not overridden after linking.",
        root.display()
    );
}
