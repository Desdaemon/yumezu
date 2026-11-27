//! Frame cost logging for `profile` builds, logged and never drawn. GPU timing on the web build
//! needs `EXT_disjoint_timer_query_webgl2`; see [`readable`].

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use three_d::renderer::*;

use super::AppEntities;
use super::camera::AppPersistent;

/// How often [`FrameStats`] logs a report.
const FRAME_STATS_SECONDS: f64 = 2.0;

/// Timing the passes appends one column per pass.
const COLUMNS: [&str; 9] = [
    "fps",
    "p50 ms",
    "p99 ms",
    "worst ms",
    "physics ms",
    "gpu ms",
    "draws",
    "rebuilt",
    "wall",
];
/// One wider than the longest name in [`COLUMNS`].
const COLUMN: usize = 11;
const HEADER_SECONDS: f64 = 30.0;

fn row<'a>(cells: impl IntoIterator<Item = &'a str>) -> String {
    cells
        .into_iter()
        .map(|it| format!("{it:>COLUMN$}"))
        .collect()
}

/// Whether a switch below starts on, for an unattended session: named in the comma-separated
/// `YUMEZU_FRAMES`, or in `?frames=` on the web build.
fn named(switch: &str) -> bool {
    #[cfg(not(target_family = "wasm"))]
    let frames = std::env::var("YUMEZU_FRAMES").ok();
    #[cfg(target_family = "wasm")]
    let frames = super::link::frames();
    frames.is_some_and(|frames| frames.split(',').any(|it| it.trim() == switch))
}

/// A measured session's camera pose, as the six numbers the `camera in ... dimensions` log line
/// prints: `eye` then `target`, from `YUMEZU_CAMERA`, or `?camera=` on the web build.
fn named_pose() -> Option<(Vec3, Vec3)> {
    #[cfg(not(target_family = "wasm"))]
    let named = std::env::var("YUMEZU_CAMERA").ok()?;
    #[cfg(target_family = "wasm")]
    let named = super::link::camera()?;
    let numbers: Vec<f32> = named
        .split(',')
        .filter_map(|it| it.trim().parse().ok())
        .collect();
    let [ex, ey, ez, ax, ay, az] = numbers[..] else {
        log::warn!("{named:?} is not six numbers, so the camera is framed on the graph instead");
        return None;
    };
    Some((vec3(ex, ey, ez), vec3(ax, ay, az)))
}

/// GPU timers cannot nest, so the whole-frame timer is off while passes are timed.
static PARTS: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(named("parts")));
/// Draws the old way, at the display's rate with geometry and dashes rebuilt every frame, to
/// compare both in one binary. A lower bound on the old cost.
static EAGER: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(named("eager")));
static DOLLY: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(named("dolly")));

/// Stored, since vsync is fixed when the surface is made: the switch takes effect next session.
/// Native build only; see [`unlocked`].
#[cfg(not(target_family = "wasm"))]
const UNLOCKED: &str = "frames-unlocked";

pub(super) fn eager() -> bool {
    EAGER.load(Ordering::Relaxed)
}

/// Latched at the first read, when the surface is made. Always off on the web build, where the
/// browser paces frames through `requestAnimationFrame`.
pub(super) fn unlocked() -> bool {
    #[cfg(target_family = "wasm")]
    return false;
    #[cfg(not(target_family = "wasm"))]
    {
        static RUNNING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNING.get_or_init(|| super::store::read_switch(UNLOCKED) == Some(true))
    }
}

/// Labels are untranslated: `profile` builds never ship to users.
pub(super) fn controls(ui: &mut egui::Ui) {
    ui.separator();
    switch(
        ui,
        &PARTS,
        "time each pass",
        "Moves the GPU's clock off the whole frame and onto the passes one at a time. The \
         whole-frame figure goes quiet while this is on.",
    );
    switch(
        ui,
        &EAGER,
        "draw every frame",
        "Redraws at the display's rate whether or not anything moved, which is how the app \
         worked before frames were drawn on demand.",
    );
    switch(
        ui,
        &DOLLY,
        "pan on a schedule",
        "Frames the whole graph, or the pose YUMEZU_CAMERA names, and steps the camera sideways \
         and back once a report, so two windows differ by what they were drawing and nothing \
         else.",
    );
    // The browser paces frames on the web build.
    #[cfg(not(target_family = "wasm"))]
    {
        let mut unlock = super::store::read_switch(UNLOCKED) == Some(true);
        if ui
            .checkbox(&mut unlock, "run past the display")
            .on_hover_text("Turns vsync off, which is the only way to watch the GPU's cost move.")
            .changed()
        {
            super::store::write_switch(UNLOCKED, unlock);
        }
        if unlock != unlocked() {
            ui.label("restart to apply");
        }
    }
}

fn switch(ui: &mut egui::Ui, flag: &AtomicBool, label: &str, hint: &str) {
    let mut on = flag.load(Ordering::Relaxed);
    if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
        flag.store(on, Ordering::Relaxed);
    }
}

/// How long [`pan_aside`] waits for the layout to stop growing before framing anything.
const PAN_ASIDE_SETTLES_SECONDS: f64 = 8.0;
/// Pan reach to either side of the framed middle, as a fraction of the framed radius. Worlds pile
/// up near the middle, so a window at the rim times empty space.
const PAN_ASIDE_REACH: f32 = 0.5;
/// Reports per pan cycle, which divide the reach so sessions of the same length draw the same views
/// in the same order.
const PAN_ASIDE_REPORTS: f32 = 16.0;

/// Frames of queries kept, which bounds how late an answer may arrive. Reading a query the GPU has
/// not reached stalls the GPU being measured.
const GPU_CLOCK_DEPTH: usize = 4;

/// Timer query readers from `dlsym`: on the GL 4.1 context macOS gives, glow calls unloaded
/// pointers and panics. Apple only; elsewhere the GL library is loaded out of `dlsym`'s reach.
#[cfg(target_vendor = "apple")]
mod answers {
    use std::ffi::{c_char, c_void};
    use std::sync::LazyLock;

    type Uiv = unsafe extern "C" fn(u32, u32, *mut u32);
    type Ui64v = unsafe extern "C" fn(u32, u32, *mut u64);

    // SAFETY: the declaration is `dlsym`'s, from `<dlfcn.h>`.
    #[allow(unsafe_code)]
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// Apple's `RTLD_DEFAULT`: every symbol the process has loaded.
    const LOADED: *mut c_void = -2isize as *mut c_void;

    /// `name` must be nul terminated, and `F` must be the symbol's own signature.
    #[allow(unsafe_code)]
    unsafe fn find<F: Copy>(name: &str) -> Option<F> {
        const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
        // SAFETY: documented handle, nul-terminated name; a missing symbol is null.
        let found = unsafe { dlsym(LOADED, name.as_ptr().cast()) };
        // SAFETY: the caller gives the signature, and a function pointer is pointer sized.
        (!found.is_null()).then(|| unsafe { std::mem::transmute_copy(&found) })
    }

    #[allow(unsafe_code)]
    pub(super) static UIV: LazyLock<Option<Uiv>> =
        // SAFETY: the signature is `glGetQueryObjectuiv`'s own.
        LazyLock::new(|| unsafe { find("glGetQueryObjectuiv\0") });
    #[allow(unsafe_code)]
    pub(super) static UI64V: LazyLock<Option<Ui64v>> =
        // SAFETY: the signature is `glGetQueryObjectui64v`'s own.
        LazyLock::new(|| unsafe { find("glGetQueryObjectui64v\0") });
}

/// Whether timer queries can be read, decided before [`GpuClock`] makes any: glow panics on macOS,
/// and the web build reads zero without the extension.
fn readable(context: &Context) -> bool {
    #[cfg(target_vendor = "apple")]
    {
        let _ = context;
        answers::UIV.is_some() && answers::UI64V.is_some()
    }
    /// Owner of `TIME_ELAPSED`; without the extension every query silently reads zero. Only
    /// Chromium on a desktop GPU advertises the extension.
    #[cfg(target_family = "wasm")]
    const TIMER: &str = "EXT_disjoint_timer_query_webgl2";
    #[cfg(target_family = "wasm")]
    {
        context.supported_extensions().contains(TIMER)
    }
    #[cfg(not(any(target_vendor = "apple", target_family = "wasm")))]
    {
        let _ = context;
        true
    }
}

/// # Safety
/// `query` must be the context's own, and `parameter` one the query answers.
#[allow(unsafe_code)]
unsafe fn parameter_u32(context: &Context, query: three_d::context::Query, parameter: u32) -> u32 {
    #[cfg(target_vendor = "apple")]
    {
        let _ = context;
        let mut answer = 0;
        if let Some(uiv) = *answers::UIV {
            // SAFETY: the caller vouched for both arguments, and the answer is one `u32`.
            unsafe { uiv(query.0.get(), parameter, &raw mut answer) };
        }
        answer
    }
    #[cfg(not(target_vendor = "apple"))]
    // SAFETY: the caller vouched for both arguments.
    unsafe {
        context.get_query_parameter_u32(query, parameter)
    }
}

/// # Safety
/// As [`parameter_u32`].
#[allow(unsafe_code)]
unsafe fn parameter_u64(context: &Context, query: three_d::context::Query, parameter: u32) -> u64 {
    #[cfg(target_vendor = "apple")]
    {
        let _ = context;
        let mut answer = 0;
        if let Some(ui64v) = *answers::UI64V {
            // SAFETY: the caller vouched for both arguments, and the answer is one `u64`.
            unsafe { ui64v(query.0.get(), parameter, &raw mut answer) };
        }
        answer
    }
    #[cfg(not(target_vendor = "apple"))]
    // SAFETY: the caller vouched for both arguments.
    unsafe {
        context.get_query_parameter_u64(query, parameter)
    }
}

/// GPU seconds per frame, from timer queries; CPU time misses the wait in the driver.
struct GpuClock {
    context: Context,
    queries: Vec<three_d::context::Query>,
    /// Frames begun; indexes this frame's query and the oldest one to read.
    begun: usize,
}

impl GpuClock {
    /// Holds no queries when the driver refuses them, and the report drops the GPU column.
    fn new(context: &Context) -> Self {
        // SAFETY: names no object and reads no memory.
        #[allow(unsafe_code)]
        let queries = if readable(context) {
            (0..GPU_CLOCK_DEPTH)
                .map_while(|_| unsafe { context.create_query() }.ok())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            context: context.clone(),
            queries,
            begun: 0,
        }
    }

    /// False when the driver refused the queries.
    fn ready(&self) -> bool {
        self.queries.len() == GPU_CLOCK_DEPTH
    }

    fn begin(&self) {
        if !self.ready() {
            return;
        }
        // SAFETY: the query is this context's, and `end` closes each before `begin` opens the next.
        #[allow(unsafe_code)]
        unsafe {
            self.context.begin_query(
                three_d::context::TIME_ELAPSED,
                self.queries[self.begun % GPU_CLOCK_DEPTH],
            );
        }
    }

    /// Ends this frame's query and returns the frame from [`GPU_CLOCK_DEPTH`] ago, if the GPU has
    /// finished that frame; `None` is a missed measurement.
    fn end(&mut self) -> Option<f64> {
        use three_d::context::{QUERY_RESULT, QUERY_RESULT_AVAILABLE, TIME_ELAPSED};
        if !self.ready() {
            return None;
        }
        // SAFETY: `begin` opened this context's query, and every object below is the context's.
        #[allow(unsafe_code)]
        unsafe {
            self.context.end_query(TIME_ELAPSED)
        };
        self.begun += 1;
        // Before the ring fills, the slot holds a never-begun query.
        if self.begun < GPU_CLOCK_DEPTH {
            return None;
        }
        // The next slot is the oldest; query readiness only, as reading an unready answer stalls.
        let oldest = self.queries[self.begun % GPU_CLOCK_DEPTH];
        #[allow(unsafe_code)]
        unsafe {
            (parameter_u32(&self.context, oldest, QUERY_RESULT_AVAILABLE) != 0)
                .then(|| parameter_u64(&self.context, oldest, QUERY_RESULT) as f64 * 1e-9)
        }
    }
}

/// One pass of a frame, timed separately. See [`FrameStats::timed`].
struct Section {
    name: &'static str,
    clock: GpuClock,
    /// Seconds this pass took over the [`Section::answered`] frames. Reset by the report, outside
    /// [`Tally`], so the queries survive.
    spent: f64,
    answered: u32,
}

/// Sums over the current reporting window.
#[derive(Default)]
struct Tally {
    frames: u32,
    /// CPU seconds inside [`super::App::draw`]: the frame without the wait for the display.
    spent: f64,
    /// Frames of [`Tally::frames`] that rebuilt the layout's geometry.
    rebuilt: u32,
    /// GPU seconds over the [`Tally::answered`] frames; see [`GpuClock`].
    gpu: f64,
    answered: u32,
    /// Seconds in the graph layout's force simulation step; part of [`Tally::spent`].
    layout: f64,
    /// Draw calls over [`Tally::frames`]: the scene's submission cost.
    calls: u32,
}

/// Frame costs over a reporting window.
pub(super) struct FrameStats {
    since: web_time::Instant,
    tally: Tally,
    /// Each frame's time in the window, for the percentiles.
    each: Vec<f64>,
    /// Built on the first frame, once the context exists. See [`GpuClock`].
    clock: Option<GpuClock>,
    /// In draw order, which is the order they are first timed in.
    sections: Vec<Section>,
    /// [`PARTS`] latched at frame start: the checkbox flips mid-frame, and GPU timers cannot nest.
    parts: bool,
    headed: Vec<&'static str>,
    /// `None` until the first header is written, so the first report gets one. The web build's
    /// clock starts at page load, too recently to backdate by [`HEADER_SECONDS`].
    headed_at: Option<web_time::Instant>,
}

impl FrameStats {
    pub(super) fn new() -> Self {
        Self {
            since: web_time::Instant::now(),
            tally: Tally::default(),
            each: Vec::new(),
            clock: None,
            sections: Vec::new(),
            parts: false,
            headed: Vec::new(),
            headed_at: None,
        }
    }

    /// `body` returns the draw call count. Every pass goes through here, measured or not, so the
    /// measured frame is the shipped one.
    pub(super) fn timed(
        &mut self,
        name: &'static str,
        context: &Context,
        body: impl FnOnce() -> u32,
    ) {
        if self.parts {
            let which = match self.sections.iter().position(|it| it.name == name) {
                Some(which) => which,
                None => {
                    self.sections.push(Section {
                        name,
                        clock: GpuClock::new(context),
                        spent: 0.0,
                        answered: 0,
                    });
                    self.sections.len() - 1
                }
            };
            self.sections[which].clock.begin();
            let calls = body();
            if let Some(spent) = self.sections[which].clock.end() {
                self.sections[which].spent += spent;
                self.sections[which].answered += 1;
            }
            self.tally.calls += calls;
            return;
        }
        self.tally.calls += body();
    }

    pub(super) fn stepped(&mut self, spent: std::time::Duration) {
        self.tally.layout += spent.as_secs_f64();
    }

    /// Opens the frame the next [`FrameStats::frame`] closes.
    pub(super) fn began(&mut self, context: &Context) {
        self.parts = PARTS.load(Ordering::Relaxed);
        if !self.parts {
            self.clock
                .get_or_insert_with(|| GpuClock::new(context))
                .begin();
        }
    }

    pub(super) fn frame(&mut self, spent: std::time::Duration, rebuilt: bool) {
        if let Some(gpu) = (!self.parts)
            .then(|| self.clock.as_mut().and_then(GpuClock::end))
            .flatten()
        {
            self.tally.gpu += gpu;
            self.tally.answered += 1;
        }
        let spent = spent.as_secs_f64();
        self.tally.frames += 1;
        self.tally.spent += spent;
        self.each.push(spent);
        self.tally.rebuilt += u32::from(rebuilt);
        let window = self.since.elapsed().as_secs_f64();
        if window < FRAME_STATS_SECONDS {
            return;
        }
        let Tally {
            frames,
            spent,
            rebuilt,
            gpu,
            answered,
            layout,
            calls,
        } = std::mem::take(&mut self.tally);
        self.since = web_time::Instant::now();
        self.each.sort_unstable_by(f64::total_cmp);
        let at = |part: f64| self.each[((self.each.len() - 1) as f64 * part).round() as usize];
        let (median, ninety_ninth, worst) = (at(0.5), at(0.99), at(1.0));
        self.each.clear();
        let (passes, timings): (Vec<&str>, Vec<String>) = self
            .sections
            .iter_mut()
            .filter(|it| it.answered > 0)
            .map(|it| {
                let each = (
                    it.name,
                    format!("{:.2}", 1e3 * it.spent / f64::from(it.answered)),
                );
                (it.spent, it.answered) = (0.0, 0);
                each
            })
            .unzip();
        let cells: Vec<(&'static str, String)> = COLUMNS
            .into_iter()
            .zip([
                format!("{:.1}", frames as f64 / window),
                format!("{:.2}", 1e3 * median),
                format!("{:.2}", 1e3 * ninety_ninth),
                format!("{:.2}", 1e3 * worst),
                format!("{:.2}", 1e3 * layout / frames as f64),
                // A dash where the GPU was not timed.
                match answered {
                    0 => "-".to_string(),
                    answered => format!("{:.2}", 1e3 * gpu / answered as f64),
                },
                format!("{:.1}", f64::from(calls) / f64::from(frames)),
                format!("{rebuilt}/{frames}"),
                format!("{:.0}%", 100.0 * spent / window),
            ])
            .chain(passes.iter().copied().zip(timings))
            .collect();
        if self
            .headed_at
            .is_none_or(|at| at.elapsed().as_secs_f64() >= HEADER_SECONDS)
            || self.headed != passes
        {
            log::info!("{}", row(cells.iter().map(|(name, _)| *name)));
            (self.headed, self.headed_at) = (passes, Some(web_time::Instant::now()));
        }
        log::info!("{}", row(cells.iter().map(|(_, it)| it.as_str())));
    }
}

/// Frames the whole graph once, then pans sideways a step per reporting window; see [`DOLLY`].
/// Returns whether the pan drives the view, so the caller requests the next frame.
pub(super) fn pan_aside(persistent: &mut AppPersistent, data: &AppEntities) -> bool {
    if !DOLLY.load(Ordering::Relaxed) {
        return false;
    }
    // Skips the seed blow-out, which nobody views.
    static SINCE: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    let waited = SINCE
        .get_or_init(web_time::Instant::now)
        .elapsed()
        .as_secs_f64()
        - PAN_ASIDE_SETTLES_SECONDS;
    if waited < 0.0 {
        return true;
    }
    // Latched only once a graph exists.
    static FRAMED: std::sync::OnceLock<(Vec3, Vec3, f32)> = std::sync::OnceLock::new();
    let (eye, at, radius) = match FRAMED.get() {
        Some(framed) => *framed,
        None => {
            let framed = match named_pose() {
                // A named pose sets the reach to the eye-target distance.
                Some((eye, at)) => (eye, at, (eye - at).magnitude()),
                None => {
                    let Some(bounds) = data.whole_bounds() else {
                        return true;
                    };
                    persistent.snap_to_frame(&bounds);
                    (
                        persistent.camera.position(),
                        persistent.camera.target(),
                        bounds.radius,
                    )
                }
            };
            let _ = FRAMED.set(framed);
            framed
        }
    };
    let step = (waited / FRAME_STATS_SECONDS) as u32;
    // A triangle wave over the reach.
    let span = PAN_ASIDE_REPORTS / 4.0;
    let phase = step as f32 % PAN_ASIDE_REPORTS;
    let units = match phase {
        phase if phase <= span => phase,
        phase if phase <= 3.0 * span => 2.0 * span - phase,
        phase => phase - PAN_ASIDE_REPORTS,
    };
    let aside = radius * PAN_ASIDE_REACH * units / span;
    // Once a step: a browser console line per frame slows the frames being timed.
    static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if SAID.swap(step + 1, Ordering::Relaxed) != step + 1 {
        log::info!("{aside:.0} units aside of {radius:.0}");
    }
    let sideways = persistent.camera.right_direction().normalize() * aside;
    let up = persistent.camera.up();
    persistent
        .camera
        .set_view(eye + sideways, at + sideways, up);
    true
}
