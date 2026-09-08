//! What a frame costs, and the switches that make one comparable with another.
//!
//! Native `profile` builds only; `super` stands in for it everywhere else. Native because the page
//! has no timer query -- browsers do not hand one out.
//!
//! Numbers are logged, never drawn: a counter the frame paints is a counter the frame pays for.

use std::sync::atomic::{AtomicBool, Ordering};

use three_d::renderer::*;

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

/// The GPU's timer will not nest, so the whole-frame clock stands down while this is on.
static PARTS: AtomicBool = AtomicBool::new(false);
/// Draws the way every frame was drawn before the demand was worked out, so that the two can be
/// measured against each other in one binary: it redraws at the display's rate whatever is on
/// screen, rebuilds the whole frame's geometry whether or not anything it is built from has
/// changed, and lays the dash runs out again every frame. What it does not put back is the pair of
/// matrix products a dash used to cost -- see [`super::DashRun`] -- so it reads as a floor on the
/// old cost rather than the old cost itself.
static EAGER: AtomicBool = AtomicBool::new(false);
/// See [`pan_aside`].
static DOLLY: AtomicBool = AtomicBool::new(false);

/// Vsync is settled when the surface is made, so unlike the switches above this has to outlive
/// the run that asked for it.
const UNLOCKED: &str = "frames-unlocked";

pub(super) fn eager() -> bool {
    EAGER.load(Ordering::Relaxed)
}

/// Latched at the first ask, which is when the surface is made.
pub(super) fn unlocked() -> bool {
    static RUNNING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNING.get_or_init(|| super::store::read(UNLOCKED).as_deref() == Some("on"))
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
        "Holds the pose the run opened on and steps the camera sideways once a report, so two \
         windows differ by what they were drawing and nothing else.",
    );
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

fn switch(ui: &mut egui::Ui, flag: &AtomicBool, label: &str, hint: &str) {
    let mut on = flag.load(Ordering::Relaxed);
    if ui.checkbox(&mut on, label).on_hover_text(hint).changed() {
        flag.store(on, Ordering::Relaxed);
    }
}

/// How far each step of [`pan_aside`] moves, and how long it waits for the layout to
/// stop growing first.
const PAN_ASIDE_STEP: f32 = 40.0;
const PAN_ASIDE_SETTLES_SECONDS: f64 = 8.0;

/// How many frames of queries are kept, and so how many frames late an answer may be without
/// being thrown away. The GPU is a few frames behind the processor and asking it for an answer it
/// has not reached would stall the very thing being measured.
const GPU_CLOCK_DEPTH: usize = 4;

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
        let queries = (0..GPU_CLOCK_DEPTH)
            .map_while(|_| unsafe { context.create_query() }.ok())
            .collect();
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
            (self
                .context
                .get_query_parameter_u32(oldest, QUERY_RESULT_AVAILABLE)
                != 0)
                .then(|| self.context.get_query_parameter_u64(oldest, QUERY_RESULT) as f64 * 1e-9)
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
    headed_at: web_time::Instant,
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
            // So the first report writes a header above itself.
            headed_at: web_time::Instant::now()
                - std::time::Duration::from_secs_f64(HEADER_SECONDS),
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
        if self.headed_at.elapsed().as_secs_f64() >= HEADER_SECONDS || self.headed != passes {
            log::info!("{}", row(cells.iter().map(|(name, _)| *name)));
            (self.headed, self.headed_at) = (passes, web_time::Instant::now());
        }
        log::info!("{}", row(cells.iter().map(|(_, it)| it.as_str())));
    }
}

/// Holds the pose a run opens on and then pans away from it, a reporting window a step. See
/// [`DOLLY`].
///
/// A pan rather than a dolly because it changes what is in the frame and nothing else -- the
/// distance and the orientation stay, so two reports differ only by what they were drawing.
pub(super) fn pan_aside(camera: &mut Camera) {
    if !DOLLY.load(Ordering::Relaxed) {
        return;
    }
    static SINCE: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    static OPENED: std::sync::OnceLock<(Vec3, Vec3)> = std::sync::OnceLock::new();
    let (eye, at) = *OPENED.get_or_init(|| (camera.position(), camera.target()));
    // Held off while the layout is still blowing out from its seed, which is not a view
    // anybody looks at and not one worth timing.
    let waited = SINCE
        .get_or_init(web_time::Instant::now)
        .elapsed()
        .as_secs_f64()
        - PAN_ASIDE_SETTLES_SECONDS;
    let step = (waited / FRAME_STATS_SECONDS).max(0.0) as u32;
    let aside = camera.right_direction().normalize() * PAN_ASIDE_STEP * step as f32;
    if step > 0 {
        log::info!("{:.0} units aside", PAN_ASIDE_STEP * step as f32);
    }
    let up = camera.up();
    camera.set_view(eye + aside, at + aside, up);
}
