//! See [App::draw] for the main flow.

use egui::special_emojis::GITHUB;
use egui_material_icons::icons::*;
use force_graph_3d::{
    DefaultNodeIdx, Dimensions, EdgeData, ForceGraph, NodeData, SimulationParameters,
};
use three_d::{FrameInput, FrameInputGenerator, SurfaceSettings, WindowedContext, renderer::*};
use winit::{
    application::ApplicationHandler,
    event::{Touch, TouchPhase, WindowEvent},
    window::{CursorIcon, Window},
};

use world::{Ask, Gate};

/// How long after the last thing that moved the window goes on being drawn at the display's rate.
///
/// Without it the pacing would follow the input rather than the view: a drag reaches the app as
/// events that arrive unevenly and stop the moment a hand pauses, so a frame that saw no event
/// would drop straight to [`IDLE_REDRAW_HZ`] and the next one back to the display's rate. What
/// settles into the slow rate is a view that has genuinely stopped -- the layout at rest and the
/// camera untouched this long.
const IDLE_AFTER_SECONDS: f32 = 2.0;
/// Frames a second the window falls back to when the dashes are the only thing still moving.
///
/// A settled layout nobody is touching would otherwise be redrawn at the display's own rate for
/// as long as the app is open, which on a phone is most of what it costs. The marching is by wall
/// clock rather than by frame, so a slower rate makes it coarser and not slower. See
/// [`App::wanted`].
const IDLE_REDRAW_HZ: f32 = 30.0;
/// Frames a second while the only reason to draw one is to see whether something asked for over
/// the network has landed. Nothing on screen is moving, so this is a poll and not an animation.
const POLL_REDRAW_HZ: f32 = 10.0;
/// How long the app waits before asking for the thumbnail atlas again, after a try that could not
/// be had. Longer than the dump's retries because nothing is waiting on it: the graph draws while
/// the atlas is missing, only without the pictures on its nodes. See [`Atlas`].
const ATLAS_RETRIED_AFTER_SECONDS: f32 = 10.0;
/// How long the loading frame takes to fade off the graph behind it. See [`App::veil`].
const LOADING_FADE_SECONDS: f32 = 0.5;

/// How often the server is asked what it is building, while there is nothing to draw. A stage
/// lasts tens of seconds, so this is about how soon a change is said rather than about following
/// anything closely.
const BUILDING_ASKED_EVERY_SECONDS: f32 = 1.0;
/// How long a run waits before asking for the dump again, after the server said it is building one.
///
/// Longer than the ask above because this one is not for the screen: nothing about the wait looks
/// different for having asked, and the server is being polled about its progress anyway.
const DUMP_ASKED_EVERY_SECONDS: f32 = 3.0;
/// How long a run waits before asking again, after the dump could not be had at all.
///
/// Much longer than the ask above, which waits on a server that answered and will have a dump
/// within the minute, where this is a host that is not there. Still short enough that a server
/// coming back, or a laptop finding the network again, is picked up while the person is still
/// looking at the window.
const DUMP_RETRIED_AFTER_SECONDS: f32 = 10.0;
mod arrivals;
mod camera;
mod detail;
mod download;
mod fetch;
mod graph;
mod gui;
mod guide;
// Re-exported from the crate root so `t!` resolves the same in both of this crate's targets: see
// `main.rs`.
pub(crate) mod i18n;
mod japanese;
mod layout;
mod link;
mod map;
#[cfg(all(feature = "profile", not(target_family = "wasm")))]
mod profile;

/// What [`profile`] is everywhere it is not compiled: the same surface, all of it nothing, so no
/// other file has to know which build this is and the frame that ships has no counters, no timer
/// queries and no branches on either.
#[cfg(not(all(feature = "profile", not(target_family = "wasm"))))]
mod profile {
    use three_d::{Camera, Context};

    pub(super) fn eager() -> bool {
        false
    }

    pub(super) fn unlocked() -> bool {
        false
    }

    pub(super) fn pan_aside(_camera: &mut Camera) {}

    pub(super) fn controls(_ui: &mut egui::Ui) {}

    pub(super) struct FrameStats;

    impl FrameStats {
        pub(super) fn new() -> Self {
            Self
        }

        pub(super) fn timed(
            &mut self,
            _name: &'static str,
            _context: &Context,
            body: impl FnOnce() -> u32,
        ) {
            body();
        }

        pub(super) fn stepped(&mut self, _spent: std::time::Duration) {}

        pub(super) fn began(&mut self, _context: &Context) {}

        pub(super) fn frame(&mut self, _spent: std::time::Duration, _rebuilt: bool) {}
    }
}
mod scene;
mod store;
#[cfg(target_family = "wasm")]
mod text_agent;
mod thumbnails;
mod ui;
#[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
mod update;

/// What [`update`] is on the two targets that do not install packages: the page reloads and the
/// apk goes through the store, so neither has anything to replace.
#[cfg(any(target_family = "wasm", target_os = "android"))]
mod update {
    pub(super) fn controls(_ui: &mut egui::Ui) {}
}
mod world;
mod yno;

use arrivals::*;
use camera::*;
use graph::*;
use i18n::t;
use layout::*;
use profile::FrameStats;
use scene::*;
use ui::*;

use crate::i18n::speaking_japanese;

/// The handle the activity glue passed to `android_main`, which is handed out exactly once and is
/// the only way to reach anything the framework owns. Kept because two things want it well after
/// startup: the assets in [`thumbnails`], and the safe area and soft keyboard in [`Overlay`].
#[cfg(target_os = "android")]
static ANDROID: std::sync::OnceLock<winit::platform::android::activity::AndroidApp> =
    std::sync::OnceLock::new();

/// Everything that wants the handle runs too late to be given it directly.
#[cfg(target_os = "android")]
pub(crate) fn use_android_app(app: winit::platform::android::activity::AndroidApp) {
    let _ = ANDROID.set(app);
}

/// Samples the surface is asked for when the edges are being smoothed, and the frame's largest
/// cost by a distance: multisampling rasterizes every covered fragment this many times over, and
/// at the size the graph is drawn that was the whole difference between keeping up with the
/// display and not -- measured, by turning it off.
///
/// Never shown, and there is no setting for it, because it is not a number everywhere: the page
/// hears only "on" or "off", WebGL taking a boolean and the browser picking the count. A count
/// the person could lower would lower nothing there, and the one screen where lowering it is free
/// is the one where the cost was never a problem.
const MULTISAMPLES: u8 = 4;

/// Whether the surface smooths the edges, which is settled as it is built. Multisampling until
/// someone says otherwise -- it is the only thing that reads the geometry, and the graph is mostly
/// thin high-contrast cylinders.
///
/// The one thing a pass over the finished frame would have to offer is being cheaper, and it is
/// only cheaper where the fill is the limit: on a desktop it costs the same and looks worse, and
/// on the page it turned out to cost more than the surface's own. So there is nothing between
/// these two.
fn antialias_remembered() -> bool {
    store::read(ANTIALIAS).as_deref() != Some("off")
}

fn antialias_remember(on: bool) {
    store::write(ANTIALIAS, Some(if on { "on" } else { "off" }));
}

/// Whether the view leans onto a world a list row is pointing at, which the panel's switch sets.
/// See [`AppStatics::lean_toward`].
///
/// What it opens on where the switch has never been touched is what the platform already says
/// about motion: asking the person again for something they have said once is how a preference
/// gets ignored.
fn leaning_remembered() -> bool {
    match store::read(LEANING).as_deref() {
        Some("off") => false,
        Some(_) => true,
        None => !reduced_motion(),
    }
}

fn leaning_remember(on: bool) {
    store::write(LEANING, Some(if on { "on" } else { "off" }));
}

/// Whether the person has asked for less movement than an app would otherwise make.
///
/// The page has somewhere to read this from and nothing else here does: winit carries no such
/// preference, and the platform APIs behind it are one per platform. So elsewhere the switch is
/// the only answer, and it opens on.
fn reduced_motion() -> bool {
    #[cfg(target_family = "wasm")]
    {
        // A browser that will not answer is one that was never asked to hold back.
        web_sys::window()
            .and_then(|window| {
                window
                    .match_media("(prefers-reduced-motion: reduce)")
                    .ok()
                    .flatten()
            })
            .is_some_and(|query| query.matches())
    }
    #[cfg(not(target_family = "wasm"))]
    {
        false
    }
}

/// When the next frame is wanted, which is what a frame ends by working out.
///
/// The window is redrawn on demand rather than continuously: see [`App::wanted`] for what is
/// asking, and [`profile::eager`] for the switch that puts the old always-on behaviour back.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Wanted {
    /// As soon as the display will take one: something is moving.
    Now,
    /// Not before this long has passed. Nothing is moving fast enough to need the display's rate.
    After(std::time::Duration),
    /// Not until an event arrives. Every pixel would come out the way it already is.
    Never,
}

impl Wanted {
    /// The stricter of two demands, a frame being drawn for whichever of them asks first.
    fn or_sooner(self, other: Self) -> Self {
        self.min(other)
    }

    fn after_hz(hz: f32) -> Self {
        Self::After(std::time::Duration::from_secs_f32(1.0 / hz))
    }

    /// Zero reads as [`Wanted::Now`] and [`std::time::Duration::MAX`] as [`Wanted::Never`], which
    /// is how egui spells both.
    fn after(delay: std::time::Duration) -> Self {
        if delay.is_zero() {
            Self::Now
        } else if delay == std::time::Duration::MAX {
            Self::Never
        } else {
            Self::After(delay)
        }
    }
}

pub(super) struct App {
    ctx: AppContext,
    data: Option<AppEntities>,
    statics: AppStatics,
    /// Built with the graphics context, so it cannot exist before the window does.
    overlay: Option<Overlay>,
    /// What everything in [`AppEntities`] is built out of. See [`Dump`].
    dump: Dump,
    /// Whose game is being drawn: the whole of it, or as much as one player has seen. Kept beside
    /// the dump because it is the other thing the entities are built out of. See [`App::build`].
    yno: yno::Account,
    /// What the graph being replaced was, held between the frame that throws it away and the one
    /// that builds the next out of it. `None` at every other moment. See [`Before`].
    before: Option<Before>,
    building: Building,
    /// What was lit when the drawing surface was last given back, to light again once there is a
    /// graph to light it in.
    ///
    /// Kept out here for the same reason the dump is: on a phone the entities holding it are
    /// dropped every time the app leaves the screen -- see [`App::release`] -- and a selection is
    /// the reader's rather than the surface's. The layout underneath is built afresh, so the
    /// camera is sent to the selection again rather than left where it was.
    selected: Option<Highlight>,
    /// The world the link asked to open on, until there is a graph to find it in. Taken by the
    /// first [`App::build`], which turns it into a selection like any other.
    asked_for: Option<String>,
    /// The line the loading frame was last showing, which the fade carries out over the graph: a
    /// line that changed on the way out would read as a new thing to look at, and what is waited
    /// for is not the same thing throughout. See [`App::veil`].
    said: String,
    /// How much of the loading frame is still on screen, 1 for all of it and 0 for none.
    ///
    /// The frame is not switched off when the dump lands but faded off over the graph that
    /// replaced it, so the graph is uncovered rather than dropped on screen. Written back to 1 by
    /// every [`App::draw_loading`], so a run that has to wait again gets the reveal again.
    veil: f32,
    /// When the frame just drawn wants the next one. Read by the event loop, which is where the
    /// asking is turned into a wait. See [`Wanted`].
    wanted: Wanted,
    /// Whether a frame has been asked for and not yet drawn. See [`App::ask_for_a_frame`].
    asked: bool,
    /// Seconds since anything on screen last moved. See [`IDLE_AFTER_SECONDS`].
    still: f32,
    stats: FrameStats,
}

/// Where the thumbnail atlas has got to, which is a state rather than a handle because a try that
/// fails is retried: it is fetched over the network like everything else, and one flaky moment
/// should not leave a run without pictures. Modelled on [`Dump`], which waits the same way.
enum Atlas {
    Loading(fetch::Pending<Option<CpuTexture>>),
    /// Seconds until the next ask.
    Waiting(f32),
    /// On screen, or wrong-shaped and therefore never going to be: a mismatch between the atlas and
    /// the dump is not something asking again can mend.
    Settled,
}

struct AppEntities {
    graph: Graph,
    /// Held here rather than beside the camera, so rebuilding the graph drops the gesture along
    /// with the node index it names.
    gesture: Option<Gesture>,
    /// Kept so a restart can carry on drawing from the same sequence.
    rng: Rng,
    routes: world::Routes,
    /// Indexed like the nodes. Only the overlay reads them. See [`world::Title`].
    titles: Vec<world::Title>,
    /// Per world, in the order the wiki lists them and empty for the worlds it draws none of. Only
    /// the overlay reads them.
    maps: Vec<Vec<world::Map>>,
    /// Busiest first, which is also what [`Highlight::Author`] indexes. See
    /// [`world::Dump::authors`].
    authors: Vec<world::Author>,
    /// Per world, which of those authors made it.
    author_of: Vec<usize>,
    /// Newest first, each carrying what it added. What [`Highlight::Version`] indexes, and what
    /// the catalog lists.
    versions: Vec<world::Version>,
    /// What is lit. Everything off the highlight is dimmed.
    selected: Option<Highlight>,
    /// The depth of the furthest world the origin can reach, which is the last layer the rocker
    /// can step to.
    deepest: u32,
    /// Where the right button went down, while it is still down. The button both pans and opens
    /// the menu, and only the travel between press and release tells those apart.
    right_press: Option<PhysicalPoint>,
    /// Drawn by the overlay, which also closes it once one of its entries is taken.
    menu: Option<ContextMenu>,
    /// Where the pointer last was with nothing pressed, in physical pixels, and what the hover
    /// test found there. `None` for a pointer that has left the window or moved onto the panel,
    /// and on a touch screen throughout: a finger never moves without pressing, so it never
    /// hovers.
    cursor: Option<PhysicalPoint>,
    hover: Option<usize>,
    /// The world a sidebar list row is pointing at, brightened where it sits in the graph and
    /// cleared as soon as the pointer leaves the row. Only ever one: see `Panel::pointed`.
    pointed: Option<usize>,
    /// The world the view is leaning onto, which outlives the pointing that asked for it. `None`
    /// once the person has taken the camera back. See [`AppStatics::lean_toward`].
    leaning_at: Option<usize>,
    /// Per world, every connection it has and which ways round it can be walked. The lines are
    /// built from it, and the panel reads a world's ways on out of it.
    connections: Vec<Vec<world::Step>>,
    /// Per world, how many other worlds it connects to: the degree of the graph as drawn, which is
    /// what tells a junction from a dead end. Only the overlay reads it.
    degrees: Vec<usize>,
    /// At most `UNTAKEN_WORLDS` worlds, the ones with the most ways out nobody has taken first.
    /// Empty in a run drawing the whole game, which is what settles whether the panel offers the
    /// list at all. See [`Highlight::Untaken`].
    untaken: Vec<usize>,
    /// The world a run opens framed on, with everything behind it: `None` in a dump that does not
    /// hold it, which opens on the camera's own pose instead. See [`opening_room`].
    opening: Option<usize>,
    /// Cleared once the camera arrives at the selected route, or as soon as the person takes the
    /// camera back.
    framing: bool,
    /// Whether a framed route is framed whole rather than on the world at its end. Off by default,
    /// the route being read out in the panel and the camera wanted for the world the reader
    /// picked. Held across selections, being a way of looking rather than a fact about one route.
    frame_route: bool,
    /// Connection colors before a selection dims them, so clearing one restores the distance ramp.
    edge_colors: Vec<Srgba>,
    /// Per node, the half-height of its quad, from how much of the graph hangs off it.
    node_radii: Vec<f32>,
    arrivals: Arrivals,
    /// The panel's setting for how much of a world's size goes into how hard it pushes. See
    /// [`node_mass`].
    hub_repulsion: f32,
    /// The panel's setting for how far a connection may stretch, in layers. Held here rather than
    /// read back off the simulation, which knows it only as the distance the layer spacing turns
    /// it into. See `LINK_REACH_DEFAULT`.
    link_reach: f32,
    /// See [`world::Routes::descendant_counts`].
    descendants: Vec<u32>,
    /// Drawn as a screen-filling quad before the graph. Not a [`Gm`] like the rest: it has no
    /// geometry of its own, only the material [`apply_screen_material`] stretches over the window.
    backdrop: ColorMaterial,
    /// The worlds: a picture of each on a camera-facing quad. Drawn only once the atlas they
    /// sample has arrived, there being nothing to sample until then.
    thumbnails: Gm<InstancedMesh, ColorMaterial>,
    /// One world's thumbnail again, blended over the node it is already drawn on to brighten it.
    /// Drawn only while something is [`AppEntities::pointed`] at, and aimed at its cell of the
    /// atlas by [`AppEntities::aim_glow`].
    glow: Gm<Mesh, ColorMaterial>,
    edges: Gm<InstancedMesh, ColorMaterial>,
    /// `EDGE_DASHES` per one-way connection, standing in for a solid line. See
    /// [`AppEntities::march_dashes`].
    dashes: Gm<InstancedMesh, DashMaterial>,
    /// Kept so each frame can rewrite the transformations while keeping the colors:
    /// `set_instances` replaces the whole [`Instances`] struct.
    thumbnail_instances: Instances,
    /// Only the two-way connections: the one-way ones are in `dash_instances` instead. Both are
    /// filled by walking the graph's edges in the one order it visits them in, so an edge always
    /// reaches the same slot of whichever of the two it belongs to.
    edge_instances: Instances,
    dash_instances: Instances,
    /// What a selection lights, drawn again over the finished scene so the layout cannot bury it.
    /// See [`AppEntities::gather_lit`].
    lit_thumbnails: Gm<InstancedMesh, ColorMaterial>,
    lit_edges: Gm<InstancedMesh, ColorMaterial>,
    /// Apart from `lit_edges` so a lit one-way connection still reads as one-way: the overlay's
    /// solid lines carry no dash material.
    lit_dashes: Gm<InstancedMesh, DashMaterial>,
    /// Compacted copies of the lit slots of the buffers above, rather than world-indexed as those
    /// are: nothing but the overlay's own draw reads them.
    lit_thumbnail_instances: Instances,
    lit_edge_instances: Instances,
    lit_dash_instances: Instances,
    /// Which slots the overlay copies, decided where the lighting itself is decided. See
    /// [`AppEntities::repaint`].
    lit_nodes: Vec<usize>,
    lit_lines: Vec<usize>,
    lit_dashed: Vec<usize>,
    /// Whether [`AppEntities::repaint`] has changed an instance colour since the last frame was
    /// built. A highlight moves no node, so nothing else in a settled graph would say so.
    recolored: bool,
    /// The world [`AppEntities::aim_glow`] last stood the glow quad over, so that a frame in
    /// which the pointer moved from one row to another is told apart from one in which it did
    /// not move at all.
    glowing: Option<usize>,
    /// The thumbnail atlas, from the ask through to the pictures being on screen. See [`Atlas`].
    atlas: Atlas,
    /// The same atlas as the sidebar's catalog draws out of. `None` until it arrives, and forever
    /// if it cannot be had.
    sheet: Option<thumbnails::Sheet>,
    /// `None` for a world the player has not been to, which wears the placeholder instead. See
    /// [`thumbnails::cells`].
    cells: Vec<Option<usize>>,
    /// The worlds drawn from the whole placeholder picture rather than from their cell of the
    /// atlas. Empty in a run drawing the whole game. See [`AppEntities::place_unvisited`].
    unvisited: Vec<usize>,
    /// The quads those are drawn on: the nodes' own, lifted toward the camera. Kept rather than
    /// built each frame, being one allocation the size of the frontier.
    unvisited_quads: Instances,
    detail: detail::Detail,
    /// How far the dashes have marched, in world units. Wrapped rather than
    /// counted up, so it stays exact however long the app runs.
    dash_phase: f32,
    /// The rotation that stood the quads square to the camera when their transformations were last
    /// built. Turning the camera has to rebuild them even though nothing in the layout has moved,
    /// and this is what says it has been turned.
    billboard: Mat4,
}

/// The dump, at whatever stage of arriving it has reached.
///
/// It comes off the network rather than out of the binary, so there is a stretch at the start of a
/// run with a window on screen and nothing to draw in it: see [`App::draw_loading`].
///
/// There is no state for having given up. Every reason a dump fails to arrive is a reason that
/// passes -- a server restarting, a laptop off the network, a phone in a tunnel -- and none is
/// worth leaving a person with a window they have to close and open again.
///
/// Kept once it arrives rather than dropped into the entities built from it, because a phone
/// rebuilds those every time the app comes back to the screen and the dump has not changed
/// meanwhile. See [`App::release`].
enum Dump {
    Loading(fetch::Pending<Result<Option<world::Dump>, String>>),
    /// Not this time, and this many seconds until the run asks again.
    ///
    /// Two things put a run here, and they are the same wait to sit through: the server building a
    /// dump and saying so rather than serving one it is about to replace, which is `why: None` and
    /// is over within the minute, or the ask coming to nothing at all, where `why` is what went
    /// wrong in the words the loading frame offers it in. Hence the two waits, see
    /// [`DUMP_ASKED_EVERY_SECONDS`] and [`DUMP_RETRIED_AFTER_SECONDS`].
    Waiting {
        why: Option<String>,
        until: f32,
    },
    Ready(world::Dump),
}

/// What the server says it is building, for the loading frame to say instead of the plain wait.
///
/// A server with no dump yet answers the fetch with `needs update` rather than a document, so that
/// fetch is not an account of how the wait is going -- this is. Asked on its own clock rather than
/// per frame, and only ever one ask at a time. See [`world::building`].
#[derive(Default)]
struct Building {
    /// The name of the message the last answer named.
    task: Option<&'static str>,
    asking: Option<fetch::Pending<Option<&'static str>>>,
    /// Seconds until the next ask. Starts at zero, so the first frame of a wait asks.
    until: f32,
}

impl Building {
    fn tick(&mut self, seconds: f32) {
        if let Some(asking) = &self.asking {
            if let Some(said) = asking.take() {
                let moved_on = said.is_some() && said != self.task;
                self.task = said;
                // The only account a run leaves of a wait that can last a minute. Logged as the
                // line on screen rather than the message name behind it, so the two read alike.
                if moved_on {
                    log::info!("the server is building the dump: {}", self.says());
                }
                self.asking = None;
            }
            // One at a time: an ask still in flight is the answer to how long to wait for it.
            return;
        }
        self.until -= seconds;
        if self.until <= 0.0 {
            self.until = BUILDING_ASKED_EVERY_SECONDS;
            self.asking = Some(fetch::spawn(world::building()));
        }
    }

    /// The stage the server named, or the plain wait.
    fn says(&self) -> String {
        match self.task {
            Some(said) => super::i18n::format(said, None),
            None => t!("dump-loading"),
        }
    }
}

#[derive(Default)]
struct AppContext {
    window: Option<Window>,
    wctx: Option<WindowedContext>,
    fig: Option<FrameInputGenerator>,
}

/// Whether the edges are smoothed. See [`antialias_remembered`].
const ANTIALIAS: &str = "antialias";
/// Whether the view leans onto a pointed world. See [`leaning_remembered`].
const LEANING: &str = "leaning";
impl ApplicationHandler for App {
    /// Makes a timed wake-up one-shot.
    ///
    /// winit re-arms a `WaitUntil` whose deadline has already passed with no delay at all rather
    /// than dropping it, so the deadline the loop stops at is not the end of it: the loop wakes on
    /// it again immediately, and goes on doing so until something replaces it. What replaces it is
    /// the frame this asks for. On a page that frame can be a long way off -- a browser serves no
    /// animation frames to a hidden tab, so a tab put in the background between the two would
    /// otherwise wake, ask, find the deadline still in the past, and spin there for as long as it
    /// stayed hidden.
    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        if matches!(cause, winit::event::StartCause::ResumeTimeReached { .. }) {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            self.ask_for_a_frame();
        }
    }
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        #[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
        let window_builder = Window::default_attributes()
            .with_title("yume 2kki world graph")
            .with_min_inner_size(winit::dpi::LogicalSize::new(1280, 720))
            .with_maximized(true);

        // A phone gives an app the whole screen and nothing to say about it.
        #[cfg(target_os = "android")]
        let window_builder = Window::default_attributes();

        #[cfg(target_family = "wasm")]
        let window_builder = {
            use wasm_bindgen::JsCast;
            use winit::platform::web::WindowAttributesExtWebSys;
            Window::default_attributes()
                .with_canvas(Some(
                    web_sys::window()
                        .unwrap()
                        .document()
                        .unwrap()
                        .get_elements_by_tag_name("canvas")
                        .item(0)
                        .expect("#canvas is missing")
                        .dyn_into::<web_sys::HtmlCanvasElement>()
                        .expect("#canvas is not a canvas"),
                ))
                .with_inner_size(winit::dpi::LogicalSize::new(1280, 720))
                .with_prevent_default(true)
        };
        self.reset(
            event_loop,
            event_loop.create_window(window_builder).unwrap(),
        );
    }
    /// Only a phone ever calls this. See [`App::release`].
    fn suspended(&mut self, _: &winit::event_loop::ActiveEventLoop) {
        self.release();
    }
    /// The last call the loop makes, and the only chance to let go of anything while the loop is
    /// still there to let go of it against: the app is a local of `super::run` and the loop is
    /// not, so everything held here otherwise outlives `run_app`.
    ///
    /// Two of those things cannot be let go of after it. The meshes and the overlay's painter free
    /// buffers of a graphics context that is gone by then, and the clipboard is a second Wayland
    /// connection on the display the loop owns, torn down on a thread of its own that reaches a
    /// freed display and takes the process with it.
    fn exiting(&mut self, _: &winit::event_loop::ActiveEventLoop) {
        self.call_the_camera_out();
        self.release();
    }
    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // Between a suspend and the next resume there is nothing to draw on and nothing that
        // could act on an event, but the loop keeps delivering them.
        let Some(fig) = self.ctx.fig.as_mut() else {
            return;
        };
        // Anything that is not the frame itself may have changed what the frame would draw, and
        // nothing is drawn here that was not asked for. See [`App::ask_for_a_frame`].
        let asks_for_a_frame = !matches!(event, WindowEvent::RedrawRequested);
        fig.handle_winit_window_event(&event);
        // Offered to the overlay as well, and to the scene either way: what the panel took is
        // settled after it has been laid out, not here. See `Overlay::run`.
        if let Some(overlay) = self.overlay.as_mut() {
            overlay
                .gui
                .on_window_event(self.ctx.window.as_ref().unwrap(), &event);
        }
        match event {
            WindowEvent::Resized(physical_size) => {
                self.ctx.wctx.as_ref().unwrap().resize(physical_size);
            }
            WindowEvent::RedrawRequested => {
                self.asked = false;
                self.stats.began(self.ctx.wctx.as_ref().unwrap());
                let began = web_time::Instant::now();
                let rebuilt = self.draw();
                // Taken before the buffers are swapped, which is where the wait for the display
                // is: this is the frame's own cost, not the rate it is shown at.
                self.stats.frame(began.elapsed(), rebuilt);
                // After the frame rather than before it, so the page never has neither.
                #[cfg(target_family = "wasm")]
                take_placeholder();
                self.ctx.wctx.as_ref().unwrap().swap_buffers().unwrap();
                self.pace(event_loop);
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            // A key held as the window is left never comes back up, so the view would go on
            // walking. Nothing else three-d reports says the window stopped hearing the keyboard.
            WindowEvent::Focused(false) => {
                self.statics.walk = Walk::default();
                self.statics.focused = false;
            }
            WindowEvent::Focused(true) => self.statics.focused = true,
            WindowEvent::Touch(touch) => {
                // A second finger settles what the first was doing: not a tap, not a node drag and
                // not an orbit, but the start of a pinch.
                if self.statics.touches.track(&touch)
                    && let Some(data) = self.data.as_mut()
                {
                    data.gesture = None;
                }
            }
            _ => (),
        }
        if asks_for_a_frame {
            self.ask_for_a_frame();
        }
    }
}

impl App {
    /// Lets go of everything built on the window, in the order the pieces were built in: the
    /// meshes and the overlay have to give their buffers back while the context that holds them is
    /// still there, and the context has to let the surface go before the window it was made
    /// against.
    ///
    /// Called on the way out by [`App::exiting`], and on a phone by [`App::suspended`] too:
    /// Android takes the drawing surface back whenever the app leaves the screen, so everything on
    /// it is built again by [`App::resumed`] -- the same path a first start takes, and therefore a
    /// fresh layout.
    fn release(&mut self) {
        // Read before the entities holding it go: see [`App::selected`].
        self.selected = self.data.as_ref().and_then(|data| data.selected);
        self.data = None;
        // A dropped `gui::Gui` does not give the context's buffers back on its own.
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.gui.destroy();
        }
        self.overlay = None;
        self.ctx.fig = None;
        self.ctx.wctx = None;
        self.ctx.window = None;
    }
    /// Says where the camera was left, in the syntax of the literal in [`App::new`]: the angle a
    /// run opens from is picked by turning the graph to one worth opening on and copying this line
    /// out of the log. The layout is the same every run -- see `scatter` -- so the numbers mean
    /// the same thing next time, but only within the dimensions they were read in.
    fn call_the_camera_out(&self) {
        let (eye, at) = (self.statics.camera.position(), self.statics.camera.target());
        let dimensions = self
            .data
            .as_ref()
            .map_or(Dimensions::Three, |data| data.graph.parameters().dimensions);
        log::info!(
            "camera in {dimensions:?} dimensions: \
             vec3({:.1}, {:.1}, {:.1}), vec3({:.1}, {:.1}, {:.1})",
            eye.x,
            eye.y,
            eye.z,
            at.x,
            at.y,
            at.z,
        );
    }

    pub fn new() -> Self {
        let camera = Camera::new_perspective(
            Viewport::new_at_origo(1, 1),
            // Only which way a run looks from: where it looks and how far off it stands are
            // framed onto the tree instead, in [`App::build`]. Picked by hand off a settled
            // three-dimensional layout.
            vec3(-49.7, -47.1, -31.0),
            vec3(218.5, -173.5, 180.6),
            vec3(0.0, 1.0, 0.0),
            degrees(FOV_Y_DEGREES),
            0.1,
            1000.0,
        );
        // Read out rather than off the control below, which is moved into place before the lean's
        // own copy of it would be taken.
        let centre = camera.target();
        let control = OrbitControl::new(centre, 1.0, 500.0);
        // Nothing waits on this or holds it: it publishes what it loads where the app reads it.
        drop(fetch::spawn(world::load_pages()));

        Self {
            ctx: AppContext::default(),
            // Started here rather than at the first frame: the fetch is the longest thing a run
            // waits for, and nothing it needs is owned by the window.
            dump: Dump::Loading(fetch::spawn(world::load())),
            // For the same reason as the dump: a run that left a session behind has an account to
            // read before it has a window to draw it in.
            yno: yno::Account::new(),
            before: None,
            statics: AppStatics {
                control,
                camera,
                panning: false,
                cursor: CursorIcon::Default,
                touches: Touches::default(),
                walk: Walk::default(),
                lean_aim: centre,
                focused: true,
            },
            data: None,
            overlay: None,
            building: Building::default(),
            selected: None,
            asked_for: link::location(),
            said: String::new(),
            veil: 1.0,
            wanted: Wanted::Now,
            asked: false,
            still: 0.0,
            stats: FrameStats::new(),
        }
    }
    fn reset(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, window: Window) {
        self.ctx.window = Some(window);
        let window = self.ctx.window.as_ref().unwrap();
        let surface = SurfaceSettings {
            vsync: !profile::unlocked(),
            // Read here rather than carried, so the surface and the overlay that reports it are
            // built from one answer. See [`Overlay::antialias_running`].
            multisamples: match antialias_remembered() {
                true => MULTISAMPLES,
                false => 0,
            },
            ..Default::default()
        };
        self.ctx.wctx = Some(WindowedContext::from_winit_window(window, surface).unwrap());
        let ctx = self.ctx.wctx.as_ref().unwrap();
        self.ctx.fig = Some(FrameInputGenerator::from_winit_window(window));
        self.overlay = Some(Overlay::new(event_loop, window, ctx));
        self.build();
        // Nothing else would: the window is drawn on demand, and this is the first demand.
        self.asked = false;
        self.ask_for_a_frame();
    }

    /// Builds the graph, once there is a dump to build it out of, and does nothing until there
    /// is: a run still fetching one draws [`App::draw_loading`] until [`App::draw`] finds the dump
    /// has landed and calls this again.
    ///
    /// On a phone it is also called by every [`App::reset`], the entities all being built on a
    /// graphics context the framework takes back whenever the app leaves the screen.
    fn build(&mut self) {
        let Dump::Ready(dump) = &self.dump else {
            return;
        };
        // A run drawing one player's game has to know which worlds are theirs before it draws
        // anything: building on the dump alone would put the whole game on screen and take it away
        // a moment later. See `yno::Account::settled`.
        if !self.yno.settled() {
            return;
        }
        // The frontier is a different graph rather than a different drawing of one: it takes
        // worlds out, and a connection names the world it leads to by index. Applied here, where
        // the dump becomes something to build from, so nothing downstream has to know.
        let frontier;
        let dump = match self.yno.frontier() {
            Some(visited) => {
                frontier = dump.showing(&visited);
                &frontier
            }
            None => dump,
        };
        let ctx = self.ctx.wctx.as_ref().unwrap();
        // Whether this graph is starting rather than carrying one on, which settles whether the
        // camera is the person's to keep.
        let opening = self.before.is_none();
        // Taken rather than borrowed: it belongs to the graph being replaced, and the one built
        // here is the last thing with any use for it.
        let mut data = entities(dump, self.before.take().as_ref(), ctx);
        // A graph solved on the plane has to be looked at square on, or the layout it was given
        // depth to avoid is seen edge on anyway. The switch does this when thrown, but a run
        // opening in two dimensions because that is where an earlier run left it never passes
        // through the switch and would open oblique.
        //
        // Only on a graph that is opening: one carrying another on keeps whatever the person had
        // turned the camera to, as its layout and its selection do.
        if opening && data.graph.parameters().dimensions == Dimensions::Two {
            self.statics.face_plane();
        }

        // A link's world is lit the way picking it out of the search box would light it. A name
        // this graph has not got is not an error: a stale link opens on the tree.
        if let Some(name) = self.asked_for.take()
            && self.selected.is_none()
        {
            self.selected = data.world_named(&name).map(Highlight::Route);
            if self.selected.is_none() {
                log::warn!("no world is named {name}");
            }
        }
        // Lit again where a resume dropped it, which also aims the camera: this is a selection in
        // a new layout, so where the camera was looking is nowhere in particular.
        if self.selected.is_some() {
            data.select(self.selected);
        }
        // Where a graph with nothing lit opens: on the tree, framed as picking it out would frame
        // it, and snapped rather than eased -- there is no view yet to travel from.
        if let Some(bounds) = (opening && data.selected.is_none())
            .then(|| data.framing_bounds())
            .flatten()
        {
            self.statics.snap_to_frame(&bounds);
            data.framing = true;
        }
        self.data = Some(data);
    }
}

impl App {
    /// The frame drawn while there is no graph: the background, and a word about why. The whole of
    /// the app until the dump lands, however many asks that takes.
    ///
    /// Everything the overlay usually reads is built out of the dump, so none of the panel can be
    /// laid out here -- only the one message, over the same cleared background the graph is drawn
    /// on, so the window does not change colour when the graph arrives.
    fn draw_loading(&mut self) {
        // Whole for as long as this is the frame: what fades is what is left once the graph draws.
        self.veil = 1.0;
        let ctx = self.ctx.wctx.as_ref().unwrap();
        let frame_input = self.ctx.fig.as_mut().unwrap().generate(ctx);
        let window = self.ctx.window.as_ref().unwrap();
        let seconds = (frame_input.elapsed_time as f32 * 1e-3).min(0.05);
        // Only worth asking of a server that is answering: a host that could not be reached has
        // nothing to say about a dump it is not building, and one whose dump arrived has nothing
        // left to say at all.
        if matches!(
            self.dump,
            Dump::Loading(_) | Dump::Waiting { why: None, .. }
        ) {
            self.building.tick(seconds);
        }
        // The clock every wait ends on: however the last ask turned out, the next one is what this
        // run does about it.
        if let Dump::Waiting { until, .. } = &mut self.dump {
            *until -= seconds;
            if *until <= 0.0 {
                self.dump = Dump::Loading(fetch::spawn(world::load()));
            }
        }
        let says = match &self.dump {
            // The dump is here and the graph is not, which leaves one thing being waited for.
            Dump::Ready(_) => t!("yno-loading"),
            Dump::Waiting { why: Some(_), .. } => t!("dump-failed"),
            _ => self.building.says(),
        };
        // What went wrong, for the reader who goes looking. The frame keeps spinning either way,
        // because either way it will ask again.
        let failed = match &self.dump {
            Dump::Waiting { why, .. } => why.as_deref(),
            _ => None,
        };
        // Kept for the fade that follows this frame, which carries out whichever line was on it.
        self.said = says.clone();
        let overlay = self.overlay.as_mut().unwrap();
        // The same scale the panel is laid out at, so the message is the size the rest of the
        // interface will be.
        overlay.gui.context().set_zoom_factor(overlay.ui_scale);
        // As in `Overlay::run`: the face Japanese needs is not compiled in, and this frame is the
        // first that may want it.
        let context = overlay.gui.context().clone();
        overlay.japanese.serve(&context);
        overlay
            .gui
            .run(window, |ui| loading_frame(ui.ctx(), &says, failed, 1.0));
        frame_input
            .screen()
            .clear(ClearState::color_and_depth(
                BACKGROUND_COLOR[0],
                BACKGROUND_COLOR[1],
                BACKGROUND_COLOR[2],
                1.0,
                1.0,
            ))
            .write::<std::convert::Infallible>(|| {
                overlay.gui.paint(window);
                Ok(())
            })
            .unwrap();
    }

    /// One frame: takes in whatever arrived, moves the camera, steps the layout, and draws it.
    /// Answers whether the frame got as far as rebuilding the layout's geometry, which is what
    /// [`FrameStats`] counts.
    fn draw(&mut self) -> bool {
        // Until the end of the frame says otherwise. The loading frame below returns through
        // here, and it animates.
        self.wanted = Wanted::Now;
        // Whether or not there is a graph yet: a run that resumed a session is reading an account
        // while the dump is still on its way, and the answer has to be in hand before the graph is
        // built out of it.
        self.yno.poll();
        if self.yno.restated() {
            // A frontier is a different numbering of the worlds, so what was lit means nothing in
            // the graph about to be built.
            self.selected = None;
            // Where this graph had got to, for the next to carry on from: the person is looking at
            // a map, and a refresh should add to it rather than replace it.
            self.before = self.data.as_ref().map(AppEntities::before);
            self.data = None;
            self.build();
        }
        if self.data.is_none() {
            // Only while there is nothing to draw: every frame after this one has a graph in it
            // and nothing left to wait for.
            let arrived = match &self.dump {
                Dump::Loading(pending) => pending.take(),
                _ => None,
            };
            if let Some(loaded) = arrived {
                self.dump = match loaded {
                    Ok(Some(dump)) => Dump::Ready(dump),
                    // Not an error and not a dump: the server is building one.
                    Ok(None) => Dump::Waiting {
                        why: None,
                        until: DUMP_ASKED_EVERY_SECONDS,
                    },
                    // Not the end of the run: everything that stops a dump arriving is something
                    // that passes, so the loading frame goes on saying so and asking again.
                    Err(error) => {
                        log::warn!("{error}");
                        Dump::Waiting {
                            why: Some(error),
                            until: DUMP_RETRIED_AFTER_SECONDS,
                        }
                    }
                };
                self.build();
            }
            if self.data.is_none() {
                self.draw_loading();
                return false;
            }
        }
        let ctx = self.ctx.wctx.as_ref().unwrap();
        let mut frame_input = self.ctx.fig.as_mut().unwrap().generate(ctx);
        let window = self.ctx.window.as_ref().unwrap();

        // Clamped as the layout's own step is: the frame the graph was built on is a long one, and
        // the reveal should not be spent paying for it.
        if self.veil > 0.0 {
            let step = (frame_input.elapsed_time as f32 * 1e-3).min(0.05);
            self.veil = (self.veil - step / LOADING_FADE_SECONDS).max(0.0);
        }
        // Whatever the loading frame was saying when it gave way, still saying it as it goes.
        let fading = (self.veil > 0.0).then(|| (self.said.clone(), self.veil));
        let data = self.data.as_mut().unwrap();
        self.statics.camera.set_viewport(frame_input.viewport);
        profile::pan_aside(&mut self.statics.camera);
        let account = &mut self.yno;
        // The whole game, which is the yardstick the settings tab measures one person's share
        // against: the graph beside it may be only the frontier.
        let dump = match &self.dump {
            Dump::Ready(dump) => Some(dump),
            _ => None,
        };
        if self
            .overlay
            .as_mut()
            .unwrap()
            .run(window, &mut frame_input, data, account, dump, fading)
        {
            // Repulsion acts along the offset between two nodes, so a layout flattened onto the
            // plane has no depth for the forces to reinflate. Leaving two dimensions has to reseed
            // that axis, and restarting the whole layout is cheaper than special-casing it.
            scatter(data);
            if data.graph.parameters().dimensions == Dimensions::Two {
                // Squared onto the plane before the turn is locked, or a camera left oblique by
                // the three-dimensional view would stay that way with no way to straighten it.
                self.statics.face_plane();
            }
        }
        // Ahead of the pan and the orbit control, which both take whatever left-button motion this
        // leaves unhandled.
        data.track_gesture(
            &self.statics.camera,
            &mut frame_input.events,
            self.statics.touches.pinching,
        );
        if data.graph.parameters().dimensions == Dimensions::Two {
            // A flat layout has one face worth looking at. Swallowing the drag the orbit control
            // reads leaves it the zoom and leaves the pan alone.
            lock_rotation(&mut frame_input.events);
        }
        // Two fingers do both jobs at once off the same pair of positions: three-d reads the gap
        // between them as the wheel the orbit control zooms on, and their midpoint's travel is the
        // pan. Independent, so a drag that also spreads does both.
        let dragged = self
            .statics
            .touches
            .take_travel(frame_input.device_pixel_ratio);
        // Read after the overlay, which settles whether the keys are being typed into the search
        // box rather than walked with.
        self.statics
            .walk
            .track(&frame_input.events, self.overlay.as_ref().unwrap().keyboard);
        let (across, into) = self
            .statics
            .walk
            .travel((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
        let walking = across != 0.0 || into != 0.0;
        let panned = self
            .statics
            .pan(&mut frame_input.events, frame_input.device_pixel_ratio)
            | self.statics.pan_by(dragged, frame_input.device_pixel_ratio)
            | self
                .statics
                .pan_by((across, 0.0), frame_input.device_pixel_ratio)
            | self.statics.dolly_by(into, frame_input.device_pixel_ratio);
        let orbited = self
            .statics
            .control
            .handle_events(&mut self.statics.camera, &mut frame_input.events);
        // The camera belongs to whoever last touched it: an orbit, a pan or a zoom abandons the
        // framing and the lean rather than fighting either for the rest of the move. `panned`
        // carries the walk keys and the dolly as well, so this is every way the camera is asked
        // for by hand.
        if panned || orbited {
            data.framing = false;
            data.leaning_at = None;
        }
        let orbiting = matches!(data.gesture, Some(Gesture::Orbiting))
            && data.graph.parameters().dimensions == Dimensions::Three;
        self.statics
            .track_cursor(self.ctx.window.as_ref().unwrap(), orbiting);
        let dt = (frame_input.elapsed_time as f32 * 1e-3).min(0.05);
        if data.framing {
            // Recomputed every frame rather than fixed when the selection was made: the layout is
            // usually still moving, and a goal taken once would be stale before the camera got
            // there.
            // What is arriving comes first: while a refresh is coming in, the camera moves onto it
            // rather than onto whatever was lit before. Once it is over the selection has the
            // camera back, and a run with neither stops framing.
            data.framing = match data.arrival_bounds() {
                Some(bounds) => {
                    // Kept on whether or not the camera has caught up, unlike a selection: what it
                    // follows is still being pushed about by the worlds that landed in it.
                    self.statics.ease_to_frame(&bounds, dt);
                    true
                }
                None => match data.framing_bounds() {
                    Some(bounds) => {
                        let easing = self.statics.ease_to_frame(&bounds, dt);
                        // An opening frame is kept whether or not the camera has caught up, as an
                        // arrival is: the tree is still spreading out of its scatter, so arriving
                        // only means arriving at how far it had got.
                        easing || (data.selected.is_none() && !data.graph.is_settled())
                    }
                    None => false,
                },
            };
        }

        // Latched rather than read off `pointed` each frame: a lean carries on to the world it was
        // given after the pointer has left the row, so letting go of a row is not what stops it --
        // taking the camera is.
        if let Some(world) = data.pointed {
            data.leaning_at = Some(world);
        }
        // A framing move owns the camera while it runs, so the lean waits rather than dragging on
        // the goal that move is easing onto and leaving it never arrived.
        let leaning = self.statics.lean_toward(
            (self.overlay.as_ref().unwrap().leaning && !data.framing)
                .then(|| data.leaning_at.and_then(|world| data.world_at(world)))
                .flatten(),
            data.graph.parameters().dimensions,
            dt,
        );

        data.pull_grabbed_node(&self.statics.camera);
        data.receive_atlas(
            (frame_input.elapsed_time as f32 * 1e-3).min(0.05),
            ctx,
            self.overlay.as_ref().unwrap().gui.context(),
        );
        // Unclamped: the layout steps at a fixed rate and caps how much of a long frame it catches
        // up on itself, so a stalled tab is already its problem. What it returns is whether there
        // is geometry to rebuild.
        let stepping = web_time::Instant::now();
        let stepped = data.graph.update(frame_input.elapsed_time as f32 * 1e-3);
        self.stats.stepped(stepping.elapsed());
        // The quads face the camera, so turning it dates their transformations even over a layout
        // that has not moved at all.
        let turned = data.billboard != billboard(&self.statics.camera);
        // Clamped as the reveal is, and for the same reason: an arrival should not be half over
        // before it is first drawn.
        let arriving = data
            .arrivals
            .tick((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
        // Everything below reads the node quads, the camera, or the instance colors, so this is
        // what says any of it has to be worked out again.
        let recolored = std::mem::take(&mut data.recolored);
        let moved = stepped || turned || arriving || recolored || profile::eager();
        if moved {
            data.rebuild_instances(&self.statics.camera);
        }
        // Whether or not anything else moved: see [`AppEntities::march_dashes`].
        data.march_dashes((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
        // Both of these are also where a picture that has arrived is taken out of its fetch, and a
        // picture can arrive on a frame that moved nothing at all -- so a frame with one still on
        // its way does them whether or not anything moved. Skipping that would strand the fetch:
        // nothing else reads it, so it would stay pending for good.
        let reading = data.detail.pending();
        // After the instances, whose colors the full pictures borrow. Coming in on a node changes
        // nothing in the layout and everything about how much of the atlas the screen asks for,
        // but coming in on one is a camera move and so is already a frame that moved.
        if moved || reading {
            let magnified = data.magnified(&self.statics.camera, frame_input.viewport);
            data.detail.track(ctx, &magnified);
        }
        if moved || reading {
            // What it copies is the node quads rebuilt just above, and the camera it is lifted
            // against is the one that turned them.
            data.place_unvisited(ctx, &self.statics.camera);
        }
        // Also on a frame that only moved the pointer from one list row to another, which moves
        // the glow without moving anything it is copied from.
        if moved || data.glowing != data.pointed {
            data.glowing = data.pointed;
            data.aim_glow(&self.statics.camera);
        }

        if let Some(texture) = data.backdrop.texture.as_mut() {
            texture.transformation = panorama_transform(
                frame_input.viewport,
                frame_input.device_pixel_ratio,
                self.statics.camera.view_direction(),
            );
        }
        let screen = frame_input.screen();
        // Depth only: the backdrop written straight after covers every pixel of the target,
        // with the depth test off and no blending, so the color is written twice otherwise.
        //
        // Which way this goes depends on the renderer, and only one of the two has been
        // measured. An IMR keeps the target in memory, so this is a write saved. A TBDR
        // resolves per tile and the clear is its LoadOp: drop it and every tile loads the last
        // frame back instead. Phones are all TBDR, as is Apple silicon. Measure there first.
        screen.clear(ClearState::depth(1.0));
        // Split only to fix the order: one call would sort these by each mesh's centre, which says
        // nothing about which covers which. Pictures before lines, so the depth they write drops
        // the lines behind them unshaded. All opaque bar the glow, so only the skipping changes.
        let camera = &self.statics.camera;
        self.stats.timed("backdrop", ctx, || {
            screen
                .write::<std::convert::Infallible>(|| {
                    apply_screen_material(ctx, &data.backdrop, camera, &[]);
                    Ok(())
                })
                .unwrap();
            1
        });
        self.stats.timed("pictures", ctx, || {
            let mut calls = 0;
            screen.render(
                camera,
                data.drawn_thumbnails()
                    .into_iter()
                    .chain(data.detail.drawn())
                    .inspect(|_| calls += 1),
                &[],
            );
            calls
        });
        self.stats.timed("lines", ctx, || {
            let mut calls = 0;
            screen.render(
                camera,
                data.edges
                    .into_iter()
                    .chain(&data.dashes)
                    .inspect(|_| calls += 1),
                &[],
            );
            calls
        });
        // What the selection lights, again, over everything the passes above drew. Nothing at all
        // while nothing is selected, the clear included.
        self.stats.timed("lit", ctx, || {
            let mut calls = 0;
            if data.lit_anything() {
                // Depth only, and only the scene's own: what the overlay has to be in front of is
                // whatever the layout parked between it and the eye, and clearing that is what
                // lets the overlay keep a real depth test among its own worlds. A clear inside a
                // pass is a tile op rather than a resolve, so this is cheap on a TBDR too.
                screen.clear(ClearState::depth(1.0));
                screen.render(camera, data.drawn_lit().inspect(|_| calls += 1), &[]);
            }
            calls
        });
        // Last of the scene, because it brightens whichever pass above drew the world it is over.
        self.stats.timed("glow", ctx, || {
            let glow = data.drawn_glow();
            screen.render(camera, glow, &[]);
            u32::from(glow.is_some())
        });
        // Over the scene, into the same target, which is what makes it an overlay.
        let overlay = self.overlay.as_mut().unwrap();
        self.stats.timed("panel", ctx, || {
            let mut calls = 0;
            screen
                .write::<std::convert::Infallible>(|| {
                    calls = overlay.gui.paint(window) as u32;
                    Ok(())
                })
                .unwrap();
            calls
        });

        // Last, so that everything able to start something moving has had its turn.
        let data = self.data.as_ref().unwrap();
        // `moved` is what the geometry was rebuilt for, which covers the layout stepping, the
        // camera turning and a selection repainting; the rest is movement that leaves the frame
        // it started in looking the same -- a pan or a dolly, which turn nothing, and the reveal.
        // A lean turns in three dimensions and pans in two, and either way carries on easing after
        // the turn it is making has grown too small to date a billboard.
        let moving = moved
            || panned
            || orbited
            || walking
            || leaning
            || self.veil > 0.0
            || data.framing
            || data.gesture.is_some()
            || !data.graph.is_settled();
        self.still = match moving {
            true => 0.0,
            false => self.still + frame_input.elapsed_time as f32 * 1e-3,
        };
        self.wanted = if profile::eager() {
            Wanted::Now
        } else {
            [
                // Moving, or not long enough ago to start pacing the view by anything but the
                // display. See [`IDLE_AFTER_SECONDS`].
                (self.still < IDLE_AFTER_SECONDS).then_some(Wanted::Now),
                // A fade in the panel, a blinking caret, a tooltip about to show itself.
                Some(Wanted::after(
                    self.overlay.as_ref().unwrap().gui.repaint_after(),
                )),
                // The one thing that goes on moving over a settled layout, and so the only reason
                // a graph nothing else is happening to is drawn at all. See [`AppStatics::focused`].
                (!data.dash_instances.transformations.is_empty()
                    && (self.statics.focused || cfg!(target_family = "wasm")))
                .then(|| Wanted::after_hz(IDLE_REDRAW_HZ)),
                // Nothing is moving; a frame is drawn only because reading what has arrived is
                // something only a frame does. See [`App::draw`].
                (self.yno.asking()
                    || matches!(data.atlas, Atlas::Loading(_))
                    || data.detail.pending())
                .then(|| Wanted::after_hz(POLL_REDRAW_HZ)),
            ]
            .into_iter()
            .flatten()
            .fold(Wanted::Never, Wanted::or_sooner)
        };
        moved
    }

    /// Turns what the frame just drawn asked for into how the loop waits. See [`Wanted`].
    fn pace(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        use winit::event_loop::ControlFlow;
        let flow = match self.wanted {
            // The redraw request below is what wakes the loop; the wait is what it falls back to
            // once the frame has been drawn.
            Wanted::Now | Wanted::Never => ControlFlow::Wait,
            Wanted::After(delay) => ControlFlow::wait_duration(delay),
        };
        event_loop.set_control_flow(flow);
        if self.wanted == Wanted::Now {
            self.ask_for_a_frame();
        }
    }

    /// Asks for a frame, unless one has already been asked for and not yet drawn.
    ///
    /// The guard is not tidiness. On the page a request is an animation frame, and winit serves a
    /// second one by cancelling the first: a stream of input -- a wheel being turned, a pointer
    /// being dragged -- arrives faster than the display refreshes, so asking on every event
    /// cancels the frame that was about to be drawn, over and over, and the view is left running
    /// at whatever survives that rather than at the display's rate.
    fn ask_for_a_frame(&mut self) {
        if self.asked {
            return;
        }
        if let Some(window) = self.ctx.window.as_ref() {
            self.asked = true;
            window.request_redraw();
        }
    }
}
