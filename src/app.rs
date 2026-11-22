//! The winit application: window, GL context and per-frame state. See [`App::draw`] in [`frame`]
//! for the main loop, [`loading`] for the loading screen, and [`pacing`] for redraw scheduling.

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
mod instanced;
mod japanese;
mod layout;
mod link;
mod loading;
mod map;
mod pacing;
mod prelude;
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
use instanced::RefilledMesh;
use layout::*;
use loading::*;
use pacing::*;
use prelude::*;
use profile::FrameStats;
use scene::*;
use ui::*;
use world::{Conditions, Gate};

use crate::i18n::speaking_japanese;

/// The `AndroidApp` from `android_main`, needed later by [`thumbnails`] (assets) and [`Overlay`]
/// (safe area, soft keyboard).
#[cfg(target_os = "android")]
static ANDROID: std::sync::OnceLock<winit::platform::android::activity::AndroidApp> =
    std::sync::OnceLock::new();

#[cfg(target_os = "android")]
pub(crate) fn use_android_app(app: winit::platform::android::activity::AndroidApp) {
    let _ = ANDROID.set(app);
}

/// MSAA sample count when antialiasing is on; by far the largest per-frame GPU cost (measured).
/// Not user-configurable: WebGL only takes on/off, and the browser picks the count.
const MULTISAMPLES: u8 = 4;

/// Whether MSAA is on; read only when the surface is created. MSAA over post-process AA: the
/// graph is thin high-contrast lines, and post-process AA measured no cheaper on desktop and slower
/// on the web build.
fn load_antialias_setting() -> bool {
    store::read_switch(ANTIALIAS).unwrap_or(true)
}

fn save_antialias_setting(on: bool) {
    store::write_switch(ANTIALIAS, on);
}

/// Whether the camera follows the world under a hovered list row. Defaults to off when the user
/// prefers reduced motion. See [`AppStatics::follow_hovered_row`].
fn load_row_follow_setting() -> bool {
    store::read_switch(ROW_FOLLOW).unwrap_or_else(|| !reduced_motion())
}

fn save_row_follow_setting(on: bool) {
    store::write_switch(ROW_FOLLOW, on);
}

/// The `prefers-reduced-motion` media query. Web only: winit does not expose the OS setting, so
/// native builds return false.
fn reduced_motion() -> bool {
    #[cfg(target_family = "wasm")]
    {
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
    /// Needs the GL context, so `None` until the window exists.
    overlay: Option<Overlay>,
    /// Source data for [`AppEntities`]. See [`Dump`].
    dump: Dump,
    /// The YNO account; its progress filters the graph while the frontier switch is on. See
    /// [`App::build`].
    yno: yno::Account,
    /// Progress through the secret key code. See [`world::Code`].
    code: world::Code,
    /// Whether the code was entered; later dump fetches, retries included, keep its worlds.
    revealed: bool,
    /// The previous graph, kept only between teardown and the rebuild that animates from it. See
    /// [`Before`].
    before: Option<Before>,
    building: Building,
    /// Selection saved across an Android suspend, which drops [`AppEntities`] (see
    /// [`App::release`]). Restored on rebuild, and the camera re-frames it.
    selected: Option<Highlight>,
    /// World named in the URL; the first [`App::build`] turns it into a selection.
    opening_on: Option<String>,
    /// Last loading-screen message, kept so it does not change during the fade-out. See
    /// [`App::veil`].
    said: String,
    /// Loading-screen opacity, 1 to 0. Reset to 1 by every [`App::draw_loading`].
    veil: f32,
    /// When the next frame is due; the event loop turns it into a wait. See [`Wanted`].
    wanted: Wanted,
    frame_requested: bool,
    still_seconds: f32,
    stats: FrameStats,
}

struct AppEntities {
    graph: Graph,
    /// Here, not in [`AppStatics`], so a rebuild drops it with the node index it refers to.
    gesture: Option<Gesture>,
    /// Kept so a layout restart continues the same random sequence.
    rng: Rng,
    routes: world::Routes,
    /// Indexed by node; UI only. See [`world::Title`].
    titles: Vec<world::Title>,
    /// Per world, in wiki order; empty when the wiki has no map. UI only.
    maps: Vec<Vec<world::Map>>,
    /// Most worlds first; indexed by [`Highlight::Author`]. See [`world::Dump::authors`].
    authors: Vec<world::Author>,
    author_of: Vec<usize>,
    /// Game versions, newest first, with the worlds each added. Indexed by
    /// [`Highlight::Version`].
    versions: Vec<world::Version>,
    /// The current highlight; everything else is dimmed.
    selected: Option<Highlight>,
    /// Distinct paths for [`Highlight::Path`], least restricted first. Empty otherwise or when
    /// no path exists.
    ways: Vec<world::Way>,
    /// Index of the displayed path in `ways`. See [`AppEntities::take_way`].
    way: usize,
    /// Where the Eyeball Bomb effect teleports the player; used by pathfinding. See
    /// [`world::hub_world`].
    hub: Option<usize>,
    /// Maximum depth reachable from the origin; the depth stepper's upper bound.
    deepest: u32,
    /// Right-button press position while held. Right drag pans and right click opens the menu;
    /// drag distance tells them apart.
    right_press: Option<PhysicalPoint>,
    /// Open context menu; the UI closes it when an entry is chosen.
    menu: Option<ContextMenu>,
    /// Unpressed cursor position in physical pixels, and the node hit-tested under it. `None`
    /// outside the window, over the panel, and always on touch screens.
    cursor: Option<PhysicalPoint>,
    hover: Option<usize>,
    /// World of the hovered list row, highlighted in the graph. See `Panel::hovered_row`.
    hovered_row: Option<usize>,
    /// The world row-follow moves toward; kept after the hover ends, cleared when the user moves
    /// the camera. See [`AppStatics::follow_hovered_row`].
    row_follow_target: Option<usize>,
    /// Per world, its connections and their directions. Feeds the edge meshes and the panel.
    connections: Vec<Vec<world::Step>>,
    /// Node degree in the drawn graph. UI only.
    degrees: Vec<usize>,
    /// Up to `UNTAKEN_WORLDS` worlds with the most unexplored exits, most first. Empty without a
    /// frontier filter, which hides the list. See [`Highlight::Untaken`].
    untaken: Vec<usize>,
    /// World the camera frames at startup, with its subtree; `None` if the dump lacks it, which
    /// keeps the default camera pose. See [`opening_room`].
    opening: Option<usize>,
    /// Whether the camera is moving to frame the selection; cleared on arrival or user input.
    framing: bool,
    /// Frame the whole route instead of its last world. Persists across selections.
    frame_route: bool,
    /// Connection colors before a selection dims them, so clearing one restores the distance ramp.
    edge_colors: Vec<Srgba>,
    /// Per node, the half-height of its quad, from how much of the graph hangs off it.
    node_radii: Vec<f32>,
    arrivals: Arrivals,
    /// UI setting: how much subtree size adds to a node's repulsion. See [`node_mass`].
    hub_repulsion: f32,
    /// UI setting: maximum edge length in layers. The simulation only stores it converted to
    /// distance. See `LINK_REACH_DEFAULT`.
    link_reach: f32,
    descendant_counts: Vec<u32>,
    /// Full-screen background, drawn first via [`apply_screen_material`]; a material, not a [`Gm`].
    backdrop: ColorMaterial,
    /// World thumbnails on billboard quads. Drawn once the atlas has loaded.
    thumbnails: Gm<RefilledMesh, ColorMaterial>,
    /// Additive copy of the hovered row's thumbnail, drawn over its node while
    /// [`AppEntities::hovered_row`] is set. See [`AppEntities::aim_glow`].
    glow: Gm<Mesh, ColorMaterial>,
    edges: Gm<RefilledMesh, ColorMaterial>,
    /// One-way edges, drawn as animated dashes. See [`AppEntities::march_dashes`].
    dashes: Gm<RefilledMesh, DashMaterial>,
    /// CPU-side instances; source for [`AppEntities::copy_highlighted_instances`].
    thumbnail_instances: Instances,
    /// Two-way edges only; one-way edges are in `dash_instances`. Both are in graph edge order, so
    /// an edge's slot is stable.
    edge_instances: Instances,
    dash_instances: Instances,
    /// Highlighted instances, redrawn on top of the scene so nothing occludes them. See
    /// [`AppEntities::copy_highlighted_instances`].
    highlight_thumbnails: Gm<RefilledMesh, ColorMaterial>,
    highlight_edges: Gm<RefilledMesh, ColorMaterial>,
    /// Separate from `highlight_edges` so highlighted one-way edges stay dashed.
    highlight_dashes: Gm<RefilledMesh, DashMaterial>,
    /// Compacted copies of the highlighted slots, not indexed by world.
    highlight_thumbnail_instances: Instances,
    highlight_edge_instances: Instances,
    highlight_dash_instances: Instances,
    /// Highlighted slot indices, set by [`AppEntities::repaint`].
    highlight_nodes: Vec<usize>,
    highlight_lines: Vec<usize>,
    highlight_dashed: Vec<usize>,
    /// Set when [`AppEntities::repaint`] changed a color, which forces a redraw of a settled graph.
    recolored: bool,
    /// World the glow quad was last placed on by [`AppEntities::aim_glow`]; detects hover changes.
    glowing: Option<usize>,
    /// Thumbnail atlas loading state. See [`Atlas`].
    atlas: Atlas,
    /// The atlas as the versions tab uses it; `None` until loaded, or if loading failed.
    sheet: Option<thumbnails::Sheet>,
    /// Atlas cell per world; `None` for unvisited worlds, which use the placeholder. See
    /// [`thumbnails::cells`].
    cells: Vec<Option<usize>>,
    /// Worlds drawn with the placeholder image. Empty without a frontier filter. See
    /// [`AppEntities::place_unvisited`].
    unvisited: Vec<usize>,
    /// Their quads, offset toward the camera. Reused across frames to avoid reallocating.
    unvisited_quads: Instances,
    highlight_unvisited_quads: Instances,
    detail: detail::Detail,
    /// Dash animation offset in world units; wrapped so it keeps f32 precision.
    dash_phase: f32,
    /// Billboard rotation at the last rebuild; a camera rotation triggers a rebuild.
    billboard: Mat4,
}
#[derive(Default)]
struct AppContext {
    window: Option<Window>,
    wctx: Option<WindowedContext>,
    fig: Option<FrameInputGenerator>,
}

/// Settings key. See [`load_antialias_setting`].
const ANTIALIAS: &str = "antialias";
/// Settings key for row-follow; the old name is kept so saved settings survive.
const ROW_FOLLOW: &str = "leaning";
impl ApplicationHandler for App {
    /// Turns a timed wake-up into a one-shot. winit re-arms an expired `WaitUntil`, which would
    /// busy-loop until the next frame, and a hidden tab gets no animation frames.
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
    /// Android only. See [`App::release`].
    fn suspended(&mut self, _: &winit::event_loop::ActiveEventLoop) {
        self.release();
    }
    /// The last safe point to drop GL resources and the clipboard: afterwards the GL context is
    /// gone, and dropping the clipboard against a freed Wayland display crashes.
    fn exiting(&mut self, _: &winit::event_loop::ActiveEventLoop) {
        self.log_camera_pose();
        self.release();
    }
    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // Events still arrive between suspend and resume.
        let Some(fig) = self.ctx.fig.as_mut() else {
            return;
        };
        // See [`App::request_a_frame`].
        let wants_a_frame = !matches!(event, WindowEvent::RedrawRequested);
        fig.handle_winit_window_event(&event);
        // Whether egui consumed the event is decided in `Overlay::run`.
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
                self.frame_requested = false;
                self.stats.began(self.ctx.wctx.as_ref().unwrap());
                let began = web_time::Instant::now();
                let rebuilt = self.draw();
                // Before the swap, which waits for the display.
                self.stats.frame(began.elapsed(), rebuilt);
                // After drawing, so the page is never blank between placeholder and canvas.
                #[cfg(target_family = "wasm")]
                take_placeholder();
                self.ctx.wctx.as_ref().unwrap().swap_buffers().unwrap();
                self.pace(event_loop);
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            // A key held while focus is lost never gets a release event.
            WindowEvent::Focused(false) => {
                self.statics.walk = Walk::default();
                self.statics.focused = false;
            }
            WindowEvent::Focused(true) => self.statics.focused = true,
            WindowEvent::Touch(touch) => {
                // A second finger turns the gesture into a pinch.
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
    /// Drops everything built on the window, innermost first: buffers before the GL context,
    /// the context before the window. On Android, [`App::resumed`] rebuilds from scratch.
    fn release(&mut self) {
        // See [`App::selected`].
        self.selected = self.data.as_ref().and_then(|data| data.selected);
        self.data = None;
        // Dropping `gui::Gui` without `destroy` leaks GL buffers.
        if let Some(overlay) = self.overlay.as_mut() {
            overlay.gui.destroy();
        }
        self.overlay = None;
        self.ctx.fig = None;
        self.ctx.wctx = None;
        self.ctx.window = None;
    }
    /// Logs the camera pose as the `vec3` literals in [`App::new`], for picking the startup view.
    /// The layout is deterministic (see `scatter`), but only per dimension count.
    fn log_camera_pose(&self) {
        let (eye, at) = (self.statics.camera.position(), self.statics.camera.target());
        let dimensions = self
            .data
            .as_ref()
            .map_or(Dimensions::Three, AppEntities::dimensions);
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
            // Direction only, picked off a settled 3D layout; [`App::build`] frames the rest.
            vec3(-49.7, -47.1, -31.0),
            vec3(218.5, -173.5, 180.6),
            vec3(0.0, 1.0, 0.0),
            degrees(FOV_Y_DEGREES),
            0.1,
            1600.0,
        );
        // Read before the control moves into place.
        let centre = camera.target();
        let control = OrbitControl::new(centre, 1.0, 800.0);
        // Fire and forget: the task stores its result where the app reads it.
        drop(fetch::spawn(world::load_pages()));

        Self {
            ctx: AppContext::default(),
            // Start now: the fetch is the longest wait and does not need the window.
            dump: Dump::Loading(fetch::spawn(world::load(false))),
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
                row_follow_aim: centre,
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
            frame_requested: false,
            still_seconds: 0.0,
            stats: FrameStats::new(),
        }
    }
    fn reset(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, window: Window) {
        self.ctx.window = Some(window);
        let window = self.ctx.window.as_ref().unwrap();
        let surface = SurfaceSettings {
            vsync: !profile::unlocked(),
            // One read for both surface and overlay. See [`Overlay::antialias_running`].
            multisamples: match load_antialias_setting() {
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
        // The window draws on demand; this is the first.
        self.frame_requested = false;
        self.request_a_frame();
    }

    /// Builds the graph once the dump has loaded; [`App::draw`] retries until then. On Android,
    /// every [`App::reset`] calls it, since a suspend drops the GL context.
    fn build(&mut self) {
        let Dump::Ready(dump) = &self.dump else {
            return;
        };
        // Wait for the YNO account, or the unfiltered graph flashes. See `yno::Account::settled`.
        if !self.yno.settled() {
            return;
        }
        // Filtering to the frontier removes worlds and renumbers the connections.
        let frontier;
        let dump = match self.yno.frontier() {
            Some(visited) => {
                frontier = dump.showing(&visited);
                &frontier
            }
            None => dump,
        };
        let ctx = self.ctx.wctx.as_ref().unwrap();
        // On the first build the user has not touched the camera yet.
        let opening = self.before.is_none();
        let mut data = entities(dump, self.before.take().as_ref(), ctx);
        // Starting in 2D skips the toggle that would face the camera at the plane.
        if opening && data.dimensions() == Dimensions::Two {
            self.statics.face_plane();
        }

        // An unknown world in the URL falls back to the default view.
        if let Some(name) = self.opening_on.take()
            && self.selected.is_none()
        {
            self.selected = data.world_named(&name).map(Highlight::Route);
            if self.selected.is_none() {
                log::warn!("no world is named {name}");
            }
        }
        // Reselect in the new layout; the old camera aim no longer applies.
        if self.selected.is_some() {
            data.select(self.selected);
        }
        // Snap: there is no previous view to animate from.
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

/// No-op [`profile`] when the feature is off, so callers need no `cfg`.
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

/// No-op [`update`] on web and Android, which update through a reload and the app store.
#[cfg(any(target_family = "wasm", target_os = "android"))]
mod update {
    pub(super) fn controls(_ui: &mut egui::Ui) {}
}
