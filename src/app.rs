//! See [App::draw] for the main flow.

use egui_material_icons::icons::*;
use force_graph_3d::{DefaultNodeIdx, Dimensions, ForceGraph, NodeData};
use three_d::{FrameInput, FrameInputGenerator, GUI, WindowedContext, egui, renderer::*};
use winit::{
    application::ApplicationHandler,
    event::{Touch, TouchPhase, WindowEvent},
    window::{CursorIcon, Window},
};

/// Side of the cube nodes are scattered over at the start of a layout, in simulation units.
const SPAWN_EXTENT: f32 = 1000.0;
/// World units per simulation unit. [`world_pos`] multiplies a layout by it. [`sim_pos`] divides
/// by it to return a point to simulation space.
///
/// Simulation coordinates are pixel-sized. A graph a thousand of them across has to stand in
/// front of a camera that measures in tens. This factor is what puts it there.
///
/// The name is about the space, not about the thing in it. Everywhere else in this file a
/// "world" is one of the game's worlds, which is a node. [`NODE_BASE_RADIUS`] sizes those.
const SIM_TO_WORLD: f32 = 0.012;
/// The size of a world with nothing behind it, which is the floor every other node is measured up
/// from. See [`NODE_HUB_GROWTH`], which moves against it: the two together set the range rather
/// than either alone.
const NODE_BASE_RADIUS: f32 = 0.2;
/// Mass of a world of [`NODE_BASE_RADIUS`]. [`node_mass`] scales every bigger world from it.
/// The value is the library's own default, restated here because the scale starts from it.
const NODE_BASE_MASS: f32 = 10.0;
/// Where the panel's hub-push knob starts, and so the value [`AppEntities::hub_repulsion`] holds
/// until somebody drags it. It decides how much of a world's size settles how hard the world
/// pushes. See [`node_mass`].
///
/// At `0.0` every world pushes alike. At `1.0` the push is proportional to the size. The higher
/// value spaces the hubs generously, and the rest of the layout with them.
const HUB_REPULSION_DEFAULT: f32 = 0.25;

/// The anchor the node sizes are drawn through. A world with [`NODE_HUB_DESCENDANTS`] worlds
/// behind it is [`NODE_HUB_GROWTH`] times the size of a world with nothing behind it. Every other
/// size follows from those two on a logarithmic curve. The curve keeps the ranks a player can act
/// on apart, and the origin, with the whole game behind it, still fits on the same scale.
///
/// The growth fell as the floor rose, so lifting the smallest nodes clear of the vanishing point
/// left the largest where they were. A range this wide is already as much as the layout can hold
/// without the hubs eating their neighbours.
const NODE_HUB_DESCENDANTS: f32 = 4.0;
const NODE_HUB_GROWTH: f32 = 3.3;
const EDGE_RADIUS: f32 = 0.02;
/// Spacing between depth layers in layered mode, in simulation units.
///
/// Measured against how wide a layer actually settles, rather than picked for looks. The busiest
/// layers hold a few hundred worlds and spread to a radius near 7000. Anything much smaller than
/// this stacks sixteen layers into a pancake, which reads as one thickened cloud, not as layers.
const DAG_LEVEL_DISTANCE: f32 = 800.0;
/// The same spacing in two dimensions, where a layer is a line rather than a plane.
///
/// Wider than [`DAG_LEVEL_DISTANCE`]. A flat layer carries the whole crowd on one line, and it
/// may also stack either side of that line. The gap has to hold the stack and still read as a gap.
///
/// Not much wider, though. The gaps multiply over sixteen layers, and a tree taller than it is
/// wide takes scrolling to read rather than one glance.
const DAG_LEVEL_DISTANCE_2D: f32 = 1000.0;
/// How far a world may sit from its layer per microstep of slack, in simulation units.
///
/// About two nodes across, which is what makes a microstep worth taking. A step of one node
/// leaves the worlds it separates touching, and that reads as one clump rather than as a stack.
const DAG_LEVEL_MICROSTEP: f32 = 60.0;
/// How many [`DAG_LEVEL_MICROSTEP`]s of slack a layer has in two dimensions. This constant is a
/// count. The microstep it multiplies is a distance. [`Overlay::run`] uses only their product.
///
/// A layer in three dimensions is a plane, and it spreads a crowd across that plane. In two
/// dimensions a layer is a line. A busy line has nowhere for the overflow but on top of itself,
/// so it may stack this far either side. Keep the count a fraction of [`DAG_LEVEL_DISTANCE_2D`],
/// or the layers meet and stop reading as layers.
const DAG_LEVEL_SLACK_MICROSTEPS_2D: f32 = 6.0;
/// Force per unit of distance between a grabbed node and the cursor.
///
/// The cursor pulls the node rather than places it, so the graph attached to the node travels
/// with it. Momentum makes the response second order. The gain has to stay well under the value
/// that would close the gap in a single frame.
const GRAB_STIFFNESS: f32 = 0.5;
/// Ceiling on that force. A grabbed node and a cursor flung across the window cannot throw the
/// node clear of the layout.
const GRAB_FORCE_MAX: f32 = 2000.0;
/// How far from a node a press may land and still count as over it, in physical pixels. It
/// applies to a node drawn smaller than that.
///
/// The tolerance is measured on screen rather than in the world. A world-space radius is a
/// cylinder through the whole depth of the layout, and in a cloud this dense it catches a node
/// under nearly every press.
///
/// A press on a node drawn wider than this has to land inside the node's own drawn edge. Without
/// that, a close enough zoom would leave a plate that fills the window pickable only near its
/// centre.
const GRAB_TOLERANCE_PIXELS: f32 = 14.0;
/// How far the cursor may travel between press and release and still count as a click, in
/// physical pixels.
const GESTURE_SLOP_PIXELS: f32 = 6.0;
/// How many worlds the panel names out of a highlighted subtree.
const NOTABLE_WORLDS: usize = 10;
/// A world with at least this many connections counts as a "notable" descendant.
const NOTABLE_HUB_CONNECTIONS: usize = 3;
/// Vertical field of view. The camera uses it, and so does the pan, which converts cursor pixels
/// into world units through it.
const FOV_Y_DEGREES: f32 = 45.0;
/// The color the frame clears to. [`panorama_texture`] also lays the backdrop's glows over it.
/// Both readings want sRGB, so the value is sRGB.
const BACKGROUND_COLOR: [f32; 3] = [0.03, 0.03, 0.05];
/// Side of the backdrop's repeating tile in texels, and the spacing of the lattice of glows
/// inside it. Four glows lie across the tile, each with its own brightness, so the lattice does
/// not read as one cell stamped over and over.
///
/// [`PANORAMA_TILE_PIXELS`] gives the size on screen, which is a separate matter. The tile
/// stretches to that size whatever number of texels it holds.
const PANORAMA_TILE_TEXELS: usize = 256;
const PANORAMA_CELL_TEXELS: usize = 64;
/// How far a glow reaches, in texels, and how sharply it fades over that reach. The reach is
/// most of a cell, so neighbouring glows almost meet.
const PANORAMA_GLOW_RADIUS_TEXELS: f32 = 52.0;
const PANORAMA_GLOW_FALLOFF: f32 = 2.2;
/// Which norm measures the distance to a glow's centre. It sits between the diamond a norm of 1
/// draws and the circle a norm of 2 draws. That is the rounded-square shape of the reference
/// backdrop.
const PANORAMA_GLOW_NORM: f32 = 1.5;
/// Peak color of a glow, at the brightest cell. It is blue and dim on purpose. It has to stay
/// visible past a graph whose own edges are only a little brighter, and it must not compete
/// with them.
const PANORAMA_GLOW_COLOR: [f32; 3] = [3.0 / 255.0, 6.0 / 255.0, 42.0 / 255.0];
/// Peak brightness per cell of the tile, as a fraction of [`PANORAMA_GLOW_COLOR`]. The values are
/// irregular. They belong to the tile rather than to the screen, so the pattern repeats
/// seamlessly.
const PANORAMA_CELL_PEAKS: [[f32; 4]; 4] = [
    [0.55, 0.30, 0.85, 0.40],
    [0.35, 1.00, 0.45, 0.65],
    [0.90, 0.50, 0.70, 0.30],
    [0.40, 0.75, 0.35, 0.95],
];
/// Side of the tile on screen, in logical pixels. It is independent of the texel size, so the
/// backdrop keeps its scale on a high-density display rather than shrinking to half of it.
const PANORAMA_TILE_PIXELS: f32 = 260.0;
/// How far the backdrop slides, in tiles, per unit of the view direction's horizontal or vertical
/// component. A quarter turn of the camera moves it by this much.
///
/// Small on purpose. A panorama sits far enough away that a turn toward it barely shifts it. Only
/// that faint drift is wanted, so that the graph reads as turning in front of a scene rather than
/// against a decal.
const PANORAMA_PARALLAX: f32 = 0.12;
/// Brightness of the connection into a world with nothing behind it, against the one into the
/// world the whole game is behind. See [`edge_colors`]. The connections carry the depth ramp, so
/// this value is the range the ramp is drawn over rather than a color of their own.
const EDGE_LEAF_BRIGHTNESS: f32 = 0.55;
/// How far the connection colors move toward even apparent brightness. At 0 the ramp keeps its
/// own stops. At 1 every stop takes the brightness of the dimmest.
///
/// The ramp is picked for hue, and hue drags brightness with it. Most of what the eye reads as
/// brightness is the green channel. So the green stop in the middle of the ramp looks glaring
/// beside the magenta at the end, even though both are nominally full.
///
/// This value levels them part of the way. Far enough that no stop shouts over the others, and
/// not so far that the ramp flattens into four shades of one brightness and stops reading as
/// depth.
const EDGE_LUMINANCE_EVENNESS: f32 = 0.5;
/// Warm and bright, for the edges of the route back to the origin world.
const ROUTE_COLOR: Srgba = Srgba::new(255, 242, 194, 255);
/// Brightness left to everything off the highlighted route. Low enough to set the surrounding
/// graph behind the route, high enough that it stays readable as context.
const DIMMED_BRIGHTNESS: f32 = 0.25;
/// Color for the worlds the origin cannot reach. It is deliberately off the distance ramp. Those
/// worlds are at no distance at all rather than at a large one, and reading them as the far end
/// of the ramp would be a lie.
const UNREACHED_COLOR: Srgba = Srgba::new(110, 110, 120, 255);
/// Time constant of the frame-rate smoothing, in milliseconds. The per-frame number swings far too
/// much to read from a moving graph.
const FPS_WINDOW_MS: f32 = 500.0;
/// How many worlds the search box offers at once.
const SEARCH_CANDIDATES: usize = 10;
/// How many of a release's worlds the catalog shows it by, and how tall it draws those pictures,
/// in egui's points.
const CATALOG_THUMBNAILS: usize = 4;
const CATALOG_THUMBNAIL_HEIGHT: f32 = 34.0;
/// Time constant of the camera's ease onto a selected route, in milliseconds. Long enough that the
/// move reads as travel across the graph rather than as a cut. That is the whole point: the
/// person has to keep their bearings while the view changes.
const FRAMING_WINDOW_MS: f32 = 600.0;
/// Slack left around a framed route, as a fraction of the distance that would touch it to the
/// window edges.
const FRAMING_MARGIN: f32 = 1.2;
/// Smallest sphere the camera will frame, in world units. A route of one world has no extent at
/// all, and without this the camera would fly into the node.
const FRAMING_MIN_RADIUS: f32 = 2.0;

/// How near the goal the camera has to be to count as arrived, as a fraction of the framed radius
/// and of the framing distance. The ease is asymptotic, so it needs a floor to stop at.
const FRAMING_ARRIVAL_TOLERANCE: f32 = 0.01;

mod detail;
mod fetch;
mod guide;
mod thumbnails;
mod world;

/// The handle the activity glue passed to `android_main`, which is the only way to reach anything
/// the framework owns and is handed out exactly once, before any of this runs. Kept here because
/// two things need it well after that: the assets in [`thumbnails`], and the window's safe area
/// and soft keyboard in [`Overlay`].
#[cfg(target_os = "android")]
static ANDROID: std::sync::OnceLock<winit::platform::android::activity::AndroidApp> =
    std::sync::OnceLock::new();

/// Lends that handle to everything below, which all runs too late to be given it directly.
#[cfg(target_os = "android")]
pub(crate) fn use_android_app(app: winit::platform::android::activity::AndroidApp) {
    let _ = ANDROID.set(app);
}

pub(super) struct App {
    ctx: AppContext,
    data: Option<AppEntities>,
    statics: AppStatics,
    /// Built with the graphics context, so it cannot exist before the window does.
    overlay: Option<Overlay>,
}

#[derive(Default)]
struct AppContext {
    window: Option<Window>,
    wctx: Option<WindowedContext>,
    fig: Option<FrameInputGenerator>,
}

struct AppStatics {
    control: OrbitControl,
    camera: Camera,
    /// Whether a pan drag is under way. Raised once the drag actually moves the camera rather
    /// than on the press, so a right-click that only opens a menu is not read as a pan.
    panning: bool,
    /// The pointer the window was last given, so it is only set when it changes.
    cursor: CursorIcon,
    /// The fingers on the screen, on a machine that has any. See [`Touches`].
    touches: Touches,
    /// The WASD keys being held. See [`Walk`].
    walk: Walk,
}

/// The WASD keys, held down.
///
/// A key says only that it went down or came up, and walking has to carry on between the two, so
/// what is held is kept here and read once a frame as the distance it stands for.
#[derive(Default)]
struct Walk {
    left: bool,
    right: bool,
    forward: bool,
    back: bool,
}

impl Walk {
    /// Follows the keys down and up. Emptied while egui has the keyboard, so typing a world's
    /// name into the search box does not also walk the view across the graph.
    fn track(&mut self, events: &[Event], typing: bool) {
        if typing {
            *self = Self::default();
            return;
        }
        for event in events {
            let (key, down) = match event {
                Event::KeyPress { kind, .. } => (kind, true),
                Event::KeyRelease { kind, .. } => (kind, false),
                _ => continue,
            };
            match key {
                Key::W => self.forward = down,
                Key::A => self.left = down,
                Key::S => self.back = down,
                Key::D => self.right = down,
                _ => (),
            }
        }
    }

    /// How far the held keys walk over `dt` seconds: across the view, and into it. Both are
    /// given in logical pixels, so that [`AppStatics::pan_by`] and [`AppStatics::dolly_by`] can
    /// scale them the same way and a step sideways covers as much ground as a step forward.
    ///
    /// Across is signed as a drag rather than as a move, because that is what a pan is given:
    /// pushing the view right is pulling the graph left.
    fn travel(&self, dt: f32) -> (f32, f32) {
        let (across, into) = (
            (self.left as i32 - self.right as i32) as f32,
            (self.forward as i32 - self.back as i32) as f32,
        );
        // Normalized, so two keys held at once carry the view as far as one does rather than
        // over the diagonal of it.
        let step = WALK_SPEED * dt / across.hypot(into).max(1.0);
        (across * step, into * step)
    }
}

/// The fingers on the screen, which the mouse events the app is otherwise driven by cannot say
/// enough about.
///
/// three-d turns a touch into a mouse: the first finger presses, drags and releases the left
/// button, and a second one turns the pair into a wheel so that a pinch zooms. What it never
/// carries is that the second finger is there at all. So a pinch ends with a left release that
/// reads as a tap and throws the selection away, and the travel of the pair across the screen —
/// the only pan gesture a screen with no second button has — is dropped. Both are read here
/// instead, off the events winit delivers before three-d ever sees them.
#[derive(Default)]
struct Touches {
    /// Every finger down, by the id winit gave it, at its latest position in physical pixels.
    fingers: Vec<(u64, (f32, f32))>,
    /// Where their midpoint last was, while at least two of them are down.
    midpoint: Option<(f32, f32)>,
    /// How far that midpoint has travelled since a frame last took it, in physical pixels.
    travel: (f32, f32),
    /// Whether a second finger has landed and not every finger has left since.
    ///
    /// Latched rather than read off the count, because the fingers do not lift together: the one
    /// still down when the other leaves would otherwise carry on as an orbit, and the last to
    /// leave would release as a tap.
    pinching: bool,
}

impl Touches {
    /// Takes a finger's news, and says whether a pinch has just begun — which is the moment the
    /// gesture the single finger before it had nominated stops being any of the things it could
    /// have been.
    fn track(&mut self, touch: &Touch) -> bool {
        let at = (touch.location.x as f32, touch.location.y as f32);
        match touch.phase {
            TouchPhase::Started => self.fingers.push((touch.id, at)),
            TouchPhase::Moved => {
                if let Some(finger) = self.fingers.iter_mut().find(|(id, _)| *id == touch.id) {
                    finger.1 = at;
                }
            }
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.fingers.retain(|(id, _)| *id != touch.id)
            }
        }
        let began = self.fingers.len() > 1 && !self.pinching;
        self.pinching = (self.pinching || began) && !self.fingers.is_empty();

        // Only a move measures travel: a finger arriving or leaving moves the midpoint by half
        // the gap between the fingers, which is not a drag and would fling the camera.
        let midpoint = self.midpoint();
        if let (Some(now), Some(was), TouchPhase::Moved) = (midpoint, self.midpoint, touch.phase) {
            self.travel.0 += now.0 - was.0;
            self.travel.1 += now.1 - was.1;
        }
        self.midpoint = midpoint;
        began
    }

    /// Where the fingers' midpoint is, once there are enough of them for it to mean anything.
    fn midpoint(&self) -> Option<(f32, f32)> {
        (self.fingers.len() > 1).then(|| {
            let sum = self
                .fingers
                .iter()
                .fold((0.0, 0.0), |sum, (_, at)| (sum.0 + at.0, sum.1 + at.1));
            let count = self.fingers.len() as f32;
            (sum.0 / count, sum.1 / count)
        })
    }

    /// The travel since this was last asked, in the logical pixels a pan is measured in.
    fn take_travel(&mut self, device_pixel_ratio: f32) -> (f32, f32) {
        let travel = std::mem::take(&mut self.travel);
        (travel.0 / device_pixel_ratio, travel.1 / device_pixel_ratio)
    }
}

/// The sphere a framing move has to bring into view: where it sits and how far it reaches.
struct Bounds {
    center: Vec3,
    radius: f32,
}

/// Gap the panel and the rocker keep off the edges of the safe area. See [`safe_insets`].
const PANEL_MARGIN: i8 = 12;
/// Width of the sidebar before anyone drags its edge.
const SIDEBAR_WIDTH: f32 = 280.0;
/// How opaque the sidebar is, out of 255.
///
/// The graph runs underneath the sidebar rather than stopping at its edge. A sidebar that hid its
/// own share of the layout would make the person move the camera to read what they had just
/// selected. This value is still opaque enough to keep text legible over a bright stretch of the
/// graph.
const SIDEBAR_OPACITY: u8 = 200;
/// How fast the WASD keys walk the view, in logical pixels a second. The speed is read against
/// the window rather than against the graph, because [`AppStatics::world_per_pixel`] already
/// scales a screen distance into world units at whatever distance the camera stands.
const WALK_SPEED: f32 = 800.0;
/// Width of the menu a right-click opens, and of the tooltip a hover opens, in egui's points.
/// Each one carries a world's title, and a long title would otherwise stretch it across the
/// window.
const POPUP_WIDTH: f32 = 240.0;
/// How far below and right of the cursor the hover tooltip sits, in egui's points, so that it
/// names the world under the pointer rather than covering it.
const TOOLTIP_OFFSET: f32 = 16.0;

/// The on-screen readout: how fast the layout is drawing, and what is selected.
struct Overlay {
    gui: GUI,
    /// Exponentially smoothed frame rate, over [FPS_WINDOW_MS].
    fps: f32,
    /// What the sidebar was left showing and holding. See [`Sidebar`].
    sidebar: Sidebar,
    /// Whether the on-screen keyboard was last asked for. See [`track_keyboard`].
    keyboard: bool,
    /// The controls, named on the first run. See [`guide::Guide`].
    guide: guide::Guide,
}

/// What the sidebar keeps between frames.
///
/// Held apart from [`Panel`], which is this frame's decisions: these are the person's, and they
/// outlive the frame that made them.
#[derive(Default)]
struct Sidebar {
    /// Whether the person has folded it away, leaving the graph the whole window. Kept this way
    /// round so that the default is the sidebar open, which is where a person who has never
    /// touched the button should find it.
    closed: bool,
    tab: Tab,
    /// What has been typed into each tab's own search box. Matched against every frame rather
    /// than cached, and only for the tab that is open: a scan of a few thousand titles, or a few
    /// hundred names, is far cheaper than the frame it happens in.
    ///
    /// One box per tab rather than one shared: they search different things, and carrying a
    /// world's name over into the authors would only ever come up empty.
    worlds: String,
    authors: String,
    versions: String,
}

/// Which of the sidebar's four readings is open.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// The graph in front of the person: what is selected, and what to look at next.
    #[default]
    Graph,
    /// Who made the worlds.
    Authors,
    /// What each release added.
    Versions,
    /// The knobs, which are set once and then left alone.
    Settings,
}

/// What the panel was left at, once it is built and the frame's clicks have landed on it.
struct Panel {
    dimensions: Dimensions,
    hub_repulsion: f32,
    layered: bool,
    /// A world picked out of one of the lists, to be routed to.
    chosen: Option<usize>,
    /// What the menu, a link, or the rocker asked to light, if any of them asked at all. The
    /// inner `None` is the rocker's off position, which asks for nothing to be lit.
    lit: Option<Option<Highlight>>,
    /// Whether the menu was acted on and should close.
    menu_taken: bool,
    /// Whether egui is working a widget and the frame's pointer events are the panel's.
    wants_pointer: bool,
    /// Whether the settings tab asked for the controls to be named again.
    guide: bool,
}

/// Everything the panel reads, walked out of [AppEntities] before it is built.
struct PanelData<'a> {
    data: &'a AppEntities,
    fps: f32,
    /// The route home from what is lit, origin last.
    route: Vec<usize>,
    /// The worlds worth naming among the descendants of what is lit.
    notable: Vec<usize>,
    /// The worlds a highlight names as a plain list: an author's work, a release's additions, a
    /// whole layer. Empty for the two highlights the panel reads some other way.
    listed: Vec<usize>,
    /// What the worlds search box matches.
    candidates: Vec<usize>,
    /// What the open tab's box matches, as indices into [`AppEntities::authors`] or
    /// [`AppEntities::versions`]. Empty for the tab that is not open, and the whole list, in the
    /// order it is kept in, for a box with nothing in it.
    authors: Vec<usize>,
    versions: Vec<usize>,
    selected: Option<Highlight>,
    /// The world a right-click opened a menu over, and where to draw it in egui's points.
    menu: Option<(usize, egui::Pos2)>,
    /// The world the pointer is over, and where the pointer is in egui's points. `None` whenever
    /// nothing is hovered, and on a touch screen throughout. See [`AppEntities::track_gesture`].
    hovered: Option<(usize, egui::Pos2)>,
}

/// What is lit, and which reading of it: the ways the graph can be cut down to a part worth
/// looking at.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Highlight {
    /// The way to the world: its canonical route back to the origin. A plain click.
    Route(usize),
    /// What the world is the way to: everything whose route home passes through it. Asked for
    /// from the menu a right-click opens.
    Descendants(usize),
    /// Who made it: every world an author is credited with, indexed into
    /// [`AppEntities::authors`]. Asked for by clicking an author's name, in the panel or in the
    /// catalog.
    Author(usize),
    /// What a release added, indexed into [`AppEntities::versions`]. Asked for from the catalog.
    Version(usize),
    /// A whole depth at once: every world exactly this many connections from the origin. Asked
    /// for by the rocker in the bottom corner. See [`Panel::rocker`].
    Layer(u32),
}

impl Highlight {
    /// The one world the highlight is about. `None` for the three that are about a set of
    /// worlds rather than about a place in the graph: they have no world to name, and no route
    /// home of their own for the panel to walk.
    fn world(self) -> Option<usize> {
        match self {
            Self::Route(world) | Self::Descendants(world) => Some(world),
            Self::Author(_) | Self::Version(_) | Self::Layer(_) => None,
        }
    }
}

/// The menu a right-click opened over a world, and where it opened.
struct ContextMenu {
    world: usize,
    /// Where the click landed, in physical pixels, which is where the menu is drawn.
    at: PhysicalPoint,
}

/// A node being dragged by the cursor.
struct Grab {
    node: DefaultNodeIdx,
    /// Distance from the camera to the node when it was grabbed. Dragging slides the node across
    /// the plane it was picked on; without this the node would be pulled toward the camera.
    depth: f32,
    /// Cursor position in physical pixels.
    cursor: PhysicalPoint,
}

/// The left-button gesture in progress.
///
/// Three readings compete for the same button — click a node, drag a node, orbit the camera — and
/// a press alone does not say which. So a press only nominates: the gesture stays [Gesture::Held]
/// until the cursor either travels far enough to be a drag or lets go without doing so, and until
/// then nothing downstream sees the motion.
enum Gesture {
    /// Pressed, and still ambiguous. Carries what the press landed on, so a release can select it
    /// and a drag can start moving it without picking again.
    Held {
        hit: Option<Grab>,
        /// Where the press landed, in physical pixels, to measure travel against.
        origin: PhysicalPoint,
    },
    /// Awarded to the node the press landed on.
    Moving(Grab),
    /// Awarded to the camera: the motion is left unhandled for [OrbitControl].
    Orbiting,
}

struct AppEntities {
    graph: ForceGraph,
    /// Held here rather than beside the camera, so that rebuilding the graph drops the gesture
    /// along with the node index it names.
    gesture: Option<Gesture>,
    /// Kept so a restart can carry on drawing from the same sequence.
    rng: Rng,
    /// The canonical route back to the origin world, per world. See [`world::canonical_routes`].
    routes: world::Routes,
    /// World names, indexed like the nodes. Only the overlay reads them.
    titles: Vec<String>,
    /// Everyone credited, and everything each of them made. Busiest first: see
    /// [`world::Dump::authors`], which also settles what [`Highlight::Author`] indexes.
    authors: Vec<world::Author>,
    /// Per world, which of those authors made it.
    author_of: Vec<usize>,
    /// Every release that added a world, newest first, each carrying what it added. What
    /// [`Highlight::Version`] indexes, and what the catalog lists. See [`world::Dump::versions`].
    versions: Vec<world::Version>,
    /// What is lit. Everything off the highlight is dimmed.
    selected: Option<Highlight>,
    /// The depth of the furthest world the origin can reach, which is the last layer the rocker
    /// can step to.
    deepest: u32,
    /// Where the right button went down, while it is still down. The button both pans and opens
    /// the menu, and only the travel between press and release tells those apart.
    right_press: Option<PhysicalPoint>,
    /// The open context menu, if a right-click landed on a world. Drawn by the overlay, which
    /// also closes it once one of its entries is taken.
    menu: Option<ContextMenu>,
    /// Where the pointer last was with nothing pressed, in physical pixels, and what the hover
    /// test found there. `None` for a pointer that has left the window or moved onto the panel,
    /// and on a touch screen throughout: a finger never moves without pressing, so it never
    /// hovers. See [`AppEntities::track_gesture`].
    cursor: Option<PhysicalPoint>,
    hover: Option<usize>,
    /// Per world, how many other worlds it connects to: the degree of the graph as drawn, which
    /// is what tells a junction from a dead end. Only the overlay reads it.
    degrees: Vec<usize>,
    /// Whether the camera is still easing onto the selected route. Set by a selection and cleared
    /// once the camera arrives, or as soon as the person takes the camera back.
    framing: bool,
    /// Connection colors before a selection dims them, so clearing one restores the distance
    /// ramp. See [`edge_colors`].
    edge_colors: Vec<Srgba>,
    /// Per node, the half-height of its quad, from how much of the graph hangs off it.
    node_radii: Vec<f32>,
    /// The panel's setting for how much of a world's size goes into how hard it pushes. See
    /// [`node_mass`].
    hub_repulsion: f32,
    /// Per world, how many worlds hang off it. See [`world::Routes::descendant_counts`].
    descendants: Vec<u32>,
    /// The backdrop, drawn as a screen-filling quad before the graph. Not a [Gm] like the rest:
    /// it has no geometry of its own, only the material [`apply_screen_material`] stretches over
    /// the window.
    backdrop: ColorMaterial,
    /// The worlds: a picture of each on a camera-facing quad. Drawn only once the atlas they
    /// sample has arrived, because until then there is nothing to sample: see [`thumbnails`].
    thumbnails: Gm<InstancedMesh, ColorMaterial>,
    edges: Gm<InstancedMesh, ColorMaterial>,
    /// Kept so each frame can rewrite the transformations while keeping the colors:
    /// `set_instances` replaces the whole [Instances] struct.
    thumbnail_instances: Instances,
    edge_instances: Instances,
    /// The thumbnail atlas on its way in, until [`AppEntities::receive_atlas`] takes it.
    atlas: Option<fetch::Pending<Option<CpuTexture>>>,
    /// The same atlas as the sidebar's catalog draws out of. `None` until it arrives, and forever
    /// if it cannot be had: see [`thumbnails::Sheet`].
    sheet: Option<thumbnails::Sheet>,
    /// Full-size pictures for the worlds the view has come close enough to, over the atlas cells
    /// they stand in for. See [`detail`].
    detail: detail::Detail,
    /// The rotation that stood the quads square to the camera when their transformations were
    /// last built. Turning the camera has to rebuild them even though nothing in the layout has
    /// moved, and this is what says that it has been turned.
    billboard: Mat4,
}

/// Naive pseudo-RNG
struct Rng(u32);

impl Rng {
    fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
    fn next_f32(&mut self) -> f32 {
        self.next_u32() as f32 / u32::MAX as f32
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        #[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
        let window_builder = Window::default_attributes()
            .with_title("yume 2kki world graph")
            .with_min_inner_size(winit::dpi::LogicalSize::new(1280, 720))
            .with_maximized(true);

        // A phone gives an app the whole screen and nothing to say about it, so none of the hints
        // above mean anything here.
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
        self.reset(event_loop.create_window(window_builder).unwrap());
    }
    /// Android takes the drawing surface back whenever the app leaves the screen, and everything
    /// built on it -- the context, the meshes, the atlas -- dies with it. So it is all let go of
    /// here and built again by [`Self::resumed`], which is the same path a first start takes and
    /// therefore costs a fresh layout. The other platforms never call this.
    fn suspended(&mut self, _: &winit::event_loop::ActiveEventLoop) {
        // In this order: the meshes and the overlay have to give their buffers back while the
        // context that holds them is still there, and the context has to let the surface go
        // before the window it was made against.
        self.data = None;
        self.overlay = None;
        self.ctx.fig = None;
        self.ctx.wctx = None;
        self.ctx.window = None;
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
        fig.handle_winit_window_event(&event);
        match event {
            WindowEvent::Resized(physical_size) => {
                self.ctx.wctx.as_ref().unwrap().resize(physical_size);
            }
            WindowEvent::RedrawRequested => {
                self.draw();
                self.ctx.wctx.as_ref().unwrap().swap_buffers().unwrap();
                self.ctx.window.as_ref().unwrap().request_redraw();
            }
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            // A key held as the window is left never comes back up, and the view would go on
            // walking for as long as the app ran. Nothing else three-d reports says the window
            // has stopped hearing the keyboard.
            WindowEvent::Focused(false) => self.statics.walk = Walk::default(),
            WindowEvent::Touch(touch) => {
                // A second finger settles what the first one was doing: it was not a tap, not a
                // node drag and not an orbit, it was the start of a pinch.
                if self.statics.touches.track(&touch)
                    && let Some(data) = self.data.as_mut()
                {
                    data.gesture = None;
                }
            }
            _ => (),
        }
    }
}

impl App {
    pub fn new() -> Self {
        let camera = Camera::new_perspective(
            Viewport::new_at_origo(1, 1),
            vec3(0.0, -50., -150.),
            vec3(0.0, -50., 0.0),
            vec3(0.0, 1.0, 0.0),
            degrees(FOV_Y_DEGREES),
            0.1,
            1000.0,
        );
        let control = OrbitControl::new(camera.target(), 1.0, 300.0);

        Self {
            ctx: AppContext::default(),
            statics: AppStatics {
                control,
                camera,
                panning: false,
                cursor: CursorIcon::Default,
                touches: Touches::default(),
                walk: Walk::default(),
            },
            data: None,
            overlay: None,
        }
    }
    fn reset(&mut self, window: Window) {
        self.ctx.window = Some(window);
        let window = self.ctx.window.as_ref().unwrap();
        self.ctx.wctx =
            Some(WindowedContext::from_winit_window(&window, Default::default()).unwrap());
        let ctx = self.ctx.wctx.as_ref().unwrap();
        self.ctx.fig = Some(FrameInputGenerator::from_winit_window(&window));

        let rng = Rng(0x5eed_1337);

        let dump = world::load();
        let worlds = &dump.worlds;
        // How deep a player has to go to reach each world: see [`world::canonical_routes`]. It
        // both colors the nodes and, in layered mode, picks the layer each one is pinned to, so
        // the two always agree.
        let routes = world::canonical_routes(worlds);
        let deepest = routes.depth.iter().flatten().copied().max().unwrap_or(0);
        let furthest = deepest.max(1) as f32;
        // Sized by how much of the graph hangs off each world, through the anchor above: a leaf
        // keeps [`NODE_BASE_RADIUS`]. The logarithm is what carries the depth: a deep world with a
        // handful of worlds behind it lands within reach of a shallow one with a hundred, because
        // the size grows with the order of magnitude of what a world leads to rather than with the
        // count. See [`world::Routes::descendant_counts`].
        let descendants = routes.descendant_counts();
        let node_radii: Vec<_> = descendants
            .iter()
            .map(|&descendants| {
                let growth = (1.0 + descendants as f32).ln() / (1.0 + NODE_HUB_DESCENDANTS).ln();
                NODE_BASE_RADIUS * (1.0 + (NODE_HUB_GROWTH - 1.0) * growth)
            })
            .collect();

        let mut graph = <ForceGraph>::new(Default::default());
        // Positions come from `scatter` below, so the initial layout and a restart agree.
        let nodes: Vec<_> = routes
            .depth
            .iter()
            .enumerate()
            .map(|(world, depth)| {
                graph.add_node(NodeData {
                    // Worlds the origin cannot reach have no depth to share a layer with, so they
                    // get one of their own past the deepest that does.
                    level: depth.map_or(furthest + 1.0, |depth| depth as f32),
                    mass: node_mass(node_radii[world], HUB_REPULSION_DEFAULT),
                    ..Default::default()
                })
            })
            .collect();
        // A connection between two worlds is usually listed from both ends, and the graph is
        // directed, so a reciprocal pair would otherwise become two springs pulling the same two
        // nodes together twice as hard.
        let mut linked = std::collections::HashSet::new();
        // Counted off the same pairs the edges are built from, so the panel calls a world a
        // junction on exactly the lines it draws for it.
        let mut degrees = vec![0; worlds.len()];
        for (from, world) in worlds.iter().enumerate() {
            for connection in &world.connections {
                let to = connection.target_id;
                let pair = (from.min(to), from.max(to));
                if from != to && linked.insert(pair) {
                    graph.add_edge(nodes[from], nodes[to], Default::default());
                    degrees[from] += 1;
                    degrees[to] += 1;
                }
            }
        }

        // How deep each world is, as a color. Not drawn on the worlds themselves, which carry
        // pictures: the connections wear it instead. See [`edge_colors`].
        let depth_colors: Vec<_> = routes
            .depth
            .iter()
            .map(|depth| match depth {
                Some(depth) => distance_color(*depth as f32 / furthest),
                None => UNREACHED_COLOR,
            })
            .collect();
        let edge_colors = edge_colors(&graph, &routes, &descendants, &depth_colors);
        let titles: Vec<String> = worlds.iter().map(|world| world.title.clone()).collect();
        // The catalog's own two readings of the dump, built once here: both group every world,
        // which is not work a frame can afford, and both are ordered for the lists they fill.
        let (authors, author_of) = dump.authors();
        let versions = dump.versions();
        // White, so a picture reaches the screen as itself. Only a selection ever changes them,
        // dimming everything it does not light along with the rest of the graph.
        let thumbnail_instances = Instances {
            transformations: vec![Mat4::identity(); worlds.len()],
            colors: Some(vec![Srgba::WHITE; worlds.len()]),
            ..Default::default()
        };
        let edge_instances = Instances {
            transformations: vec![Mat4::identity(); edge_colors.len()],
            colors: Some(edge_colors.clone()),
            ..Default::default()
        };
        let backdrop = ColorMaterial {
            color: Srgba::WHITE,
            texture: Some(Texture2DRef::from_cpu_texture(ctx, &panorama_tile())),
            // Colour only, and no depth test: the quad stands in for the cleared background, so
            // it must neither occlude the graph nor be occluded by the cleared depth buffer.
            render_states: RenderStates {
                write_mask: WriteMask::COLOR,
                depth_test: DepthTest::Always,
                ..Default::default()
            },
            is_transparent: false,
        };
        // Quads rather than cubes, and turned to face the camera every frame: a picture has to be
        // seen square on to be seen at all, and a cube's own uv coordinates unwrap its six faces
        // across the image rather than giving each face the whole of it.
        //
        // No texture yet: it is the atlas, which is still on its way in. See
        // [`AppEntities::receive_atlas`].
        let thumbnails = Gm::new(
            InstancedMesh::new(&ctx, &thumbnail_instances, &CpuMesh::square()),
            ColorMaterial::default(),
        );
        let edges = Gm::new(
            InstancedMesh::new(&ctx, &edge_instances, &CpuMesh::cylinder(8)),
            ColorMaterial::default(),
        );

        self.overlay = Some(Overlay::new(ctx));

        let data = self.data.insert(AppEntities {
            graph,
            gesture: None,
            rng,
            routes,
            titles,
            authors,
            author_of,
            versions,
            selected: None,
            deepest,
            right_press: None,
            menu: None,
            cursor: None,
            hover: None,
            framing: false,
            edge_colors,
            node_radii,
            hub_repulsion: HUB_REPULSION_DEFAULT,
            descendants,
            degrees,
            backdrop,
            thumbnails,
            edges,
            thumbnail_instances,
            edge_instances,
            atlas: Some(thumbnails::load()),
            sheet: None,
            detail: detail::Detail::new(worlds.iter().map(|world| world.image.clone()).collect()),
            // Rebuilt on the first frame either way, because a fresh layout has not settled.
            billboard: Mat4::identity(),
        });
        scatter(data);
    }

    /// The meat of this module.
    fn draw(&mut self) {
        let ctx = self.ctx.wctx.as_ref().unwrap();
        let mut frame_input = self.ctx.fig.as_mut().unwrap().generate(&ctx);

        let data = self.data.as_mut().unwrap();
        self.statics.camera.set_viewport(frame_input.viewport);
        // egui marks what it uses as handled, so a click on the panel must not also reach the graph behind it.
        if self.overlay.as_mut().unwrap().run(&mut frame_input, data) {
            // Repulsion acts along the offset between two nodes, so a layout flattened onto the
            // plane has no depth for the forces to reinflate: leaving two dimensions has to
            // reseed the axis. Cheaper to restart the whole layout than to special-case it.
            scatter(data);
            if data.graph.parameters().dimensions == Dimensions::Two {
                // Squared onto the plane before the turn is locked, or a camera left oblique by
                // the three-dimensional view would stay that way with no way to straighten it.
                self.statics.face_plane();
            }
        }
        // Ahead of the pan and the orbit control, which both take whatever left-button motion
        // this leaves unhandled.
        data.track_gesture(
            &self.statics.camera,
            &mut frame_input.events,
            self.statics.touches.pinching,
        );
        if data.graph.parameters().dimensions == Dimensions::Two {
            // A flat layout has one face worth looking at, and seen from anywhere else it is a
            // layout read edge-on. So the camera is not allowed to turn: swallow the drag the
            // orbit control reads, which leaves it the zoom and leaves the pan alone.
            lock_rotation(&mut frame_input.events);
        }
        // Two fingers do both jobs at once, off the same pair of positions: three-d reads the gap
        // between them as the wheel the orbit control zooms on, and their midpoint's travel is
        // read here as the pan. They are independent, so a drag that also spreads does both.
        let dragged = self
            .statics
            .touches
            .take_travel(frame_input.device_pixel_ratio);
        // Read after the overlay, which is what settles whether the keys are being typed into
        // the search box rather than walked with.
        self.statics
            .walk
            .track(&frame_input.events, self.overlay.as_ref().unwrap().keyboard);
        let (across, into) = self
            .statics
            .walk
            .travel((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
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
        // framing rather than fighting it for the rest of the move.
        if panned || orbited {
            data.framing = false;
        }
        let orbiting = matches!(data.gesture, Some(Gesture::Orbiting))
            && data.graph.parameters().dimensions == Dimensions::Three;
        self.statics
            .track_cursor(self.ctx.window.as_ref().unwrap(), orbiting);
        if data.framing {
            // Recomputed every frame rather than fixed when the selection was made: the layout is
            // usually still moving, and a goal taken once would be stale before the camera
            // reached it.
            data.framing = match data.highlight_bounds() {
                Some(bounds) => self
                    .statics
                    .ease_to_frame(&bounds, (frame_input.elapsed_time as f32 * 1e-3).min(0.05)),
                None => false,
            };
        }

        data.pull_grabbed_node(&self.statics.camera);
        data.receive_atlas(ctx, self.overlay.as_ref().unwrap().gui.context());
        // A settled graph steps to nothing, and its instance buffers already hold the layout, so
        // there is no geometry to rebuild until something wakes it. Asked before the step,
        // because a step that ends settled has still moved the nodes.
        let stepped = !data.graph.is_settled();
        // Clamp dt so a stalled tab does not blow the simulation apart.
        data.graph
            .update((frame_input.elapsed_time as f32 * 1e-3).min(0.05));
        // The quads face the camera, so turning it dates their transformations even over a layout
        // that has not moved at all.
        let turned = data.billboard != billboard(&self.statics.camera);
        if stepped || turned {
            data.rebuild_instances(&self.statics.camera);
        }
        // After the instances, whose colors the full pictures borrow, and every frame rather than
        // only the moved ones: coming in on a node changes nothing in the layout and everything
        // about how much of the atlas the screen is asking for.
        let magnified = data.magnified(&self.statics.camera, frame_input.viewport);
        data.detail.track(ctx, &magnified);

        if let Some(texture) = data.backdrop.texture.as_mut() {
            texture.transformation = panorama_transform(
                frame_input.viewport,
                frame_input.device_pixel_ratio,
                self.statics.camera.view_direction(),
            );
        }
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
                apply_screen_material(ctx, &data.backdrop, &self.statics.camera, &[]);
                Ok(())
            })
            .unwrap()
            .render(
                &self.statics.camera,
                data.edges
                    .into_iter()
                    .chain(data.drawn_thumbnails())
                    .chain(data.detail.drawn()),
                &[],
            )
            .write(|| self.overlay.as_ref().unwrap().gui.render())
            .unwrap();
    }
}

impl Overlay {
    fn new(context: &Context) -> Self {
        let gui = GUI::new(context);
        egui_material_icons::initialize(&gui.context());
        Self {
            gui,
            fps: 0.0,
            sidebar: Sidebar::default(),
            keyboard: false,
            guide: guide::Guide::new(),
        }
    }

    /// Builds this frame's panel and consumes the events that land on it.
    ///
    /// The closure can only borrow `data` immutably, so everything the panel reads is walked out
    /// into [PanelData] first, and everything it decides is left on the [Panel] it is handed for
    /// [Overlay::run] to apply.
    fn panel(&mut self, frame_input: &mut FrameInput, data: &AppEntities) -> Panel {
        let read = PanelData::new(data, self.fps, &self.sidebar, frame_input);
        let parameters = data.graph.parameters();
        // Read here and written back after the panel: `parameters_mut` wakes the layout, so
        // touching it every frame would keep the graph from ever settling.
        let mut panel = Panel {
            dimensions: parameters.dimensions,
            hub_repulsion: data.hub_repulsion,
            layered: parameters.dag_level_distance.is_some(),
            chosen: None,
            lit: None,
            menu_taken: false,
            wants_pointer: false,
            guide: false,
        };
        // Bound out of `self` so the closure borrows this field alone, leaving `self.gui` free
        // for the call it is passed to.
        let sidebar = &mut self.sidebar;
        let guide = &mut self.guide;
        let insets = safe_insets(frame_input.viewport, frame_input.device_pixel_ratio);
        panel.wants_pointer = self.gui.update(
            &mut frame_input.events,
            frame_input.accumulated_time,
            frame_input.viewport,
            frame_input.device_pixel_ratio,
            |ui| {
                let style = ui.style();
                // Full height, so the safe area is kept by standing the contents off the panel's
                // own edges rather than by shrinking the panel: a sidebar that stopped short of
                // the status bar would leave a stripe of its own colour above it.
                let frame = egui::Frame::side_top_panel(style)
                    .inner_margin(egui::Margin {
                        left: insets.left + PANEL_MARGIN,
                        right: PANEL_MARGIN,
                        top: insets.top + PANEL_MARGIN,
                        bottom: insets.bottom + PANEL_MARGIN,
                    })
                    // The graph carries on behind the sidebar rather than stopping at it: see
                    // [`SIDEBAR_OPACITY`]. The style's own panel colour, only thinned, so the
                    // sidebar stays the same colour it would otherwise have been.
                    .fill(fade(style.visuals.panel_fill, SIDEBAR_OPACITY));
                match sidebar.closed {
                    false => {
                        egui::Panel::left("yumezu")
                            .frame(frame)
                            .default_size(SIDEBAR_WIDTH)
                            .show_inside(ui, |ui| panel.window(ui, &read, sidebar));
                    }
                    true => sidebar_opener(ui, sidebar, insets),
                }
                panel.rocker(ui, &read, insets);
                panel.menu(ui, &read);
                panel.tooltip(ui, &read);
                // Last, so it is drawn over everything it is explaining. Read a frame late when
                // the settings tab has just asked for it, which is a frame nobody sees.
                if panel.guide {
                    guide.reopen();
                }
                guide.show(ui.ctx());
            },
        );
        // Asked after the frame, which is when egui knows whether anything it drew took the
        // focus that typing would go to.
        track_keyboard(
            self.gui.context().egui_wants_keyboard_input(),
            &mut self.keyboard,
        );
        panel
    }

    /// Runs one frame of the panel and applies what it was left at.
    ///
    /// The panel owns the layout mode, so this both reads the parameters and writes back what the
    /// controls were left at. Returns whether the dimension changed, which is the one switch the
    /// caller has to follow up on.
    fn run(&mut self, frame_input: &mut FrameInput, data: &mut AppEntities) -> bool {
        // Guard the very first frame, which reports no elapsed time at all.
        let elapsed = (frame_input.elapsed_time as f32).max(1e-3);
        self.fps += (1000.0 / elapsed - self.fps) * (elapsed / FPS_WINDOW_MS).min(1.0);

        let panel = self.panel(frame_input, data);
        // Set by the panel too, and by the selections applied out here.
        let mut menu_taken = panel.menu_taken;
        if let Some(selection) = panel.lit {
            data.select(selection);
            menu_taken = true;
        }
        if menu_taken {
            data.menu = None;
        }
        // Always the route: the list is there to be walked to, and a world named in it is picked
        // to be gone to rather than to be opened out in turn.
        if let Some(world) = panel.chosen {
            data.select(Some(Highlight::Route(world)));
        }
        // [GUI::update] only surrenders the pointer while egui is actively working a widget, so
        // a scroll over the panel reaches the camera as a zoom and a press on a bare label
        // reaches the graph as a selection. Whatever egui wants the pointer for is the panel's.
        if panel.wants_pointer {
            for event in frame_input.events.iter_mut() {
                match event {
                    Event::MousePress { handled, .. }
                    | Event::MouseRelease { handled, .. }
                    | Event::MouseMotion { handled, .. }
                    | Event::MouseWheel { handled, .. }
                    | Event::PinchGesture { handled, .. } => *handled = true,
                    _ => (),
                }
            }
        }

        if panel.hub_repulsion != data.hub_repulsion {
            data.hub_repulsion = panel.hub_repulsion;
            data.apply_node_masses();
        }

        let was = data.graph.parameters();
        if panel.dimensions == was.dimensions && panel.layered == was.dag_level_distance.is_some() {
            return false;
        }
        let parameters = data.graph.parameters_mut();
        let reseed = panel.dimensions != parameters.dimensions;
        parameters.dimensions = panel.dimensions;
        let (spacing, slack) = match panel.dimensions {
            Dimensions::Two => (
                DAG_LEVEL_DISTANCE_2D,
                DAG_LEVEL_MICROSTEP * DAG_LEVEL_SLACK_MICROSTEPS_2D,
            ),
            Dimensions::Three => (DAG_LEVEL_DISTANCE, 0.0),
        };
        parameters.dag_level_distance = panel.layered.then_some(spacing);
        parameters.dag_level_slack = slack;
        // A switch rearranges the whole layout rather than nudging it, so it gets a full
        // settling window instead of what a settled graph has left: see [ForceGraph::revive].
        data.graph.revive();
        reseed
    }
}

impl<'a> PanelData<'a> {
    /// Walks out of `data` everything the panel will read, since the closure that builds it can
    /// only borrow `data` immutably and cannot call these methods itself.
    fn new(data: &'a AppEntities, fps: f32, sidebar: &Sidebar, frame_input: &FrameInput) -> Self {
        Self {
            data,
            fps,
            route: data.route(),
            notable: data.notable(),
            listed: match data.selected {
                Some(Highlight::Author(_) | Highlight::Version(_) | Highlight::Layer(_)) => {
                    data.highlighted()
                }
                _ => Vec::new(),
            },
            candidates: data.search(&sidebar.worlds),
            // Only the open one: the other's list is not drawn, and matching a few hundred names
            // for nobody is work every frame would pay for.
            authors: match sidebar.tab {
                Tab::Authors => matching(
                    data.authors.iter().map(|by| by.name.as_str()),
                    &sidebar.authors,
                ),
                _ => Vec::new(),
            },
            versions: match sidebar.tab {
                Tab::Versions => matching(
                    data.versions.iter().map(|version| version.name.as_str()),
                    &sidebar.versions,
                ),
                _ => Vec::new(),
            },
            selected: data.selected,
            menu: data
                .menu
                .as_ref()
                .map(|menu| (menu.world, into_points(menu.at, frame_input))),
            // Settled after the panel ran last frame, since the pointer is only resolved once the
            // panel has taken its share of the events: a frame behind the cursor, which at this
            // rate is not a frame anybody sees.
            hovered: data
                .hover
                .zip(data.cursor)
                .map(|(world, cursor)| (world, into_points(cursor, frame_input))),
        }
    }
}

/// A window position in physical pixels, counted from the bottom, into egui's points counted from
/// the top: the same conversion the integration puts every pointer event through.
fn into_points(at: PhysicalPoint, frame_input: &FrameInput) -> egui::Pos2 {
    let ratio = frame_input.device_pixel_ratio;
    egui::pos2(
        at.x / ratio,
        (frame_input.viewport.height as f32 - at.y) / ratio,
    )
}

impl Panel {
    /// The panel proper: the tabs, and whichever of them is open.
    ///
    /// The tab bar stands outside the scroll, so it stays reachable however far down a list the
    /// person has read.
    fn window(&mut self, ui: &mut egui::Ui, read: &PanelData, sidebar: &mut Sidebar) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut sidebar.tab, Tab::Graph, "Graph");
            ui.selectable_value(&mut sidebar.tab, Tab::Authors, "Authors");
            ui.selectable_value(&mut sidebar.tab, Tab::Versions, "Versions");
            ui.selectable_value(&mut sidebar.tab, Tab::Settings, ICON_SETTINGS);
            // Against the far edge of the row, so it is nowhere near the tabs it is not one of.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(ICON_CHEVRON_LEFT)
                    .on_hover_text("Hide the sidebar")
                    .clicked()
                {
                    sidebar.closed = true;
                }
            });
        });
        ui.separator();
        // One scroll for the whole tab, rather than one per list: the lists have the full height
        // of the window to run down, and a list that scrolled inside a column that also scrolled
        // would fight the drag that reached it.
        egui::ScrollArea::vertical().show(ui, |ui| match sidebar.tab {
            Tab::Graph => self.graph(ui, read, &mut sidebar.worlds),
            Tab::Authors => self.authors(ui, read, &mut sidebar.authors),
            Tab::Versions => self.versions(ui, read, &mut sidebar.versions),
            Tab::Settings => self.settings(ui),
        });
    }

    /// The graph in front of the person: how it is laid out, how to find a world in it, and what
    /// the current selection has to say for itself.
    fn graph(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        ui.label(format!("{:.0} fps", read.fps));
        ui.label(format!(
            "{} worlds, {} connections",
            data.titles.len(),
            data.graph.edge_count()
        ));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.dimensions, Dimensions::Two, "2D");
            ui.selectable_value(&mut self.dimensions, Dimensions::Three, "3D");
            ui.separator();
            ui.checkbox(&mut self.layered, "layered")
                .on_hover_text("Separate worlds into layers of depths");
        });
        ui.separator();
        egui::TextEdit::singleline(search)
            .prefix(ICON_SEARCH)
            .suffix(ICON_CANCEL)
            .show(ui);
        for &world in &read.candidates {
            if ui
                .selectable_label(
                    read.selected.and_then(Highlight::world) == Some(world),
                    &data.titles[world],
                )
                .clicked()
            {
                self.chosen = Some(world);
            }
        }
        ui.separator();
        self.selection(ui, read);
    }

    /// Who made the worlds: everyone the wiki credits, busiest first, and how much each of them
    /// is behind. Clicking a name lights their whole body of work.
    fn authors(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        search_box(ui, search, "author");
        ui.label(showing(read.authors.len(), data.authors.len(), "authors"));
        ui.separator();
        for &author in &read.authors {
            let by = &data.authors[author];
            if ui
                .selectable_label(
                    read.selected == Some(Highlight::Author(author)),
                    format!("{}  ({})", by.name, by.worlds.len()),
                )
                .clicked()
            {
                self.lit = Some(Some(Highlight::Author(author)));
            }
        }
    }

    /// What each release added, newest first, each shown by a few of the worlds it brought.
    ///
    /// Listed whole rather than cut to the best few: a release is looked up as often by reading
    /// down the history as by name, and the pictures are what make it worth reading down.
    fn versions(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        search_box(ui, search, "version");
        ui.label(showing(
            read.versions.len(),
            data.versions.len(),
            "versions",
        ));
        ui.separator();
        for &version in &read.versions {
            self.version(ui, read, version);
        }
    }

    /// One release in the catalog: what it is called, when it landed, and a few of the worlds it
    /// brought with it.
    ///
    /// The whole row lights the release; the pictures are hotter than that, and each one goes to
    /// its own world, so the catalog is a way into the graph rather than only a way to read it.
    fn version(&mut self, ui: &mut egui::Ui, read: &PanelData, version: usize) {
        let data = read.data;
        let release = &data.versions[version];
        // Every row of the list draws the same widgets, and egui names a widget by what is in it:
        // the release's own place in the list is what keeps two rows apart.
        ui.push_id(version, |ui| {
            let lit = read.selected == Some(Highlight::Version(version));
            if ui
                .selectable_label(
                    lit,
                    format!(
                        "{}  ({}{}{})",
                        release.name,
                        worlds(release.worlds.len()),
                        if release.released.is_empty() {
                            ""
                        } else {
                            ", "
                        },
                        release.released,
                    ),
                )
                .clicked()
            {
                self.lit = Some(Some(Highlight::Version(version)));
            }
            // Nothing at all until the atlas has arrived, and nothing ever if it cannot be had:
            // the rest of the row already says what the release is.
            let Some(sheet) = &data.sheet else { return };
            ui.horizontal(|ui| {
                for &world in release.worlds.iter().take(CATALOG_THUMBNAILS) {
                    if ui
                        .add(
                            sheet
                                .picture(world, CATALOG_THUMBNAIL_HEIGHT)
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(&data.titles[world])
                        .clicked()
                    {
                        self.chosen = Some(world);
                    }
                }
            });
        });
    }

    /// The knobs: set once and then left alone, which is why they are not in the way of the
    /// reading tabs.
    fn settings(&mut self, ui: &mut egui::Ui) {
        ui.add(egui::Slider::new(&mut self.hub_repulsion, 0.0..=1.0).text("hub push"))
            .on_hover_text("The higher the value, the harder bigger worlds' repulsion force is");
        // The way back to a panel that was dismissed for good, so ticking that box is not a
        // door that locks behind the person who ticked it.
        self.guide |= ui.button("Show controls").clicked();
    }

    /// What is lit, read out: the route to it, what hangs off it, or the author it is credited to.
    fn selection(&mut self, ui: &mut egui::Ui, read: &PanelData) {
        let data = read.data;
        match read.selected {
            None => {
                ui.label(
                    "Click a world to trace its route to the origin, \
                     or right-click it for more.",
                );
            }
            Some(Highlight::Route(world)) => {
                self.lit = self.lit.or(world_info(ui, data, world).map(Some));
                ui.label(format!(
                    "{} connections from the origin",
                    read.route.len() - 1
                ));
                // Origin first, so the list reads in the order it is walked.
                for (step, &world) in read.route.iter().rev().enumerate() {
                    ui.monospace(format!("{step:>2}  {}", data.titles[world]));
                }
            }
            Some(Highlight::Descendants(world)) => {
                self.lit = self.lit.or(world_info(ui, data, world).map(Some));
                ui.label(format!(
                    "{} worlds are reached through it",
                    data.descendants[world]
                ));
                if read.notable.is_empty() {
                    ui.label("Nothing branches off it.");
                }
                for &world in &read.notable {
                    let degree = data.degrees[world];
                    let kind = match degree {
                        ..NOTABLE_HUB_CONNECTIONS => "dead end",
                        _ => "junction",
                    };
                    if ui
                        .selectable_label(
                            false,
                            format!("{}  ({kind}, {degree})", data.titles[world]),
                        )
                        .clicked()
                    {
                        self.chosen = Some(world);
                    }
                }
            }
            // The author is the subject here, not the world that named them, so that world's own
            // information gives way to their whole body of work.
            Some(Highlight::Author(author)) => {
                let by = &data.authors[author];
                ui.horizontal(|ui| {
                    ui.strong(&by.name);
                    if ui.button(ICON_OPEN_IN_NEW).clicked() {
                        open_in_browser(&world::author_url(&by.name));
                    }
                });
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    if ui.selectable_label(false, &data.titles[world]).clicked() {
                        self.chosen = Some(world);
                    }
                }
            }
            // A release is a list like an author's work is, and read the same way: what it
            // added, in world order.
            Some(Highlight::Version(version)) => {
                let release = &data.versions[version];
                ui.strong(&release.name);
                if !release.released.is_empty() {
                    ui.label(format!("released {}", release.released));
                }
                ui.label(format!("{} added", worlds(read.listed.len())));
                for &world in &read.listed {
                    if ui.selectable_label(false, &data.titles[world]).clicked() {
                        self.chosen = Some(world);
                    }
                }
            }
            // A layer is a shell rather than a list: it is read off the graph, so the panel only
            // says which shell is lit and how much of the game sits on it.
            Some(Highlight::Layer(depth)) => {
                ui.strong(format!("Depth {depth}"));
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    if ui.selectable_label(false, &data.titles[world]).clicked() {
                        self.chosen = Some(world);
                    }
                }
            }
        }
    }

    /// The rocker in the bottom corner, which steps the lit layer in and out from the origin.
    ///
    /// An [egui::Area] rather than part of the sidebar: it is a control over the graph, and it
    /// belongs in the corner the graph is still visible in whatever the sidebar is showing. Its
    /// middle is the off position, so a layer can be dropped without having to click a world.
    fn rocker(&mut self, ui: &mut egui::Ui, read: &PanelData, insets: egui::Margin) {
        let deepest = read.data.deepest;
        let lit = match read.selected {
            Some(Highlight::Layer(depth)) => Some(depth),
            _ => None,
        };
        egui::Area::new(egui::Id::new("layer rocker"))
            .anchor(
                egui::Align2::RIGHT_BOTTOM,
                [
                    -((insets.right + PANEL_MARGIN) as f32),
                    -((insets.bottom + PANEL_MARGIN) as f32),
                ],
            )
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        if ui
                            .add_enabled(lit != Some(0), egui::Button::new(ICON_KEYBOARD_ARROW_UP))
                            .on_hover_text("Shallower")
                            .clicked()
                        {
                            self.lit = Some(Some(Highlight::Layer(
                                lit.map_or(deepest, |depth| depth.saturating_sub(1)),
                            )));
                        }
                        if ui
                            .add_enabled(
                                lit != Some(deepest),
                                egui::Button::new(ICON_KEYBOARD_ARROW_DOWN),
                            )
                            .on_hover_text("Deeper")
                            .clicked()
                        {
                            self.lit = Some(Some(Highlight::Layer(
                                lit.map_or(0, |depth| (depth + 1).min(deepest)),
                            )));
                        }
                    });
                });
            });
    }

    /// The menu a right-click opened, drawn over everything else where the click landed.
    fn menu(&mut self, ui: &mut egui::Ui, read: &PanelData) {
        let Some((world, at)) = read.menu else {
            return;
        };
        let data = read.data;
        egui::Area::new(egui::Id::new("world menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(at)
            .show(ui.ctx(), |ui| {
                egui::Frame::menu(ui.style()).show(ui, |ui| {
                    ui.set_max_width(POPUP_WIDTH);
                    ui.strong(&data.titles[world]);
                    if data.descendants[world] > 0 && ui.button("Highlight descendants").clicked() {
                        self.lit = Some(Some(Highlight::Descendants(world)));
                    }
                    if ui.button("Open on yume.wiki").clicked() {
                        open_in_browser(&world::wiki_url(&data.titles[world]));
                        self.menu_taken = true;
                    }
                });
            });
    }

    /// Names the world under the pointer, beside the pointer.
    ///
    /// Not while a menu is open: the menu is over the same world and already names it, and the
    /// tooltip would only land on top of it.
    fn tooltip(&self, ui: &mut egui::Ui, read: &PanelData) {
        let Some((world, at)) = read.hovered.filter(|_| read.menu.is_none()) else {
            return;
        };
        egui::Area::new(egui::Id::new("world tooltip"))
            .order(egui::Order::Tooltip)
            // Kept inside the window, so a world hovered near the right edge is still readable.
            .constrain(true)
            .fixed_pos(at + egui::vec2(TOOLTIP_OFFSET, TOOLTIP_OFFSET))
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_max_width(POPUP_WIDTH);
                    ui.label(&read.data.titles[world]);
                });
            });
    }
}

impl AppStatics {
    /// Slides the camera and its orbit center across the view plane.
    ///
    /// [OrbitControl] only orbits and zooms, so panning lives here, on the buttons it leaves
    /// alone. Called before it, so a pan drag is not also read as an orbit.
    fn pan(&mut self, events: &mut [Event], device_pixel_ratio: f32) -> bool {
        let mut panned = false;
        for event in events.iter_mut() {
            if let Event::MouseRelease {
                button: MouseButton::Right | MouseButton::Middle,
                ..
            } = event
            {
                self.panning = false;
            }
            let Event::MouseMotion {
                button: Some(MouseButton::Right | MouseButton::Middle),
                delta,
                handled,
                ..
            } = event
            else {
                continue;
            };
            if *handled {
                continue;
            }
            let delta = *delta;
            *handled = true;
            panned |= self.pan_by(delta, device_pixel_ratio);
            self.panning = true;
        }
        panned
    }

    /// Slides the camera and its orbit center by a drag, in logical pixels.
    ///
    /// Shared by the two ways of asking for one: a button the mouse has spare, and the two
    /// fingers a screen has instead.
    fn pan_by(&mut self, delta: (f32, f32), device_pixel_ratio: f32) -> bool {
        if delta == (0.0, 0.0) {
            return false;
        }
        let scale = self.world_per_pixel(device_pixel_ratio);
        // Opposite the drag: moving the camera left pushes the graph right.
        let translation = self.camera.up_orthogonal() * (delta.1 * scale)
            - self.camera.right_direction() * (delta.0 * scale);
        self.camera.translate(translation);
        self.control.target += translation;
        true
    }

    /// Flies the camera and its orbit center along the direction it is looking, by a distance
    /// given in logical pixels the way a pan's is.
    ///
    /// The center travels with the camera, which is what makes this a move through the graph
    /// rather than the wheel's zoom: the two never close on each other, so the view keeps its
    /// speed instead of creeping to a halt against a point it can never reach.
    fn dolly_by(&mut self, travel: f32, device_pixel_ratio: f32) -> bool {
        if travel == 0.0 {
            return false;
        }
        let translation =
            self.camera.view_direction() * (travel * self.world_per_pixel(device_pixel_ratio));
        self.camera.translate(translation);
        self.control.target += translation;
        true
    }

    /// World units per logical pixel on the plane through the orbit center. What keeps a drag
    /// holding whatever it started on, and a walk covering the same apparent ground however far
    /// out the camera is standing.
    fn world_per_pixel(&self, device_pixel_ratio: f32) -> f32 {
        let distance = self.control.target.distance(self.camera.position());
        let logical_height = self.camera.viewport().height as f32 / device_pixel_ratio;
        2.0 * distance * (FOV_Y_DEGREES.to_radians() * 0.5).tan() / logical_height
    }

    /// Puts the pointer for the camera move in progress on the window.
    ///
    /// Only the two moves that take a drag are named: a pan holds the graph, an orbit turns it.
    /// The orbit is left unnamed in two dimensions, where the turn it would promise is locked.
    fn track_cursor(&mut self, window: &Window, orbiting: bool) {
        let cursor = match () {
            _ if self.panning => CursorIcon::Move,
            _ if orbiting => CursorIcon::Grabbing,
            _ => CursorIcon::Default,
        };
        if cursor != self.cursor {
            self.cursor = cursor;
            window.set_cursor(cursor);
        }
    }

    /// Turns the camera square to the `z = 0` plane, keeping where it looks and how far off it
    /// stands.
    ///
    /// The one turn two-dimensional mode makes on its own, because it is also the one turn the
    /// person can no longer make: see [`lock_rotation`].
    fn face_plane(&mut self) {
        let target = self.control.target;
        let distance = target.distance(self.camera.position());
        self.camera.set_view(
            target + vec3(0.0, 0.0, distance),
            target,
            vec3(0.0, 1.0, 0.0),
        );
    }

    /// Eases the camera one frame's worth toward the view that holds `bounds` in frame, and
    /// reports whether it still has ground to cover.
    ///
    /// Only the orbit centre and the distance move: the direction the camera looks from is left
    /// exactly as the person left it, so the graph does not spin under them while it closes in.
    /// Framed against whichever of the two field of view angles is narrower, so a route that fits
    /// vertically cannot still hang off the sides of a tall window.
    fn ease_to_frame(&mut self, bounds: &Bounds, dt: f32) -> bool {
        let viewport = self.camera.viewport();
        let half_y = FOV_Y_DEGREES.to_radians() * 0.5;
        let half_x = (half_y.tan() * viewport.width as f32 / viewport.height as f32).atan();
        // Against the bounding sphere, so the fit does not depend on which way the route is
        // turned relative to the camera.
        let goal_distance = (bounds.radius * FRAMING_MARGIN / half_y.min(half_x).sin())
            .clamp(self.control.min_distance, self.control.max_distance);

        let offset = self.camera.position() - self.control.target;
        let up = self.camera.up();
        let distance = offset.magnitude();
        let arrived = (bounds.center - self.control.target).magnitude()
            < bounds.radius * FRAMING_ARRIVAL_TOLERANCE
            && (goal_distance - distance).abs() < goal_distance * FRAMING_ARRIVAL_TOLERANCE;
        if arrived {
            return false;
        }

        // Exponential ease: the step is a fixed fraction of what is left, taken per unit of time
        // rather than per frame, so the move takes as long on a slow frame rate as on a fast one.
        let step = 1.0 - (-dt / (FRAMING_WINDOW_MS * 1e-3)).exp();
        let target = self.control.target + (bounds.center - self.control.target) * step;
        let distance = distance + (goal_distance - distance) * step;
        self.camera
            .set_view(target + offset / offset.magnitude() * distance, target, up);
        self.control.target = target;
        true
    }
}

impl AppEntities {
    /// Rewrites every mass from the current [`AppEntities::hub_repulsion`].
    ///
    /// The visit runs in slot order, which is world order: the nodes were added one per world and
    /// none is ever removed, which is the same reading of an index the rest of the module makes.
    fn apply_node_masses(&mut self) {
        let (radii, hub_repulsion) = (&self.node_radii, self.hub_repulsion);
        let mut world = 0;
        self.graph.visit_nodes_mut(|mut node| {
            node.set_mass(node_mass(radii[world], hub_repulsion));
            world += 1;
        });
    }

    /// Resolves the left button between clicking a node, dragging a node and orbiting, marking
    /// only the events the winner actually uses as handled.
    ///
    /// Runs before [OrbitControl], which is the loser of this contest: it reads any unhandled
    /// left-drag motion, so the decision has to be made, and the undecided motion swallowed,
    /// upstream of it.
    ///
    /// Also where the hover is settled, off the same events: it is the reading of the pointer
    /// that no button is contesting.
    fn track_gesture(&mut self, camera: &Camera, events: &mut [Event], pinching: bool) {
        for event in events.iter_mut() {
            match event {
                // While more than one finger is down the left button belongs to the pinch:
                // three-d works it with the first of them, and neither its travel nor its release
                // is a gesture of its own. Swallowed rather than skipped, so that the orbit
                // control downstream does not read them either.
                Event::MousePress {
                    button: MouseButton::Left,
                    handled,
                    ..
                }
                | Event::MouseMotion {
                    button: Some(MouseButton::Left),
                    handled,
                    ..
                }
                | Event::MouseRelease {
                    button: MouseButton::Left,
                    handled,
                    ..
                } if pinching => *handled = true,
                Event::MousePress {
                    button: MouseButton::Left,
                    position,
                    handled,
                    ..
                } if !*handled => {
                    // A press anywhere the menu did not take is a press past it, which closes it.
                    self.menu = None;
                    // Nominate, do not award: the press is ambiguous even over empty space, where
                    // it may still turn out to be the click that clears the selection.
                    self.gesture = Some(Gesture::Held {
                        hit: self.pick(camera, *position),
                        origin: *position,
                    });
                    *handled = true;
                }
                // Only while the button is actually down: a release lost to a focus change
                // would otherwise leave a nomination standing for the next hover to award.
                Event::MouseMotion {
                    button: Some(MouseButton::Left),
                    position,
                    handled,
                    ..
                } if !*handled => {
                    let awarded = match &mut self.gesture {
                        Some(Gesture::Held { hit, origin, .. }) => {
                            if (position.x - origin.x).hypot(position.y - origin.y)
                                <= GESTURE_SLOP_PIXELS
                            {
                                // Still within the slop, so this may yet be a click. Swallow it,
                                // or the camera would orbit under every click on a node.
                                *handled = true;
                                None
                            } else {
                                Some(match hit.take() {
                                    Some(mut grab) => {
                                        grab.cursor = *position;
                                        *handled = true;
                                        Gesture::Moving(grab)
                                    }
                                    // Nothing under the press, so the drag belongs to the camera.
                                    None => Gesture::Orbiting,
                                })
                            }
                        }
                        Some(Gesture::Moving(grab)) => {
                            grab.cursor = *position;
                            *handled = true;
                            None
                        }
                        Some(Gesture::Orbiting) | None => None,
                    };
                    if let Some(awarded) = awarded {
                        self.gesture = Some(awarded);
                    }
                }
                // The right button is contested too — it pans the camera — so it is nominated the
                // same way the left one is, and left unhandled for the pan to claim its motion.
                // Any press closes the open menu: whatever this turns out to be, it is not the
                // menu, and a pan would carry the graph out from under it.
                Event::MousePress {
                    button: MouseButton::Right,
                    position,
                    handled,
                    ..
                } if !*handled => {
                    self.menu = None;
                    self.right_press = Some(*position);
                }
                Event::MouseRelease {
                    button: MouseButton::Right,
                    position,
                    ..
                } => {
                    // A press that never travelled is a click, and a click on a world opens its
                    // menu where it landed.
                    if let Some(origin) = self.right_press.take()
                        && (position.x - origin.x).hypot(position.y - origin.y)
                            <= GESTURE_SLOP_PIXELS
                    {
                        self.menu = self.pick(camera, *position).map(|grab| ContextMenu {
                            world: grab.node.index(),
                            at: *position,
                        });
                    }
                }
                // Motion with nothing pressed is nobody's gesture, so it is read rather than
                // taken. A position the panel already claimed is not on the graph at all, which
                // is the same as having no pointer over it.
                Event::MouseMotion {
                    button: None,
                    position,
                    handled,
                    ..
                } => self.cursor = (!*handled).then_some(*position),
                Event::MouseLeave => self.cursor = None,
                Event::MouseRelease {
                    button: MouseButton::Left,
                    ..
                } => {
                    // A gesture still undecided at release never travelled: it is a click, which
                    // selects what it landed on, or clears the selection over empty space.
                    let clicked = match &self.gesture {
                        Some(Gesture::Held { hit, .. }) => Some(hit.as_ref().map(|grab| grab.node)),
                        _ => None,
                    };
                    self.gesture = None;
                    if let Some(node) = clicked {
                        self.select(node.map(|node| Highlight::Route(node.index())));
                    }
                }
                _ => (),
            }
        }
        // Retested every frame rather than only on motion: the layout is usually still moving and
        // the camera can be flown with the keys, so what is under a cursor that has not moved at
        // all changes anyway. One walk of the nodes, which is a fraction of the walk
        // [`AppEntities::magnified`] already makes per frame. Nothing hovers while a gesture is
        // in progress: the pointer is busy dragging a node or turning the camera.
        self.hover = match self.gesture {
            None => self
                .cursor
                .and_then(|cursor| self.pick(camera, cursor))
                .map(|grab| grab.node.index()),
            Some(_) => None,
        };
    }

    /// Finds the node nearest the camera drawn under `cursor`.
    ///
    /// Against the node positions rather than against the drawn geometry: the plates are one
    /// instanced mesh, so there is nothing per node to intersect. The test is how far the node
    /// lands from the cursor on screen, which is the only distance the person clicking can see.
    fn pick(&self, camera: &Camera, cursor: PhysicalPoint) -> Option<Grab> {
        let origin = camera.position_at_pixel(cursor);
        let direction = camera.view_direction_at_pixel(cursor);
        let mut nearest: Option<Grab> = None;
        self.graph.visit_nodes(|node| {
            let position = world_pos(node.position());
            let depth = (position - origin).dot(direction);
            // Behind the camera, or further off than something already found.
            if depth <= 0.0 || nearest.as_ref().is_some_and(|near| near.depth <= depth) {
                return;
            }
            let pixel = camera.pixel_at_position(position);
            let tolerance = GRAB_TOLERANCE_PIXELS
                .max(0.5 * drawn_width(camera, self.node_radii[node.index().index()], position));
            if (pixel.x - cursor.x).hypot(pixel.y - cursor.y) < tolerance {
                nearest = Some(Grab {
                    node: node.index(),
                    depth,
                    cursor,
                });
            }
        });
        nearest
    }

    /// Pulls a node being dragged toward the cursor, which also keeps the simulation awake.
    fn pull_grabbed_node(&mut self, camera: &Camera) {
        let Some(Gesture::Moving(grab)) = &self.gesture else {
            return;
        };
        let Some(position) = self.graph.node(grab.node).map(|node| node.position()) else {
            return;
        };
        let target = camera.position_at_pixel(grab.cursor)
            + camera.view_direction_at_pixel(grab.cursor) * grab.depth;
        let offset = sim_pos(target) - sim_pos(world_pos(position));
        let force = offset * GRAB_STIFFNESS;
        let force = force * (GRAB_FORCE_MAX / force.magnitude().max(GRAB_FORCE_MAX));
        self.graph.apply_force(grab.node, force.into());
    }

    /// The selected node and every step from it back to the origin world, selection first. Empty
    /// with nothing selected, whatever the selection lights: the route is what the panel walks.
    fn route(&self) -> Vec<usize> {
        let mut route = Vec::new();
        let mut step = self.selected.and_then(Highlight::world);
        while let Some(node) = step {
            route.push(node);
            step = self.routes.parents[node];
        }
        route
    }

    /// Every world the selection lights: its route home, or everything behind it. Empty with
    /// nothing selected.
    fn highlighted(&self) -> Vec<usize> {
        match self.selected {
            None => Vec::new(),
            Some(Highlight::Route(_)) => self.route(),
            Some(Highlight::Descendants(world)) => self.routes.subtree(world),
            Some(Highlight::Author(author)) => self.authors[author].worlds.clone(),
            Some(Highlight::Version(version)) => self.versions[version].worlds.clone(),
            Some(Highlight::Layer(depth)) => self.layer(depth),
        }
    }

    /// Every world exactly `depth` connections from the origin, in world order. The worlds the
    /// origin cannot reach have no depth and so sit on no layer.
    fn layer(&self, depth: u32) -> Vec<usize> {
        self.routes
            .depth
            .iter()
            .enumerate()
            .filter(|(_, at)| **at == Some(depth))
            .map(|(world, _)| world)
            .collect()
    }

    /// The worlds worth naming out of a lit subtree, at most [`NOTABLE_WORLDS`] of them. Empty
    /// unless a subtree is what is lit.
    ///
    /// Two kinds are worth a name, and they are worth it for opposite reasons: a junction, which
    /// is where the subtree opens out, and a dead end, which is where it stops. Neither measure
    /// ranks the other, so the two are ranked apart — junctions by how many ways they offer, dead
    /// ends by how far out they are — and then taken in turns, which is what keeps the list from
    /// filling with junctions before it names a single place the subtree ends.
    fn notable(&self) -> Vec<usize> {
        let Some(Highlight::Descendants(root)) = self.selected else {
            return Vec::new();
        };
        let subtree = self.routes.subtree(root);
        let mut junctions: Vec<_> = subtree
            .iter()
            .copied()
            .filter(|&world| world != root && self.degrees[world] >= NOTABLE_HUB_CONNECTIONS)
            .collect();
        junctions.sort_unstable_by_key(|&world| (std::cmp::Reverse(self.degrees[world]), world));
        let mut dead_ends: Vec<_> = subtree
            .iter()
            .copied()
            .filter(|&world| world != root && self.degrees[world] <= 1)
            .collect();
        dead_ends
            .sort_unstable_by_key(|&world| (std::cmp::Reverse(self.routes.depth[world]), world));

        let mut notable = Vec::with_capacity(NOTABLE_WORLDS);
        let (mut junctions, mut dead_ends) = (junctions.into_iter(), dead_ends.into_iter());
        // Whichever kind runs out first, the other goes on filling the list alone.
        while notable.len() < NOTABLE_WORLDS {
            let taken = notable.len();
            notable.extend(junctions.next());
            if notable.len() < NOTABLE_WORLDS {
                notable.extend(dead_ends.next());
            }
            if notable.len() == taken {
                break;
            }
        }
        notable
    }

    /// The sphere that holds every world the selection lights. `None` with nothing selected.
    ///
    /// Centred on the middle of their bounding box rather than on their average, so a route that
    /// piles up near the origin and reaches out with a few steps is still framed around what it
    /// spans instead of around where most of it sits.
    fn highlight_bounds(&self) -> Option<Bounds> {
        let highlighted = self.highlighted();
        if highlighted.is_empty() {
            return None;
        }
        let mut on_route = vec![false; self.titles.len()];
        for node in highlighted {
            on_route[node] = true;
        }
        let mut positions = Vec::new();
        self.graph.visit_nodes(|node| {
            if on_route[node.index().index()] {
                positions.push(world_pos(node.position()));
            }
        });

        let fold = |f: fn(f32, f32) -> f32| {
            positions
                .iter()
                .copied()
                .reduce(|a, b| vec3(f(a.x, b.x), f(a.y, b.y), f(a.z, b.z)))
                .unwrap()
        };
        let center = (fold(f32::min) + fold(f32::max)) * 0.5;
        let radius = positions
            .iter()
            .map(|p| (p - center).magnitude())
            .fold(0.0, f32::max);
        Some(Bounds {
            center,
            radius: radius.max(FRAMING_MIN_RADIUS),
        })
    }

    /// The worlds whose titles match `needle`, best first, at most [`SEARCH_CANDIDATES`] of them.
    ///
    /// Ranked by where the match falls and then by how much title is left over, so a world whose
    /// name starts with what was typed comes before one that merely contains it, and an exact
    /// name comes before the longer names that extend it. Empty for an empty needle: every world
    /// matches it, and ten arbitrary ones are noise rather than suggestions.
    fn search(&self, needle: &str) -> Vec<usize> {
        let needle = needle.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut hits: Vec<_> = self
            .titles
            .iter()
            .enumerate()
            .filter_map(|(world, title)| {
                let at = title.to_lowercase().find(&needle)?;
                Some((at, title.len(), world))
            })
            .collect();
        hits.sort_unstable();
        hits.truncate(SEARCH_CANDIDATES);
        hits.into_iter().map(|(_, _, world)| world).collect()
    }

    /// Points the highlight at a world, or clears it.
    fn select(&mut self, selected: Option<Highlight>) {
        if selected != self.selected {
            self.selected = selected;
            // Only a selection is worth moving the camera for: clearing one leaves the person
            // looking at whatever they were looking at.
            self.framing = selected.is_some();
            self.repaint();
        }
    }

    /// Rewrites the instance colors for the current selection: the connections it lights keep
    /// their depth colors, the steps between the lit worlds are lit brighter still, and everything
    /// else is dimmed. With nothing selected the graph goes back to its plain colors.
    ///
    /// Uploads on its own rather than waiting for [`Self::rebuild_instances`], which a settled
    /// graph never reaches.
    fn repaint(&mut self) {
        let mut on_route = vec![false; self.titles.len()];
        for node in self.highlighted() {
            on_route[node] = true;
        }
        let lit = self.selected.is_some();

        // White leaves a picture as itself, so a world is only ever painted to push it back behind
        // what a selection lights.
        let colors = self.thumbnail_instances.colors.as_mut().unwrap();
        for (color, &on_route) in colors.iter_mut().zip(&on_route) {
            *color = if !lit || on_route {
                Srgba::WHITE
            } else {
                dim(Srgba::WHITE)
            };
        }

        let (routes, on_route) = (&self.routes.parents, &on_route);
        let (colors, base) = (
            self.edge_instances.colors.as_mut().unwrap(),
            &self.edge_colors,
        );
        // The same order [`Self::rebuild_instances`] writes the transformations in.
        let mut edge = 0;
        self.graph.visit_edges(|a, b, _| {
            let (a, b) = (a.index().index(), b.index().index());
            // Whether this edge is a canonical step: the move from one of its ends to that
            // end's own parent.
            let step = routes[a] == Some(b) || routes[b] == Some(a);
            colors[edge] = match self.selected {
                None => base[edge],
                // A layer is a shell rather than a walk, so what is worth seeing across it is
                // where it is stitched to itself, not the step each of its worlds takes home.
                // Both ends being lit is the whole test, which for a layer already means both
                // ends are on it, and such an edge is never a canonical step: a parent is always
                // exactly one depth in. So it carries no route, keeps its own distance color
                // rather than being lit as if it did, and only escapes the dimming.
                Some(Highlight::Layer(_)) if on_route[a] && on_route[b] => base[edge],
                // Both ends being lit is not enough: it also has to be the step from one of them
                // to that end's own parent, or a shortcut between two distant points of a route
                // would light up as if the walk went through it, and a subtree would be webbed
                // with lines that carry no route at all.
                Some(_) if step && on_route[a] && on_route[b] => ROUTE_COLOR,
                Some(_) => dim(base[edge]),
            };
            edge += 1;
        });

        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.edges.set_instances(&self.edge_instances);
    }

    /// Rewrites the instance transformations from the current node positions and the direction
    /// the camera is looking from.
    fn rebuild_instances(&mut self, camera: &Camera) {
        let billboard = billboard(camera);
        let thumbnails = &mut self.thumbnail_instances.transformations;
        thumbnails.clear();
        self.graph.visit_nodes(|node| {
            let radius = self.node_radii[node.index().index()];
            // Wider than tall, in the shape of the pictures: a world image is a screenshot of the
            // game, so a square node would either crop a third off every one of them or stretch
            // them all.
            thumbnails.push(
                Mat4::from_translation(world_pos(node.position()))
                    * billboard
                    * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0),
            );
        });
        let edges = &mut self.edge_instances.transformations;
        edges.clear();
        self.graph.visit_edges(|a, b, _| {
            let (from, to) = (world_pos(a.position()), world_pos(b.position()));
            let dir = to - from;
            edges.push(
                Mat4::from_translation(from)
                    * rotation_matrix_from_dir_to_dir(vec3(1.0, 0.0, 0.0), dir.normalize())
                    * Mat4::from_nonuniform_scale(dir.magnitude(), EDGE_RADIUS, EDGE_RADIUS),
            );
        });
        self.thumbnails.set_instances(&self.thumbnail_instances);
        self.edges.set_instances(&self.edge_instances);
        self.billboard = billboard;
    }

    /// Takes the thumbnail atlas once it has arrived, points the thumbnail quads at their own
    /// cells of it, and hands egui a copy for the catalog. Draws nothing until then, and nothing
    /// ever if the atlas cannot be had: see [`thumbnails::load`].
    fn receive_atlas(&mut self, context: &Context, egui: &egui::Context) {
        let Some(loaded) = self.atlas.as_ref().and_then(fetch::Pending::take) else {
            return;
        };
        // Whatever came of it, there is nothing left to poll for.
        self.atlas = None;
        // Both failures are already logged where they are found, and both leave the graph drawn
        // exactly as it was before thumbnails existed.
        let Some(atlas) = loaded else { return };
        self.sheet = thumbnails::Sheet::new(egui, self.titles.len(), &atlas);
        let Some(cells) = thumbnails::cells(self.titles.len(), &atlas) else {
            return;
        };
        self.thumbnail_instances.texture_transformations = Some(cells);
        self.thumbnails.set_instances(&self.thumbnail_instances);
        // Last, because it is what [`Self::drawn_thumbnails`] reads to decide the quads are ready
        // to be drawn at all.
        self.thumbnails.material.texture = Some(Texture2DRef::from_cpu_texture(context, &atlas));
    }

    /// The thumbnail quads, once they have an atlas to sample. Nothing before that: an untextured
    /// [ColorMaterial] would paint its base color flat over every world.
    fn drawn_thumbnails(&self) -> Option<&dyn Object> {
        self.thumbnails
            .material
            .texture
            .is_some()
            .then_some(&self.thumbnails as &dyn Object)
    }

    /// Every world the view is asking more of than the atlas holds, widest on screen first.
    ///
    /// What [`detail`] is driven from, and the reason it needs no view of its own: a world is here
    /// because its node is drawn wider than [`detail::SWITCH_PIXELS`] and is on screen at all, and
    /// it carries the quad it would have been drawn on so that the full picture lands exactly over
    /// its own thumbnail.
    fn magnified(&self, camera: &Camera, viewport: Viewport) -> Vec<detail::Magnified> {
        let billboard = billboard(camera);
        let forward = camera.view_direction();
        let colors = self.thumbnail_instances.colors.as_ref();
        let mut magnified = Vec::new();
        self.graph.visit_nodes(|node| {
            let world = node.index().index();
            let position = world_pos(node.position());
            // Behind the camera a projection says nothing useful, and off the edge of the window
            // there is nothing to sharpen: either way the budget is better spent on a node that
            // is actually being looked at.
            if (position - camera.position()).dot(forward) <= 0.0 {
                return;
            }
            let center = camera.pixel_at_position(position);
            if center.x < 0.0
                || center.y < 0.0
                || center.x > viewport.width as f32
                || center.y > viewport.height as f32
            {
                return;
            }
            let radius = self.node_radii[world];
            let width = drawn_width(camera, radius, position);
            if width < detail::SWITCH_PIXELS {
                return;
            }
            magnified.push((
                width,
                detail::Magnified {
                    world,
                    transformation: Mat4::from_translation(
                        position - forward * (radius * detail::LIFT),
                    ) * billboard
                        * Mat4::from_nonuniform_scale(radius * thumbnails::ASPECT, radius, 1.0),
                    color: colors.map_or(Srgba::WHITE, |colors| colors[world]),
                },
            ));
        });
        magnified.sort_by(|a, b| b.0.total_cmp(&a.0));
        magnified.into_iter().map(|(_, it)| it).collect()
    }
}

/// How wide a node comes out on screen, in physical pixels.
///
/// Measured across the quad through its own centre, so it is the node as drawn rather than a
/// sphere around it — which is what says whether the atlas still holds as much detail as the
/// screen is asking of it. A cell of the atlas is [`thumbnails::CELL`] wide.
fn drawn_width(camera: &Camera, radius: f32, position: Vec3) -> f32 {
    let across = camera.right_direction().normalize() * radius * thumbnails::ASPECT;
    let (left, right) = (
        camera.pixel_at_position(position - across),
        camera.pixel_at_position(position + across),
    );
    (right.x - left.x).hypot(right.y - left.y)
}

/// The names `needle` matches, best first, or the whole list where nothing is asked for.
///
/// Ranked the way [`AppEntities::search`] ranks titles: by where the match falls and then by how
/// much name is left over. Uncut, unlike that one: these lists are short enough to read down, and
/// they are already kept in the order they are worth being read down in. See
/// [`world::Dump::authors`] and [`world::Dump::versions`].
fn matching<'a>(names: impl Iterator<Item = &'a str>, needle: &str) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return (0..names.count()).collect();
    }
    let mut hits: Vec<_> = names
        .enumerate()
        .filter_map(|(at, name)| {
            let found = name.to_lowercase().find(&needle)?;
            Some((found, name.len(), at))
        })
        .collect();
    hits.sort_unstable();
    hits.into_iter().map(|(_, _, at)| at).collect()
}

/// The box each list is searched with.
/// The one control left where the sidebar was, which brings it back.
///
/// In the corner the sidebar's own button was in, so the button reads as the same button facing
/// the other way. An [egui::Area] like the rocker, because with the sidebar gone there is no
/// panel left to hang it inside.
fn sidebar_opener(ui: &mut egui::Ui, sidebar: &mut Sidebar, insets: egui::Margin) {
    egui::Area::new(egui::Id::new("sidebar opener"))
        .anchor(
            egui::Align2::LEFT_TOP,
            [
                (insets.left + PANEL_MARGIN) as f32,
                (insets.top + PANEL_MARGIN) as f32,
            ],
        )
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                if ui
                    .button(ICON_CHEVRON_RIGHT)
                    .on_hover_text("Show the sidebar")
                    .clicked()
                {
                    sidebar.closed = false;
                }
            });
        });
}

fn search_box(ui: &mut egui::Ui, search: &mut String, of: &str) {
    egui::TextEdit::singleline(search)
        .hint_text(of)
        .prefix(ICON_SEARCH)
        .suffix(ICON_CANCEL)
        .show(ui);
}

/// How much of a list is being shown, which only needs saying while it is being cut down.
fn showing(shown: usize, of: usize, kind: &str) -> String {
    match shown == of {
        true => format!("{of} {kind}"),
        false => format!("{shown} of {of} {kind}"),
    }
}

/// How many worlds, said so that one of them does not read as a bug.
fn worlds(count: usize) -> String {
    match count {
        1 => "1 world".to_string(),
        _ => format!("{count} worlds"),
    }
}

/// A colour at a fraction of its own opacity, whatever it was drawn at before.
fn fade(color: egui::Color32, opacity: u8) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    egui::Color32::from_rgba_unmultiplied(r, g, b, (a as u16 * opacity as u16 / 255) as u8)
}

/// Names the selected world and how it sits in the graph: who made it, what it touches and what
/// hangs off it.
fn world_info(ui: &mut egui::Ui, data: &AppEntities, world: usize) -> Option<Highlight> {
    let mut lit = None;
    ui.horizontal(|ui| {
        ui.strong(&data.titles[world]);
        if ui.button(ICON_OPEN_IN_NEW).clicked() {
            open_in_browser(&world::wiki_url(&data.titles[world]));
        }
    });
    ui.horizontal(|ui| {
        ui.label("by");
        if ui
            .link(&data.authors[data.author_of[world]].name)
            .on_hover_text("Show every world by this author")
            .clicked()
        {
            lit = Some(Highlight::Author(data.author_of[world]));
        }
    });
    ui.horizontal(|ui| {
        ui.label(format!("{} connections,", data.degrees[world]));
        if data.descendants[world] > 0
            && ui
                .link(format!("{} descendants", data.descendants[world]))
                .clicked()
        {
            lit = Some(Highlight::Descendants(world));
        } else if data.descendants[world] == 0 {
            ui.label("dead end");
        }
    });
    lit
}

/// Swallows the left-button motion [OrbitControl] would turn the camera with.
///
/// Called after the gesture is resolved, so a drag that belongs to a node has already been taken
/// and only the camera's share is left. Presses and releases are left alone: they still resolve
/// clicks, and neither of them turns anything.
fn lock_rotation(events: &mut [Event]) {
    for event in events {
        if let Event::MouseMotion {
            button: Some(MouseButton::Left),
            handled,
            ..
        } = event
        {
            *handled = true;
        }
    }
}

/// How hard a world of a given radius pushes its neighbours away, with `hub_repulsion` mixing
/// between one mass for everything and a mass proportional to the size.
///
/// At zero every world repels alike, and the sizes are drawn on top of a layout that was spaced
/// for the smallest of them, so the hubs sit over their neighbours. At one the mass is
/// proportional to the radius, which is the setting that scales: a pair's repulsion is capped, so
/// the pair pushes at that ceiling out to the distance where the falloff drops it below, and that
/// distance works out as the square root of the product of the two masses. It is also the setting
/// that pushes hardest, hence the knob.
fn node_mass(radius: f32, hub_repulsion: f32) -> f32 {
    NODE_BASE_MASS * (1.0 + hub_repulsion * (radius / NODE_BASE_RADIUS - 1.0))
}

/// The rotation that stands a node's quad square to the camera.
///
/// A thumbnail is flat, so without this a node would be seen edge-on from half the angles the
/// graph can be turned to. Taken from the camera's own basis, which is why every node
/// in the layout shares the one rotation.
fn billboard(camera: &Camera) -> Mat4 {
    let forward = camera.view_direction();
    // Normalized because the camera's own up need only be the axis the view is kept upright
    // against, not a unit vector square to it. The orbit control refuses to look straight along
    // it, so the cross product cannot collapse.
    let right = camera.right_direction().normalize();
    let up = right.cross(forward);
    Mat4::from_cols(
        right.extend(0.0),
        up.extend(0.0),
        (-forward).extend(0.0),
        Vec4::unit_w(),
    )
}

/// Scatters every node over the spawn volume and stops it dead, restarting the layout.
///
/// Dropping a dimension deforms the layout it was solved in, and picking one back up cannot
/// undo that: a planar graph gives every pair the same z, so the repulsion along that axis is
/// exactly zero and the graph would stay flat forever. Both switches therefore start over,
/// seeding the axes the new mode actually uses.
fn scatter(data: &mut AppEntities) {
    let planar = data.graph.parameters().dimensions == Dimensions::Two;
    let rng = &mut data.rng;
    data.graph.visit_nodes_mut(|mut node| {
        node.set_position([
            rng.next_f32() * SPAWN_EXTENT,
            rng.next_f32() * SPAWN_EXTENT,
            if planar {
                0.0
            } else {
                rng.next_f32() * SPAWN_EXTENT
            },
        ]);
        node.set_velocity([0.0; 3]);
    });
}

/// Builds the backdrop: a lattice of soft blue glows on the background color, in the style of the
/// game's own panoramas.
///
/// Held in linear color, because that is what the shader's own sRGB encoding expects on the way
/// out, and at half precision because eight bits of linear is not enough for glows this dim: the
/// darkest steps land far enough apart once encoded to band, and dithering them only trades the
/// banding for grain across the whole flat backdrop.
fn panorama_tile() -> CpuTexture {
    let cells = PANORAMA_TILE_TEXELS / PANORAMA_CELL_TEXELS;
    let mut data = Vec::with_capacity(PANORAMA_TILE_TEXELS * PANORAMA_TILE_TEXELS);
    for y in 0..PANORAMA_TILE_TEXELS {
        for x in 0..PANORAMA_TILE_TEXELS {
            // Every cell whose glow could reach here, the tile's own neighbours included, so the
            // glows that straddle an edge match up when the tile repeats.
            let mut glow: f32 = 0.0;
            for cell_y in -1..=1 {
                for cell_x in -1..=1 {
                    let center = |along: usize, cell: i32| {
                        (along as i32 / PANORAMA_CELL_TEXELS as i32 + cell)
                            * PANORAMA_CELL_TEXELS as i32
                            + PANORAMA_CELL_TEXELS as i32 / 2
                    };
                    let (center_x, center_y) = (center(x, cell_x), center(y, cell_y));
                    let axis = |at: usize, center: i32| (at as f32 + 0.5 - center as f32).abs();
                    let distance = (axis(x, center_x).powf(PANORAMA_GLOW_NORM)
                        + axis(y, center_y).powf(PANORAMA_GLOW_NORM))
                    .powf(1.0 / PANORAMA_GLOW_NORM);
                    if distance >= PANORAMA_GLOW_RADIUS_TEXELS {
                        continue;
                    }
                    let cell = |center: i32| {
                        (center.div_euclid(PANORAMA_CELL_TEXELS as i32)).rem_euclid(cells as i32)
                            as usize
                    };
                    let peak = PANORAMA_CELL_PEAKS[cell(center_y)][cell(center_x)];
                    glow = glow.max(
                        peak * (1.0 - distance / PANORAMA_GLOW_RADIUS_TEXELS)
                            .powf(PANORAMA_GLOW_FALLOFF),
                    );
                }
            }

            data.push(std::array::from_fn(|channel| {
                let shown = BACKGROUND_COLOR[channel] + glow * PANORAMA_GLOW_COLOR[channel];
                f16::from_f32(srgb_to_linear(shown.min(1.0)))
            }));
        }
    }

    CpuTexture {
        name: "panorama".to_owned(),
        data: TextureData::RgbF16(data),
        width: PANORAMA_TILE_TEXELS as u32,
        height: PANORAMA_TILE_TEXELS as u32,
        ..Default::default()
    }
}

/// The sRGB transfer function, undone. The shader applies it again on the way to the screen, so
/// what a texel means is what it is worth once this has been taken off it.
fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 {
        channel / 12.92
    } else {
        ((channel + 0.055) / 1.055).powf(2.4)
    }
}

/// How the backdrop's tile is laid over the window.
///
/// Scaled so a tile keeps a fixed size in logical pixels whatever the window and the display
/// density, and slid by [`PANORAMA_PARALLAX`] as the camera turns. Where the camera *is* does not
/// enter: a panorama is far enough away that only the direction it is seen from moves it, which is
/// what keeps the backdrop from reading as a plane the graph slides over.
///
/// The offset is taken straight from the view direction rather than from a yaw and a pitch, so that
/// it stays continuous all the way around instead of snapping where an angle wraps.
fn panorama_transform(viewport: Viewport, device_pixel_ratio: f32, view: Vec3) -> Mat3 {
    let tile = PANORAMA_TILE_PIXELS * device_pixel_ratio;
    // Sampling further along an axis pulls the backdrop the opposite way on screen, so turning the
    // camera right drifts the backdrop left, as something in the distance does.
    let drift = vec2(view.x, view.y) * PANORAMA_PARALLAX;
    Mat3::from_translation(drift)
        * Mat3::from_nonuniform_scale(viewport.width as f32 / tile, viewport.height as f32 / tile)
}

/// The part of the window that no system decoration covers, in logical pixels.
///
/// Everywhere but a phone that is the whole of it. A phone keeps a status bar over the top of the
/// window and a navigation bar under the bottom, and the framework reports what is left as the
/// activity's content rect. Only the panel is brought inside it; the graph is left to fill the
/// window, which is what it is drawn behind them for.
/// How far the panel and the rocker have to stand off each edge of the window to clear the
/// system's own furniture, in egui's points. Zero on every side wherever there is no safe area
/// to keep. See [`safe_rect`].
fn safe_insets(viewport: Viewport, device_pixel_ratio: f32) -> egui::Margin {
    let safe = safe_rect(viewport, device_pixel_ratio);
    let width = viewport.width as f32 / device_pixel_ratio;
    let height = viewport.height as f32 / device_pixel_ratio;
    // A margin is measured in whole points and stored in a byte, and no system furniture is
    // anywhere near that deep.
    let inset = |value: f32| value.clamp(0.0, 127.0) as i8;
    egui::Margin {
        left: inset(safe.min.x),
        right: inset(width - safe.max.x),
        top: inset(safe.min.y),
        bottom: inset(height - safe.max.y),
    }
}

fn safe_rect(viewport: Viewport, device_pixel_ratio: f32) -> egui::Rect {
    #[cfg(target_os = "android")]
    if let Some(app) = ANDROID.get() {
        let rect = app.content_rect();
        // Empty until the framework has laid the window out and said where its content goes,
        // which is a frame or two into the first run.
        if rect.right > rect.left && rect.bottom > rect.top {
            return egui::Rect::from_min_max(
                egui::pos2(
                    rect.left as f32 / device_pixel_ratio,
                    rect.top as f32 / device_pixel_ratio,
                ),
                egui::pos2(
                    rect.right as f32 / device_pixel_ratio,
                    rect.bottom as f32 / device_pixel_ratio,
                ),
            );
        }
    }
    egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(
            viewport.width as f32 / device_pixel_ratio,
            viewport.height as f32 / device_pixel_ratio,
        ),
    )
}

/// Raises and drops the on-screen keyboard to follow what egui is doing with the search field.
///
/// egui asks for typing without saying where the typing should come from. On a desk it comes from
/// a keyboard that is already there, and there is nothing to do; on a phone the app has to ask
/// the system to draw one. Only the changes are passed on, because both calls are a trip through
/// the framework and the answer is the same every frame the field stays focused.
fn track_keyboard(wanted: bool, shown: &mut bool) {
    if wanted == *shown {
        return;
    }
    *shown = wanted;
    #[cfg(target_os = "android")]
    if let Some(app) = ANDROID.get() {
        match wanted {
            true => app.show_soft_input(true),
            false => app.hide_soft_input(false),
        }
    }
}

/// Hands a URL to whatever the person browses with.
///
/// Nothing is awaited and nothing is reported back: the browser is another program, and this app
/// has no more to do with the page once it has asked for it. A failure is logged rather than
/// surfaced, because the only thing the person can do about it is open the page themselves.
///
/// Three ways of asking, because the three platforms have nothing in common here: a new tab on
/// the page, a command on a desktop, and an intent on a phone.
fn open_in_browser(url: &str) {
    #[cfg(target_family = "wasm")]
    {
        // A new tab, so the graph the person was reading is still there when they come back.
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
    }
    #[cfg(target_os = "android")]
    {
        use jni::objects::{JObject, JValue};
        use jni::{jni_sig, jni_str};

        // There is no opener to run here. A program says it wants a URL viewed and the system
        // decides who views it, which is a Java call, and JNI is the only way to reach one.
        //
        // The context the activity glue published is the Application's rather than the Activity's
        // (see `android_activity`'s `init_android_main_thread`), and an application context may
        // not start an activity into the task it is asked from. Hence the flag, which gives the
        // browser a task of its own -- which is where a browser opened out of another app belongs
        // in any case, so that leaving it comes back here.
        const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;

        let context = ndk_context::android_context();
        // Safe as long as the glue really did publish a VM and a context, which it does before
        // it ever calls `android_main`, and they outlive the app.
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
        let opened = vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
            let application = unsafe { JObject::from_raw(env, context.context().cast()) };
            let url = env.new_string(url)?;
            let uri = env
                .call_static_method(
                    jni_str!("android/net/Uri"),
                    jni_str!("parse"),
                    jni_sig!("(Ljava/lang/String;)Landroid/net/Uri;"),
                    &[JValue::Object(&url)],
                )?
                .l()?;
            let action = env.new_string("android.intent.action.VIEW")?;
            let intent = env.new_object(
                jni_str!("android/content/Intent"),
                jni_sig!("(Ljava/lang/String;Landroid/net/Uri;)V"),
                &[JValue::Object(&action), JValue::Object(&uri)],
            )?;
            env.call_method(
                &intent,
                jni_str!("addFlags"),
                jni_sig!("(I)Landroid/content/Intent;"),
                &[JValue::Int(FLAG_ACTIVITY_NEW_TASK)],
            )?;
            env.call_method(
                &application,
                jni_str!("startActivity"),
                jni_sig!("(Landroid/content/Intent;)V"),
                &[JValue::Object(&intent)],
            )?;
            Ok(())
        });
        if let Err(error) = opened {
            log::warn!("nothing would open {url}: {error}");
        }
    }
    #[cfg(all(not(target_family = "wasm"), not(target_os = "android")))]
    {
        // The desktop's own opener, whichever this desktop is.
        let (opener, args): (_, &[&str]) = match true {
            _ if cfg!(target_os = "windows") => ("cmd", &["/C", "start", ""]),
            _ if cfg!(target_os = "macos") => ("open", &[]),
            _ => ("xdg-open", &[]),
        };
        if let Err(error) = std::process::Command::new(opener)
            .args(args)
            .arg(url)
            .spawn()
        {
            log::warn!("{opener} could not open {url}: {error}");
        }
    }
}

/// Maps a simulation position (pixel-ish, origin at a corner of the initial cube) into world space.
fn world_pos([x, y, z]: [f32; 3]) -> Vec3 {
    let center = SPAWN_EXTENT * 0.5;
    vec3(x - center, center - y, z - center) * SIM_TO_WORLD
}

/// Inverse of [`world_pos`], for turning a point picked on screen back into a simulation target.
fn sim_pos(world: Vec3) -> Vec3 {
    let center = SPAWN_EXTENT * 0.5;
    let world = world / SIM_TO_WORLD;
    vec3(world.x + center, center - world.y, world.z + center)
}

/// Maps a normalized distance from the origin onto a cyan-green-amber-magenta ramp, so how far a
/// world sits from the start of the game reads off its color. The stops stay bright, to hold up
/// against [BACKGROUND_COLOR].
fn distance_color(distance: f32) -> Srgba {
    const STOPS: [[f32; 3]; 4] = [
        [0.10, 0.90, 1.00],
        [0.35, 1.00, 0.45],
        [1.00, 0.82, 0.25],
        [1.00, 0.35, 0.75],
    ];
    let scaled = distance.clamp(0.0, 1.0) * (STOPS.len() - 1) as f32;
    let stop = (scaled as usize).min(STOPS.len() - 2);
    let blend = scaled - stop as f32;
    let channel =
        |c: usize| ((STOPS[stop][c] + (STOPS[stop + 1][c] - STOPS[stop][c]) * blend) * 255.0) as u8;
    Srgba::new(channel(0), channel(1), channel(2), 255)
}

/// Pushes a color back toward the background, for everything off the highlighted route.
fn dim(color: Srgba) -> Srgba {
    scaled(color, DIMMED_BRIGHTNESS)
}

/// How bright a color reads, against how bright its channels say it is.
///
/// The eye's own weighting of the channels, which is what [`EDGE_LUMINANCE_EVENNESS`] levels the
/// distance ramp by.
fn luminance(color: Srgba) -> f32 {
    (0.2126 * color.r as f32 + 0.7152 * color.g as f32 + 0.0722 * color.b as f32) / 255.0
}

/// A color at a fraction of its brightness, keeping its hue and its alpha.
fn scaled(color: Srgba, brightness: f32) -> Srgba {
    let scale = |channel: u8| (channel as f32 * brightness) as u8;
    Srgba::new(scale(color.r), scale(color.g), scale(color.b), color.a)
}

/// Per connection, the color it carries when nothing is selected.
///
/// The depth ramp lives on the connections rather than on the worlds, which carry pictures: a
/// connection wears the color of the world it is walked *from*, so following a line outward from
/// the origin walks the ramp. Its brightness says how much of the game lies through it — full for
/// a connection into a world the whole map is behind, [`EDGE_LEAF_BRIGHTNESS`] for one into a dead
/// end — which is what makes the trunk of the route tree stand out of its twigs.
///
/// Read through a logarithm for the same reason the node sizes are: the counts span the whole
/// graph, and on a straight scale everything but a handful of hubs would come out at the floor.
///
/// Which end is walked from is the canonical routes' answer where they have one. A connection that
/// is nobody's route home — a shortcut across the tree — is taken as walked from its shallower
/// end, which is the direction a player meets it in anyway.
fn edge_colors(
    graph: &ForceGraph,
    routes: &world::Routes,
    descendants: &[u32],
    depth_colors: &[Srgba],
) -> Vec<Srgba> {
    // Against the busiest world there is, so the brightest connection in the graph is the one the
    // whole game hangs off rather than one at an arbitrary count.
    let busiest = (1.0 + descendants.iter().copied().max().unwrap_or(0) as f32).ln();
    // The apparent brightness the ramp is levelled down to: the dimmest its own stops reach, so
    // the correction only ever darkens and no channel has to clip to make room. Read off the ramp
    // by sampling it rather than restated here, because it is [`distance_color`]'s to define.
    const SAMPLES: u32 = 64;
    let floor = (0..=SAMPLES)
        .map(|step| luminance(distance_color(step as f32 / SAMPLES as f32)))
        .fold(f32::INFINITY, f32::min);
    let mut colors = Vec::with_capacity(graph.edge_count());
    graph.visit_edges(|a, b, _| {
        let (a, b) = (a.index().index(), b.index().index());
        // Unreachable worlds are deeper than any depth rather than shallower than every one, which
        // is what [Option]'s own order would make of them.
        let depth = |world: usize| routes.depth[world].unwrap_or(u32::MAX);
        let from = match (routes.parents[b], routes.parents[a]) {
            (Some(parent), _) if parent == a => a,
            (_, Some(parent)) if parent == b => b,
            _ if depth(a) <= depth(b) => a,
            _ => b,
        };
        let to = if from == a { b } else { a };
        let reach = (1.0 + descendants[to] as f32).ln() / busiest.max(f32::MIN_POSITIVE);
        // Never above 1: a color already dimmer than the ramp's own floor — [`UNREACHED_COLOR`],
        // which is off the ramp entirely — is left alone rather than lifted onto it.
        let even = (floor / luminance(depth_colors[from]).max(f32::MIN_POSITIVE)).min(1.0);
        colors.push(scaled(
            depth_colors[from],
            (EDGE_LEAF_BRIGHTNESS + (1.0 - EDGE_LEAF_BRIGHTNESS) * reach)
                * (1.0 - EDGE_LUMINANCE_EVENNESS * (1.0 - even)),
        ));
    });
    colors
}
