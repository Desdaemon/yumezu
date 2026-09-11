//! What a frame costs, and the switches that make one comparable with another.
//!
//! `profile` builds only; `super` stands in for it everywhere else. The page gets everything here
//! but the GPU's own clock, which needs a timer query a browser only hands out where
//! `EXT_disjoint_timer_query_webgl2` is advertised -- see [`readable`].
//!
//! Numbers are logged, never drawn: a counter the frame paints is a counter the frame pays for.

use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use three_d::renderer::*;

use super::AppEntities;
use super::camera::AppStatics;

/// How often [`FrameStats`] says what it has counted.
const FRAME_STATS_SECONDS: f64 = 2.0;

/// Timing the passes appends one column each.
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
/// One wider than the longest name in [`COLUMNS`], or two columns run together.
const COLUMN: usize = 11;
const HEADER_SECONDS: f64 = 30.0;

fn row<'a>(cells: impl IntoIterator<Item = &'a str>) -> String {
    cells
        .into_iter()
        .map(|it| format!("{it:>COLUMN$}"))
        .collect()
}

/// Where the switches below start, for a run with nobody to click them: `YUMEZU_FRAMES` names
/// them, comma separated, and `?frames=` does on a page -- which has no environment to read and an
/// address instead. A starting position and nothing more -- the checkbox still moves them.
fn asked(switch: &str) -> bool {
    #[cfg(not(target_family = "wasm"))]
    let frames = std::env::var("YUMEZU_FRAMES").ok();
    #[cfg(target_family = "wasm")]
    let frames = super::link::frames();
    frames.is_some_and(|frames| frames.split(',').any(|it| it.trim() == switch))
}

/// Where a run being measured points the camera, as `eye` then `target`: six numbers, comma
/// separated, named by `YUMEZU_CAMERA` or by `?camera=` on a page.
///
/// The same six the `camera in ... dimensions` log line prints, so a pose that read badly in one
/// run is handed straight to the next. Unasked, [`pan_aside`] frames the whole graph instead --
/// which is a view of everything at once, and not the view a close pose is slow in.
fn asked_pose() -> Option<(Vec3, Vec3)> {
    #[cfg(not(target_family = "wasm"))]
    let asked = std::env::var("YUMEZU_CAMERA").ok()?;
    #[cfg(target_family = "wasm")]
    let asked = super::link::camera()?;
    let numbers: Vec<f32> = asked
        .split(',')
        .filter_map(|it| it.trim().parse().ok())
        .collect();
    let [ex, ey, ez, ax, ay, az] = numbers[..] else {
        log::warn!("{asked:?} is not six numbers, so the camera is framed on the graph instead");
        return None;
    };
    Some((vec3(ex, ey, ez), vec3(ax, ay, az)))
}

/// The GPU's timer will not nest, so the whole-frame clock stands down while this is on.
static PARTS: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(asked("parts")));
/// Draws the way every frame was drawn before the demand was worked out, so that the two can be
/// measured against each other in one binary: it redraws at the display's rate whatever is on
/// screen, rebuilds the whole frame's geometry whether or not anything it is built from has
/// changed, and lays the dash runs out again every frame. What it does not put back is the pair of
/// matrix products a dash used to cost -- see [`super::DashRun`] -- so it reads as a floor on the
/// old cost rather than the old cost itself.
static EAGER: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(asked("eager")));
/// See [`pan_aside`].
static DOLLY: LazyLock<AtomicBool> = LazyLock::new(|| AtomicBool::new(asked("dolly")));

/// Vsync is settled when the surface is made, so unlike the switches above this has to outlive
/// the run that asked for it. Native's alone -- see [`unlocked`].
#[cfg(not(target_family = "wasm"))]
const UNLOCKED: &str = "frames-unlocked";

pub(super) fn eager() -> bool {
    EAGER.load(Ordering::Relaxed)
}

/// Latched at the first ask, which is when the surface is made.
///
/// Always off on a page: the display's rate is the browser's to pace -- a frame comes when
/// `requestAnimationFrame` says it does -- and nothing this asks for changes that.
pub(super) fn unlocked() -> bool {
    #[cfg(target_family = "wasm")]
    return false;
    #[cfg(not(target_family = "wasm"))]
    {
        static RUNNING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *RUNNING.get_or_init(|| super::store::read(UNLOCKED).as_deref() == Some("on"))
    }
}

/// Labels are untranslated: a `profile` build never reaches anybody who would read them.
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
    // Nothing to offer on a page: the browser paces the frames and keeps the pacing.
    #[cfg(not(target_family = "wasm"))]
    {
        let mut asked = super::store::read(UNLOCKED).as_deref() == Some("on");
        if ui
            .checkbox(&mut asked, "run past the display")
            .on_hover_text("Turns vsync off, which is the only way to watch the GPU's cost move.")
            .changed()
        {
            super::store::write(UNLOCKED, Some(if asked { "on" } else { "off" }));
        }
        if asked != unlocked() {
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

/// How long [`pan_aside`] waits for the layout to stop growing before it frames anything.
const PAN_ASIDE_SETTLES_SECONDS: f64 = 8.0;
/// How far the pan reaches to either side of the framed middle, as a fraction of the radius it
/// framed.
///
/// Short of the whole radius: the worlds pile up near the middle, and a window drawn out at the
/// rim is a window of empty space. Bounded at all because the reach used to be unbounded -- a step
/// a report, in one direction, forever -- so a run left going long enough walked off the graph and
/// spent its last windows timing the background.
const PAN_ASIDE_REACH: f32 = 0.5;
/// Reports in one there-and-back-again. The reach is divided across these rather than stepped by a
/// fixed distance, so the path takes the same time on any graph, and two runs of the same length
/// draw the same views in the same order.
const PAN_ASIDE_REPORTS: f32 = 16.0;

/// How many frames of queries are kept, and so how many frames late an answer may be without
/// being thrown away. The GPU is a few frames behind the processor and asking it for an answer it
/// has not reached would stall the very thing being measured.
const GPU_CLOCK_DEPTH: usize = 4;

/// The two answers a timer query has, looked up out of the GL the window already opened.
///
/// glow reaches for `glGetQueryObjectuiv` only when GL 4.5's `glGetQueryBufferObjectiv` loaded
/// and for the `EXT` spelling otherwise, so on a 4.1 context -- every one macOS gives out --
/// it calls a pointer it never loaded and panics. Both symbols are in the process all the same,
/// where `dlsym` finds them. See `glow`'s `native::get_query_parameter_u32`.
#[cfg(unix)]
mod answers {
    use std::ffi::{c_char, c_void};
    use std::sync::LazyLock;

    type Uiv = unsafe extern "C" fn(u32, u32, *mut u32);
    type Ui64v = unsafe extern "C" fn(u32, u32, *mut u64);

    // SAFETY: the declaration is `dlsym`'s own, from `<dlfcn.h>`.
    #[allow(unsafe_code)]
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// `RTLD_DEFAULT`, which is every symbol the process has loaded, in the order it loaded them.
    /// Apple spells it `-2` and everybody else the null handle.
    #[cfg(target_vendor = "apple")]
    const LOADED: *mut c_void = -2isize as *mut c_void;
    #[cfg(not(target_vendor = "apple"))]
    const LOADED: *mut c_void = std::ptr::null_mut();

    /// `name` must be nul terminated, and `F` must be the symbol's own signature.
    #[allow(unsafe_code)]
    unsafe fn find<F: Copy>(name: &str) -> Option<F> {
        const { assert!(size_of::<F>() == size_of::<*mut c_void>()) };
        // SAFETY: the handle is the documented constant, the name is nul terminated, and a
        // symbol that is not there comes back null rather than as a fault.
        let found = unsafe { dlsym(LOADED, name.as_ptr().cast()) };
        // SAFETY: the caller named the signature, and a function pointer is pointer sized.
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

/// Whether an answer can be read at all, which is the one thing [`GpuClock`] cannot recover from
/// finding out late: glow panics rather than answers, and a page answers zero rather than either.
fn readable(context: &Context) -> bool {
    #[cfg(unix)]
    {
        let _ = context;
        answers::UIV.is_some() && answers::UI64V.is_some()
    }
    /// `TIME_ELAPSED` is this extension's own enum rather than WebGL2's. A context without it
    /// takes `begin_query` without complaint and then reads every answer back as zero, which is a
    /// column of `0.00` where the honest answer is a dash -- so the queries are not made at all.
    /// Chromium advertises it on a desktop GPU; Firefox and Safari do not advertise it anywhere.
    #[cfg(all(not(unix), target_family = "wasm"))]
    const TIMER: &str = "EXT_disjoint_timer_query_webgl2";
    #[cfg(all(not(unix), target_family = "wasm"))]
    {
        context.supported_extensions().contains(TIMER)
    }
    #[cfg(all(not(unix), not(target_family = "wasm")))]
    {
        let _ = context;
        true
    }
}

/// # Safety
/// `query` must be the context's own, and `parameter` one the query answers.
#[allow(unsafe_code)]
unsafe fn parameter_u32(context: &Context, query: three_d::context::Query, parameter: u32) -> u32 {
    #[cfg(unix)]
    {
        let _ = context;
        let mut answer = 0;
        if let Some(uiv) = *answers::UIV {
            // SAFETY: the caller vouched for both arguments, and the answer is one `u32`.
            unsafe { uiv(query.0.get(), parameter, &raw mut answer) };
        }
        answer
    }
    #[cfg(not(unix))]
    // SAFETY: the caller vouched for both arguments.
    unsafe {
        context.get_query_parameter_u32(query, parameter)
    }
}

/// # Safety
/// As [`parameter_u32`].
#[allow(unsafe_code)]
unsafe fn parameter_u64(context: &Context, query: three_d::context::Query, parameter: u32) -> u64 {
    #[cfg(unix)]
    {
        let _ = context;
        let mut answer = 0;
        if let Some(ui64v) = *answers::UI64V {
            // SAFETY: the caller vouched for both arguments, and the answer is one `u64`.
            unsafe { ui64v(query.0.get(), parameter, &raw mut answer) };
        }
        answer
    }
    #[cfg(not(unix))]
    // SAFETY: the caller vouched for both arguments.
    unsafe {
        context.get_query_parameter_u64(query, parameter)
    }
}

/// Seconds the GPU spent on a frame, which the processor's own milliseconds never say: a frame
/// here spends under one of them and then waits in the driver, so this is the number that tells a
/// view that is slow from one that is merely paced.
struct GpuClock {
    context: Context,
    queries: Vec<three_d::context::Query>,
    /// Frames begun, which picks both this frame's query and the one old enough to read.
    begun: usize,
}

impl GpuClock {
    /// An empty set of queries is a driver that would not give them out, which costs the report a
    /// column and nothing else.
    fn new(context: &Context) -> Self {
        // SAFETY: names no object and reads no memory. A refusal comes back as an error.
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

    /// Whether there are queries to keep, which there are unless the driver refused them.
    fn ready(&self) -> bool {
        self.queries.len() == GPU_CLOCK_DEPTH
    }

    fn begin(&self) {
        if !self.ready() {
            return;
        }
        // SAFETY: the query is this context's own and no other is open -- `end` closes each one
        // before `begin` opens the next, and nothing else in the app opens one at all.
        #[allow(unsafe_code)]
        unsafe {
            self.context.begin_query(
                three_d::context::TIME_ELAPSED,
                self.queries[self.begun % GPU_CLOCK_DEPTH],
            );
        }
    }

    /// Ends this frame's query and answers with the frame from [`GPU_CLOCK_DEPTH`] ago, if the
    /// GPU has finished it. `None` is a frame's measurement missed rather than an error.
    fn end(&mut self) -> Option<f64> {
        use three_d::context::{QUERY_RESULT, QUERY_RESULT_AVAILABLE, TIME_ELAPSED};
        if !self.ready() {
            return None;
        }
        // SAFETY: `begin` opened this context's query, and every object named below is its own.
        #[allow(unsafe_code)]
        unsafe {
            self.context.end_query(TIME_ELAPSED)
        };
        self.begun += 1;
        // Nothing has gone all the way round yet, so the slot below holds a query never begun and
        // an answer that would read as an instant frame.
        if self.begun < GPU_CLOCK_DEPTH {
            return None;
        }
        // The slot about to be written next is the oldest one written, which is the frame
        // `GPU_CLOCK_DEPTH` back. Asked whether it is ready rather than for the answer: the answer
        // would stall the processor on the very GPU it is timing.
        let oldest = self.queries[self.begun % GPU_CLOCK_DEPTH];
        #[allow(unsafe_code)]
        unsafe {
            (parameter_u32(&self.context, oldest, QUERY_RESULT_AVAILABLE) != 0)
                .then(|| parameter_u64(&self.context, oldest, QUERY_RESULT) as f64 * 1e-9)
        }
    }
}

/// One pass of a frame, timed on its own. See [`FrameStats::timed`].
struct Section {
    name: &'static str,
    clock: GpuClock,
    /// Seconds this pass took over the [`Section::answered`] frames the GPU answered for. Reset
    /// by the report rather than by [`Tally`], which would take the queries with it.
    spent: f64,
    answered: u32,
}

/// What a reporting window has added up so far.
#[derive(Default)]
struct Tally {
    frames: u32,
    /// Seconds of processor time inside [`super::App::draw`], which is the frame without the wait for
    /// the display that follows it.
    spent: f64,
    /// Frames that got as far as rebuilding the layout's geometry, of the [`Tally::frames`] that
    /// were drawn at all.
    rebuilt: u32,
    /// Seconds the GPU spent, over the [`Tally::answered`] frames it answered for. See
    /// [`GpuClock`].
    gpu: f64,
    answered: u32,
    /// Seconds inside the layout's own step -- the force simulation, not the panel's. Part of
    /// [`Tally::spent`].
    layout: f64,
    /// Draw calls over [`Tally::frames`]: what the scene costs to submit rather than to fill.
    calls: u32,
}

/// What one frame cost and how many of them there were.
pub(super) struct FrameStats {
    since: web_time::Instant,
    tally: Tally,
    /// Every frame of the window: a percentile cannot be worked out from a sum.
    each: Vec<f64>,
    /// Built on the first frame, the context being younger than this, and `None` for as long
    /// as the driver will not give out the queries. See [`GpuClock`].
    clock: Option<GpuClock>,
    /// In the order the frame draws them, which is the order they are first timed in.
    sections: Vec<Section>,
    /// [`PARTS`] as this frame opened. Latched, because the box that sets it is ticked from
    /// inside the frame: flipping halfway would open a pass timer inside the whole-frame one.
    parts: bool,
    headed: Vec<&'static str>,
    /// `None` until the first header is written, which is what puts one above the first report.
    /// Not an instant backdated by [`HEADER_SECONDS`]: a page's clock starts at the document
    /// rather than at boot, so at the first frame there is not yet that much of it to take away.
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

    /// `body` answers with the draw calls it made.
    ///
    /// Every pass of the frame goes through here whether or not anything is being measured, so
    /// that what is measured is the frame that ships rather than one drawn differently to be
    /// looked at.
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
                // A dash where the GPU was not asked: queries refused, or the passes have the
                // clock.
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

/// Brings the whole graph into view and then pans across it, a reporting window a step. See
/// [`DOLLY`].
///
/// Framed rather than left on the pose the run opened on, which is one world filling the window:
/// a view with nothing in it times the background, and what is worth timing is a frame with the
/// graph's own load in it. Framed once, so the distance and the orientation are the same in every
/// window and two of them still differ only by what they were drawing.
///
/// A pan rather than a dolly for that same reason -- it changes what is in the frame and nothing
/// else -- and a bounded one: out to one side, back through the middle, out to the other and back,
/// so a long run keeps drawing the graph instead of walking off it.
///
/// Answers whether it is the one driving the view, which the caller owes a frame: this runs inside
/// the frame, so a run that stopped asking for frames would never reach the step that would have
/// asked for the next one -- the switch would hold the opening pose forever and report nothing.
/// Every window is drawn at the display's rate for the same reason, two of them being comparable
/// only if neither was cut short.
pub(super) fn pan_aside(statics: &mut AppStatics, data: &AppEntities) -> bool {
    if !DOLLY.load(Ordering::Relaxed) {
        return false;
    }
    // Held off while the layout is still blowing out from its seed, which is not a view anybody
    // looks at and not one worth timing -- nor one worth framing, the sphere still growing.
    static SINCE: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    let waited = SINCE
        .get_or_init(web_time::Instant::now)
        .elapsed()
        .as_secs_f64()
        - PAN_ASIDE_SETTLES_SECONDS;
    if waited < 0.0 {
        return true;
    }
    // Set on the first frame past the settle, and only once there is a graph to frame: a dump that
    // has not landed yet has no sphere, and latching that would leave the run pointed at nothing.
    static FRAMED: std::sync::OnceLock<(Vec3, Vec3, f32)> = std::sync::OnceLock::new();
    let (eye, at, radius) = match FRAMED.get() {
        Some(framed) => *framed,
        None => {
            let framed = match asked_pose() {
                // A pose named by hand is the whole instruction. The reach is taken from how far
                // the camera stands off its target, there being no sphere to take it from -- so
                // the pan stays in proportion to the view whether it is a close one or not.
                Some((eye, at)) => (eye, at, (eye - at).magnitude()),
                None => {
                    let Some(bounds) = data.whole_bounds() else {
                        return true;
                    };
                    statics.snap_to_frame(&bounds);
                    (
                        statics.camera.position(),
                        statics.camera.target(),
                        bounds.radius,
                    )
                }
            };
            let _ = FRAMED.set(framed);
            framed
        }
    };
    let step = (waited / FRAME_STATS_SECONDS) as u32;
    // A triangle over the reach: 0 out to one side over a quarter of the reports, back through the
    // middle to the other side over a half, and home over the last quarter.
    let span = PAN_ASIDE_REPORTS / 4.0;
    let phase = step as f32 % PAN_ASIDE_REPORTS;
    let units = match phase {
        phase if phase <= span => phase,
        phase if phase <= 3.0 * span => 2.0 * span - phase,
        phase => phase - PAN_ASIDE_REPORTS,
    };
    let aside = radius * PAN_ASIDE_REACH * units / span;
    // Once a step rather than once a frame. Said every frame it was a line per frame in the
    // console, which a browser charges for -- the switch would have been paying for its own
    // report, and every window under it read slower than the frame it was timing.
    static SAID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if SAID.swap(step + 1, Ordering::Relaxed) != step + 1 {
        log::info!("{aside:.0} units aside of {radius:.0}");
    }
    let sideways = statics.camera.right_direction().normalize() * aside;
    let up = statics.camera.up();
    statics.camera.set_view(eye + sideways, at + sideways, up);
    true
}
