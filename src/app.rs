//! The app the event loop drives: the window and the context everything else is built on, and the
//! state a frame reads. See [`App::draw`] in [`frame`] for the main flow, [`loading`] for the
//! frames before there is a graph, and [`pacing`] for when the next one is wanted.

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

mod arrivals;
mod camera;
mod detail;
mod download;
mod fetch;
mod frame;
mod graph;
mod gui;
mod guide;
pub(crate) mod i18n;
mod japanese;
mod layout;
mod link;
mod loading;
mod map;
mod pacing;
#[cfg(feature = "profile")]
mod profile;
mod scene;
mod store;
#[cfg(target_family = "wasm")]
mod text_agent;
mod thumbnails;
mod ui;
#[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
mod update;
mod world;
mod yno;

use arrivals::*;
use camera::*;
use graph::*;
use i18n::t;
use layout::*;
use loading::*;
use pacing::*;
use profile::FrameStats;
use scene::*;
use ui::*;
use world::{Conditions, Gate};

use crate::i18n::speaking_japanese;

/// Handed to `android_main` once, and the only way to reach anything the framework owns. Kept for
/// [`thumbnails`]' assets and [`Overlay`]'s safe area and soft keyboard, which both want it later.
#[cfg(target_os = "android")]
static ANDROID: std::sync::OnceLock<winit::platform::android::activity::AndroidApp> =
    std::sync::OnceLock::new();

/// Everything that wants the handle runs too late to be given it directly.
#[cfg(target_os = "android")]
pub(crate) fn use_android_app(app: winit::platform::android::activity::AndroidApp) {
    let _ = ANDROID.set(app);
}

/// Samples the surface is built with when the edges are being smoothed, and the frame's largest
/// cost by a distance -- measured, by turning it off.
///
/// No setting exposes it: wasm ignores the count, WebGL taking a boolean and the browser choosing.
const MULTISAMPLES: u8 = 4;

/// Whether the surface smooths the edges, which is settled as it is built.
///
/// Multisampling rather than a pass over the finished frame: the graph is mostly thin
/// high-contrast cylinders, and post-processing measured no cheaper on a desktop and dearer on
/// the page.
fn antialias_remembered() -> bool {
    store::read(ANTIALIAS).as_deref() != Some("off")
}

fn antialias_remember(on: bool) {
    store::write(ANTIALIAS, Some(if on { "on" } else { "off" }));
}

/// Whether the view leans onto a world a list row is pointing at, which the panel's switch sets.
/// Defaults to what the platform already says about motion. See [`AppStatics::lean_toward`].
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

/// Whether the person prefers less movement than an app would otherwise make.
///
/// Only the page can answer: winit carries no such preference, and the APIs behind it are one per
/// platform. Everywhere else the switch is the only answer, and it opens on.
fn reduced_motion() -> bool {
    #[cfg(target_family = "wasm")]
    {
        // A browser that will not answer is one that was never told to hold back.
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
    /// How far into the code the keys typed at the graph have got. See [`world::Code`].
    code: world::Code,
    /// Whether it has been typed, which every dump fetched after it is read under: a retry must
    /// not read those worlds back out again.
    revealed: bool,
    /// What the graph being replaced was, held between the frame that throws it away and the one
    /// that builds the next out of it. `None` at every other moment. See [`Before`].
    before: Option<Before>,
    building: Building,
    /// What was lit when the drawing surface was last given back, to light again once there is a
    /// graph to light it in.
    ///
    /// Kept outside [`AppEntities`], which a phone drops every time the app leaves the screen --
    /// see [`App::release`]. The layout is built afresh, so the camera is sent to it again.
    selected: Option<Highlight>,
    /// The world the link opens on, until there is a graph to find it in. Taken by the first
    /// [`App::build`], which turns it into a selection like any other.
    opening_on: Option<String>,
    /// The line the loading frame was last showing, held so the fade does not change it on the
    /// way out. See [`App::veil`].
    said: String,
    /// How much of the loading frame is still on screen, 1 for all of it and 0 for none.
    ///
    /// Written back to 1 by every [`App::draw_loading`], so a run that waits again fades again.
    veil: f32,
    /// When the frame just drawn wants the next one. Read by the event loop, which is where the
    /// request is turned into a wait. See [`Wanted`].
    wanted: Wanted,
    /// Whether a frame has been requested and not yet drawn. See [`App::request_a_frame`].
    requested: bool,
    /// Seconds since anything on screen last moved. See [`pacing::IDLE_AFTER_SECONDS`].
    still: f32,
    stats: FrameStats,
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
    /// The ways [`Highlight::Path`] found, freest first and never two the same. Empty for every
    /// other selection, and for a pair with no way between them.
    ways: Vec<world::Way>,
    /// Which of `ways` is drawn and read out. See [`AppEntities::take_way`].
    way: usize,
    /// The world the Eyeball Bomb effect gets a player back to from anywhere, which the ways
    /// between two worlds are walked with. See [`world::hub_world`].
    hub: Option<usize>,
    /// The depth of the furthest world the origin can reach, which is the last layer the rocker
    /// can step to.
    deepest: u32,
    /// Where the right button went down, while it is still down. The button both pans and opens
    /// the menu, and only the travel between press and release tells those apart.
    right_press: Option<PhysicalPoint>,
    /// Drawn by the overlay, which also closes it once one of its entries is taken.
    menu: Option<ContextMenu>,
    /// Where the pointer last was with nothing pressed, in physical pixels, and what the hover
    /// test found there. `None` off the window or over the panel, and always on a touch screen: a
    /// finger never moves without pressing.
    cursor: Option<PhysicalPoint>,
    hover: Option<usize>,
    /// The world a sidebar list row is pointing at, brightened where it sits in the graph and
    /// cleared as soon as the pointer leaves the row. Only ever one: see `Panel::pointed`.
    pointed: Option<usize>,
    /// The world the view is leaning onto, which outlives the pointing that set it. `None`
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
    /// list. See [`Highlight::Untaken`].
    untaken: Vec<usize>,
    /// The world a run opens framed on, with everything behind it: `None` in a dump that does not
    /// hold it, which opens on the camera's own pose instead. See [`opening_room`].
    opening: Option<usize>,
    /// Cleared once the camera arrives at the selected route, or as soon as the person takes the
    /// camera back.
    framing: bool,
    /// Whether a framed route is framed whole rather than on the world at its end. Held across
    /// selections, being a way of looking rather than a fact about one route.
    frame_route: bool,
    /// Connection colors before a selection dims them, so clearing one restores the distance ramp.
    edge_colors: Vec<Srgba>,
    /// Per node, the half-height of its quad, from how much of the graph hangs off it.
    node_radii: Vec<f32>,
    arrivals: Arrivals,
    /// The panel's setting for how much of a world's size goes into how hard it pushes. See
    /// [`node_mass`].
    hub_repulsion: f32,
    /// The panel's setting for how far a connection may stretch, in layers. Held here because the
    /// simulation knows it only as the distance the layer spacing turns it into. See
    /// `LINK_REACH_DEFAULT`.
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
    /// Drawn only while something is [`AppEntities::pointed`] at. See [`AppEntities::aim_glow`].
    glow: Gm<Mesh, ColorMaterial>,
    edges: Gm<InstancedMesh, ColorMaterial>,
    /// `EDGE_DASHES` per one-way connection, standing in for a solid line. See
    /// [`AppEntities::march_dashes`].
    dashes: Gm<InstancedMesh, DashMaterial>,
    /// Kept so each frame can rewrite the transformations while keeping the colors:
    /// `set_instances` replaces the whole [`Instances`] struct.
    thumbnail_instances: Instances,
    /// Only the two-way connections: the one-way ones are in `dash_instances` instead. Both are
    /// filled in the graph's own edge order, so an edge always reaches the same slot.
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
    /// The world [`AppEntities::aim_glow`] last stood the glow quad over, which tells a pointer
    /// that moved between rows from one that did not move at all.
    glowing: Option<usize>,
    /// The thumbnail atlas, from the request through to the pictures being on screen. See
    /// [`Atlas`].
    atlas: Atlas,
    /// The same atlas as the sidebar's catalog draws out of. `None` until it resolves, and forever
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
    lit_unvisited_quads: Instances,
    detail: detail::Detail,
    /// How far the dashes have marched, in world units. Wrapped rather than
    /// counted up, so it stays exact however long the app runs.
    dash_phase: f32,
    /// The rotation that stood the quads square to the camera when their transformations were
    /// last built: a turned camera rebuilds them even though nothing in the layout has moved.
    billboard: Mat4,
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
    /// winit re-arms a `WaitUntil` whose deadline has passed rather than dropping it, so the loop
    /// spins on it until a frame replaces it -- and a hidden tab is served no animation frames, so
    /// that frame can be a long way off.
    fn new_events(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) {
        if matches!(cause, winit::event::StartCause::ResumeTimeReached { .. }) {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::Wait);
            self.request_a_frame();
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
    /// The last call the loop makes, and the only point at which two things can still be dropped
    /// safely: the meshes and the overlay's painter free buffers of a graphics context that is
    /// gone after it, and the clipboard's Wayland connection would otherwise be torn down against
    /// a freed display, taking the process with it.
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
        // nothing is drawn here that was not requested. See [`App::request_a_frame`].
        let wants_a_frame = !matches!(event, WindowEvent::RedrawRequested);
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
                self.requested = false;
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
        if wants_a_frame {
            self.request_a_frame();
        }
    }
}

impl App {
    /// Lets go of everything built on the window, innermost first: buffers have to be freed while
    /// the context still holds them, and the context before the window it was made against.
    ///
    /// Also called by [`App::suspended`], Android taking the surface back whenever the app leaves
    /// the screen. [`App::resumed`] then rebuilds by the same path a first start takes, so the
    /// layout is fresh.
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
    /// Says where the camera was left, in the syntax of the literal in [`App::new`]: the opening
    /// angle is picked by turning the graph and copying this line out of the log. The layout is
    /// the same every run -- see `scatter` -- but only within the dimensions it was read in.
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
            // framed onto the tree in [`App::build`]. Picked by hand off a settled 3D layout.
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
            dump: Dump::Loading(fetch::spawn(world::load(false))),
            // For the same reason as the dump: a run that left a session behind has an account to
            // read before it has a window to draw it in.
            yno: yno::Account::new(),
            code: world::Code::default(),
            revealed: false,
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
            opening_on: link::location(),
            said: String::new(),
            veil: 1.0,
            wanted: Wanted::Now,
            requested: false,
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
        self.requested = false;
        self.request_a_frame();
    }

    /// Builds the graph, and does nothing until there is a dump to build it out of: [`App::draw`]
    /// calls this again once one has landed.
    ///
    /// On a phone it is also called by every [`App::reset`], the entities all being built on a
    /// graphics context the framework takes back whenever the app leaves the screen.
    fn build(&mut self) {
        let Dump::Ready(dump) = &self.dump else {
            return;
        };
        // Building on the dump alone would put the whole game on screen and take it away a moment
        // later. See `yno::Account::settled`.
        if !self.yno.settled() {
            return;
        }
        // The frontier is a different graph rather than a different drawing of one: it takes
        // worlds out, and a connection names the world it leads to by index. Applied here so
        // nothing downstream has to know.
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
        // A graph solved on the plane has to be looked at square on, or it is seen edge on. The
        // switch does this when thrown; a run that opens in two dimensions because an earlier run
        // left it there never passes through the switch.
        //
        // Only on a graph that is opening: one carrying another on keeps the camera it was given.
        if opening && data.graph.parameters().dimensions == Dimensions::Two {
            self.statics.face_plane();
        }

        // A link's world is lit the way picking it out of the search box would light it. A name
        // this graph has not got is not an error: a stale link opens on the tree.
        if let Some(name) = self.opening_on.take()
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

/// What [`profile`] is where it is not compiled: the same surface, doing nothing, so no caller
/// branches on the build.
#[cfg(not(feature = "profile"))]
mod profile {
    use three_d::Context;

    pub(super) fn eager() -> bool {
        false
    }

    pub(super) fn unlocked() -> bool {
        false
    }

    pub(super) fn pan_aside(
        _statics: &mut super::camera::AppStatics,
        _data: &super::AppEntities,
    ) -> bool {
        false
    }

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

/// What [`update`] is on the two targets that do not install packages: the page reloads and the
/// apk goes through the store, so neither has anything to replace.
#[cfg(any(target_family = "wasm", target_os = "android"))]
mod update {
    pub(super) fn controls(_ui: &mut egui::Ui) {}
}
