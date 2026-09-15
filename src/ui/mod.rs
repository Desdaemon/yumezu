//! Everything drawn over the graph rather than in it: the panel and its sidebar, the rocker, the
//! right-click menu, the hover tooltip, and the frame that stands in for all of it while the dump
//! is still on its way. See [`Overlay`] and [`Panel`].

mod graph_overlay;
mod selection;
mod settings;
mod sidebar;
mod tile;

use sidebar::{matching, sidebar_opener, worlds};
use tile::{Tile, release_held_hover};

use super::*;

/// per-frame number swings far too much to read from a moving graph.
const FRAME_WINDOW_MS: f32 = 500.0;

/// How many of a release's worlds the catalog shows it by, and how tall it draws those pictures,
/// in egui's points.
const CATALOG_THUMBNAILS: usize = 4;
const CATALOG_THUMBNAIL_HEIGHT: f32 = 34.0;

/// Gap the panel and the rocker keep off the edges of the safe area. See [`safe_insets`].
pub(super) const PANEL_MARGIN: i8 = 12;
/// Everything this app does with a YNOproject account, offered beside the sign-in: a promise about
/// someone's account is worth no more than the code that keeps it.
const YNO_SOURCE: &str = "https://github.com/Desdaemon/yumezu/blob/main/src/yno.rs";

/// Width of the sidebar before anyone drags its edge.
const SIDEBAR_WIDTH: f32 = 280.0;
/// The graph runs underneath the sidebar rather than stopping at its edge, so an opaque one would
/// hide what was just selected. Still opaque enough to keep text legible over a bright graph.
const SIDEBAR_OPACITY: u8 = 200;
/// Width of the right-click menu and the hover tooltip, in egui's points. Both carry a world's
/// title, and a long one would otherwise stretch them across the window.
const POPUP_WIDTH: f32 = 240.0;
const UI_SCALE: &str = "ui-scale";

/// The far ends are a phone held at arm's length and a desk monitor read from close up. The middle
/// is the size the system itself says a point is, which is already right for most screens.
const UI_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.6..=2.0;
const UI_SCALE_DEFAULT: f32 = 1.0;

/// How far below and right of the cursor the hover tooltip sits, in egui's points, so that it
/// names the world under the pointer rather than covering it.
const TOOLTIP_OFFSET: f32 = 16.0;

// The gutter a walk's line is drawn down, and the line and its stops within it. Drawn as a transit
// map draws a route: the reader is following one line through named stops, in order.
const RAIL_WIDTH: f32 = 18.0;
const RAIL_LINE: f32 = 2.0;
const RAIL_STOP: f32 = 3.0;
const RAIL_END: f32 = 5.0;

/// What a list row is given on top of the height a pointer needs, once the app has seen a finger.
/// Enough that a thumb is not aiming at a line of text, and no more: a list of a few hundred
/// worlds loses what a button-sized row gains. See [`Tile`].
const TILE_PADDING_TOUCH: f32 = 6.0;
/// How long a finger rests on something before it is read as wanting what a pointer gets by
/// hovering. Shorter than egui's own wait, a finger being aimed where a pointer may only have come
/// to rest, and well inside the 800 ms that makes a press a right-click.
const HOVER_HOLD_SECONDS: f64 = 0.4;
/// How far a finger may wander over the hold and still be a hold on the row it started on.
///
/// egui restarts [`HOVER_HOLD_SECONDS`] at every twitch of the pointer, which a mouse left alone
/// satisfies and a finger never does, so the hold is timed here against this much slack instead.
/// See [`Tile::hover`].
const HOVER_HOLD_SLACK: f32 = 16.0;
/// How far a row's hover text stands off the row on a touch screen: clear of the fingertip that
/// called it up, and no further. It stands over the finger rather than the row, so it need not
/// clear the whole hand. See [`Tile::hover`].
const HOVER_FINGER_GAP: f32 = 16.0;
/// Above the row and below it only where there is no room above: a phone is held from below, so
/// what is under the row is under the hand.
const HOVER_ALIGNS: [egui::RectAlign; 1] = [egui::RectAlign::BOTTOM];
/// Where [`HeldHover`] is kept.
const HELD_HOVER_ID: &str = "held hover";

// Fixed rather than taken from the title, which names whichever world is being walked to: egui
// files a window's position under its id.
const DIRECTIONS_ID: &str = "directions";
// Enough of a route to read without covering the graph it is walked over.
const DIRECTIONS_SIZE: [f32; 2] = [260.0, 300.0];

/// The rocker is the one control with no words on it and the one a thumb goes for. A multiplier
/// rather than a size, so it follows the UI scale.
const ROCKER_SCALE: f32 = 3.0;

/// Frames counted over a window.
///
/// Counted rather than averaged frame by frame: an exponential average has to weight each sample
/// by how long it stood for, which gives one 100 ms stall a fifth of a 500 ms window. Counting
/// answers what is wanted, and is the arithmetic [`FrameStats`] reports.
#[derive(Default)]
struct Counted {
    frames: u32,
    /// Wall milliseconds the counted frames spanned.
    window: f32,
    /// The last full window's answer, kept because the next one is not ready yet.
    rate: f32,
}

impl Counted {
    fn frame(&mut self, elapsed: f32) {
        self.frames += 1;
        self.window += elapsed;
        if self.window < FRAME_WINDOW_MS {
            return;
        }
        self.rate = 1e3 * self.frames as f32 / self.window;
        *self = Self {
            rate: self.rate,
            ..Default::default()
        };
    }

    fn rate(&self) -> f32 {
        self.rate
    }
}

pub(super) struct Overlay {
    pub(super) gui: gui::Gui,
    /// Frames counted over the [`FRAME_WINDOW_MS`] so far. See [`Overlay::timed`].
    counted: Counted,
    sidebar: Sidebar,
    /// Whether the on-screen keyboard was last wanted. See [`track_keyboard`].
    pub(super) keyboard: bool,
    guide: guide::Guide,
    /// The offer of the phone build, made to the phones reading the page.
    download: download::Offer,
    maps: map::Maps,
    pub(super) japanese: japanese::Japanese,
    /// What the person has scaled the interface by, on top of whatever the system already says a
    /// point is worth. Multiplied into the overlay's pixel ratio and nothing else: the graph is a
    /// place rather than an interface.
    pub(super) ui_scale: f32,
    /// The scale the store was last written with, so a drag writes once at the end of itself
    /// rather than once a frame all the way along it.
    ui_scale_stored: f32,
    /// The layout choices the store was last written with, for the same reason.
    layout: Layout,
    /// Whether the person has chosen to have the edges smoothed, which is not always whether
    /// they are being smoothed. See [`Overlay::antialias_running`].
    antialias: bool,
    /// What the running surface was built for. Kept because a surface is given its samples
    /// once, as it is built, so this is a choice only the next start can honour.
    antialias_running: bool,
    /// Whether the view leans onto the world a row is pointing at. See [`leaning_remembered`].
    pub(super) leaning: bool,
}

/// What the sidebar keeps between frames: held apart from [`Panel`], which is this frame's
/// decisions, because these are the person's and outlive the frame that made them.
#[derive(Default)]
struct Sidebar {
    /// This way round so that the default is open.
    closed: bool,
    tab: Tab,
    /// What has been typed into each tab's own box. Matched every frame rather than cached, a scan
    /// of a few thousand titles being cheaper than the frame it happens in.
    ///
    /// One box per tab rather than one shared: they search different things, and a world's name
    /// carried over into the authors would only ever come up empty.
    worlds: String,
    authors: String,
    versions: String,
    /// What has been typed into the sign-in fields, until the button is pressed. Neither outlives
    /// the run, and the password is never written down. See [`Panel::yno`].
    yno_user: String,
    yno_password: String,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// The graph in front of the person: what is selected, and what to look at next.
    #[default]
    Worlds,
    Authors,
    Versions,
    Settings,
}

/// What the panel was left at, once it is built and the frame's clicks have landed on it.
struct Panel {
    dimensions: Dimensions,
    hub_repulsion: f32,
    link_reach: f32,
    ui_scale: f32,
    layered: bool,
    /// What the smoothing was left set to, and what the running surface is actually doing, so the
    /// tab can say when the two have parted. See [`Overlay::antialias_running`].
    antialias: bool,
    antialias_running: bool,
    /// What the switch was left set to. See [`leaning_remembered`].
    leaning: bool,
    /// A world picked out of one of the lists, to be routed to.
    chosen: Option<usize>,
    /// A world a list row is pointing at, to be brightened where it sits in the graph. At most one:
    /// the pointer is over one row or none.
    pointed: Option<usize>,
    /// What the menu, a link, or the rocker chose to light, if any of them chose at all. The inner
    /// `None` is the rocker's off position, which lights nothing.
    lit: Option<Option<Highlight>>,
    /// Whether the menu was acted on and should close.
    menu_taken: bool,
    /// Whether egui is working a widget and the frame's pointer events are the panel's.
    wants_pointer: bool,
    /// Whether the settings tab called for the controls to be named again.
    guide: bool,
    /// Whether the framing button was pressed, calling for a route to be framed the other way.
    refit: bool,
    /// The way among a set of directions that was picked to be drawn instead.
    way: Option<usize>,
    /// The world whose maps were called for. A second press on the same world closes them again:
    /// see [`map::Maps::toggle`].
    mapped: Option<usize>,
    /// See [`yno::Account::pretend`].
    revealed: Option<usize>,
    /// Applied after the frame, so a frame is drawn in one language throughout.
    language: Option<i18n::Language>,
}

/// Everything the panel reads, walked out of [`AppEntities`] before it is built.
struct PanelData<'a> {
    data: &'a AppEntities,
    /// See [`Counted::rate`].
    fps: f32,
    /// The route home from what is lit, origin last.
    route: Vec<usize>,
    /// The worlds worth naming among the descendants of what is lit.
    notable: Vec<usize>,
    /// The worlds a highlight names as a plain list: an author's work, a release's additions, a
    /// whole layer. Empty for the two highlights the panel reads some other way.
    listed: Vec<usize>,
    candidates: Vec<usize>,
    /// What the open tab's box matches, as indices into [`AppEntities::authors`] or
    /// [`AppEntities::versions`]. Empty for the tab that is not open, and the whole list for a box
    /// with nothing in it.
    authors: Vec<usize>,
    versions: Vec<usize>,
    selected: Option<Highlight>,
    /// The world a right-click opened a menu over, and where to draw it in egui's points.
    menu: Option<(usize, egui::Pos2)>,
    /// The world the pointer is over, and where it is in egui's points. `None` whenever nothing is
    /// hovered, and on a touch screen throughout.
    hovered: Option<(usize, egui::Pos2)>,
}

pub(super) struct ContextMenu {
    pub(super) world: usize,
    /// Where the click landed, in physical pixels, which is where the menu is drawn.
    pub(super) at: PhysicalPoint,
}

impl Overlay {
    /// Carries what the last frame took into the rate the panel reads.
    fn timed(&mut self, elapsed: f32) {
        self.counted.frame(elapsed);
    }

    pub(super) fn new(
        event_loop: &winit::event_loop::ActiveEventLoop,
        window: &Window,
        context: &Context,
    ) -> Self {
        let gui = gui::Gui::new(event_loop, window, context);
        egui_material_icons::initialize(gui.context());
        let scale = remembered_scale();
        let remembered = antialias_remembered();
        Self {
            gui,
            counted: Counted::default(),
            sidebar: Sidebar::default(),
            keyboard: false,
            guide: guide::Guide::new(),
            download: download::Offer::new(),
            maps: map::Maps::new(),
            japanese: japanese::Japanese::new(),
            ui_scale: scale,
            ui_scale_stored: scale,
            layout: Layout::remembered(),
            antialias: remembered,
            antialias_running: remembered,
            leaning: leaning_remembered(),
        }
    }

    /// Builds this frame's panel and consumes the events that land on it.
    ///
    /// The closure can only borrow `data` immutably, so what the panel reads is walked out into
    /// [`PanelData`] first and what it decides is left on the [`Panel`] for [`Overlay::run`].
    fn panel(
        &mut self,
        window: &Window,
        frame_input: &mut FrameInput,
        data: &AppEntities,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        fading: Option<(String, f32)>,
    ) -> Panel {
        // egui's zoom factor multiplies whatever the window says a point is worth, so the panel is
        // laid out and read at the product and the graph at the window's ratio alone.
        self.gui.context().set_zoom_factor(self.ui_scale);
        // Every hover text in the app that is not a row's -- those time their own hold, see
        // [`held_long_enough`]. On a touch screen the wait is shortened and the demand that the
        // pointer be still is dropped, a hand being unable to meet it.
        if self.gui.context().input(|input| input.has_touch_screen()) {
            self.gui.context().all_styles_mut(|style| {
                style.interaction.tooltip_delay = HOVER_HOLD_SECONDS as f32;
                style.interaction.show_tooltips_only_when_still = false;
            });
            release_held_hover(self.gui.context());
        }
        let ratio = frame_input.device_pixel_ratio * self.ui_scale;
        let read = PanelData::new(data, self.counted.rate(), &self.sidebar, frame_input, ratio);
        let parameters = data.graph.parameters();
        // Read here and written back after the panel: `parameters_mut` wakes the layout, so
        // touching it every frame would keep the graph from ever settling.
        let mut panel = Panel {
            dimensions: parameters.dimensions,
            hub_repulsion: data.hub_repulsion,
            link_reach: data.link_reach,
            ui_scale: self.ui_scale,
            layered: parameters.dag_level_distance.is_some(),
            antialias: self.antialias,
            antialias_running: self.antialias_running,
            leaning: self.leaning,
            chosen: None,
            pointed: None,
            lit: None,
            menu_taken: false,
            wants_pointer: false,
            guide: false,
            refit: false,
            way: None,
            mapped: None,
            revealed: None,
            language: None,
        };
        // Bound out of `self` so the closure borrows these fields alone, leaving `self.gui` free
        // for the call it is passed to.
        let sidebar = &mut self.sidebar;
        let guide = &mut self.guide;
        let download = &mut self.download;
        let maps = &mut self.maps;
        let insets = safe_insets(frame_input.viewport, ratio);
        self.gui.run(window, |ui| {
            let style = ui.style();
            // Full height, with the safe area kept by standing the contents off the panel's own
            // edges: stopping short of the status bar would leave a stripe above it.
            let frame = egui::Frame::side_top_panel(style)
                .inner_margin(egui::Margin {
                    left: insets.left + PANEL_MARGIN,
                    right: PANEL_MARGIN,
                    top: insets.top + PANEL_MARGIN,
                    bottom: insets.bottom + PANEL_MARGIN,
                })
                // The style's own panel colour, only thinned. See [`SIDEBAR_OPACITY`].
                .fill(fade(style.visuals.panel_fill, SIDEBAR_OPACITY));
            match sidebar.closed {
                false => {
                    egui::Panel::left("yumezu")
                        .frame(frame)
                        .default_size(SIDEBAR_WIDTH)
                        .show(ui, |ui| panel.window(ui, &read, sidebar, account, dump));
                }
                true => sidebar_opener(ui, sidebar, insets),
            }
            panel.rocker(ui, &read, insets);
            panel.directions(ui, &read);
            panel.menu(ui, &read);
            panel.tooltip(ui, &read);
            // Read within the frame that set it, so the window is on screen the moment the button
            // is let go of rather than the frame after.
            if let Some(world) = panel.mapped {
                maps.toggle(world, data.titles[world].show(), &data.maps[world]);
            }
            maps.show(ui.ctx(), insets);
            download.show(ui.ctx(), insets);
            // Last, so it is drawn over everything it explains. A frame late when the settings tab
            // has just called for it, which is a frame nobody sees.
            if panel.guide {
                guide.reopen();
            }
            guide.show(ui.ctx());
            // Last of all, and over the panel as much as the graph: what is fading stood in for
            // the whole interface, so the whole interface comes out from under it.
            if let Some((says, opacity)) = &fading {
                loading_frame(ui.ctx(), says, None, *opacity);
            }
        });
        // Read from egui after the frame, the only point at which it knows what was laid out under
        // the pointer.
        panel.wants_pointer = self.gui.context().egui_wants_pointer_input()
            || self.gui.context().egui_wants_keyboard_input();
        // Also after the frame: only now does egui know whether anything it drew took the focus
        // typing would go to.
        track_keyboard(
            self.gui.context().egui_wants_keyboard_input(),
            &mut self.keyboard,
        );
        panel
    }

    /// Runs one frame of the panel and applies what it was left at. Returns whether the dimension
    /// changed, which is the one switch the caller has to follow up on.
    pub(super) fn run(
        &mut self,
        window: &Window,
        frame_input: &mut FrameInput,
        data: &mut AppEntities,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        fading: Option<(String, f32)>,
    ) -> bool {
        // The frame before this one, the panel being drawn inside the frame it reports.
        self.timed(frame_input.elapsed_time as f32);

        let panel = self.panel(window, frame_input, data, account, dump, fading);
        if let Some(language) = panel.language {
            i18n::speak(language);
        }
        // Cloned because the face is installed on the context while the field installing it is
        // borrowed.
        let context = self.gui.context().clone();
        self.japanese.serve(&context);
        // Set by the panel too, and by the selections applied out here.
        let mut menu_taken = panel.menu_taken;
        if let Some(selection) = panel.lit {
            data.select(selection);
            menu_taken = true;
        }
        if menu_taken {
            data.menu = None;
        }
        // Always the route: a world named in a list is picked to be gone to rather than opened out.
        if let Some(world) = panel.chosen {
            data.select(Some(Highlight::Route(world)));
        }
        // Every frame, cleared included: a row stops pointing by no longer being hovered, which is
        // a frame that says nothing rather than one that says to stop.
        data.pointed = panel.pointed;
        // Out here because the account is what a pretence is laid on and the panel is handed only
        // what it draws. The English name, that being what the two sides of the join agree on.
        if let Some(world) = panel.revealed {
            account.pretend(data.titles[world].en.clone());
        }
        // Framing again rather than only from here on: the button is pressed to be taken there
        // now, whether or not the camera had already arrived.
        if panel.refit {
            data.frame_route = !data.frame_route;
            data.framing = true;
        }
        if let Some(way) = panel.way {
            data.take_way(way);
        }
        // three-d's own `GUI` surrenders the pointer only while egui is actively working a widget,
        // so a scroll over the panel would reach the camera as a zoom. Whatever egui wants the
        // pointer for is the panel's.
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

        if panel.antialias != self.antialias {
            self.antialias = panel.antialias;
            antialias_remember(self.antialias);
        }
        if panel.leaning != self.leaning {
            self.leaning = panel.leaning;
            leaning_remember(self.leaning);
        }
        // Followed live, so the panel resizes under the hand, but written through only once the
        // hand is off it: a store is not something to write every frame of a drag.
        self.ui_scale = panel.ui_scale;
        if self.ui_scale != self.ui_scale_stored && !self.gui.context().egui_is_using_pointer() {
            self.ui_scale_stored = self.ui_scale;
            store::write(UI_SCALE, Some(&self.ui_scale.to_string()));
        }

        // Ahead of the early return below: two of the four change no simulation parameter and
        // would never be written down otherwise. Held off while a slider is under the hand, for
        // the reason the UI scale is.
        let layout = Layout {
            dimensions: panel.dimensions,
            layered: panel.layered,
            hub_repulsion: panel.hub_repulsion,
            link_reach: panel.link_reach,
        };
        // The two knobs reach the simulation ahead of that same return: neither rearranges the
        // layout the way a mode switch does, and both are dragged, so each is followed live.
        if panel.hub_repulsion != data.hub_repulsion {
            data.hub_repulsion = panel.hub_repulsion;
            data.apply_hub_push();
        }
        if panel.link_reach != data.link_reach {
            data.link_reach = panel.link_reach;
            data.graph.parameters_mut().link_distance_max = layout.parameters().link_distance_max;
        }
        if layout != self.layout && !self.gui.context().egui_is_using_pointer() {
            self.layout = layout;
            layout.remember();
        }

        let was = data.graph.parameters();
        if panel.dimensions == was.dimensions && panel.layered == was.dag_level_distance.is_some() {
            return false;
        }
        let parameters = data.graph.parameters_mut();
        let reseed = panel.dimensions != parameters.dimensions;
        parameters.dimensions = panel.dimensions;
        let solve = layout.parameters();
        parameters.dag_level_distance = solve.dag_level_distance;
        parameters.dag_level_slack = solve.dag_level_slack;
        parameters.force_charge = solve.force_charge;
        parameters.link_distance_max = solve.link_distance_max;
        // A switch rearranges the whole layout rather than nudging it, so it gets a full settling
        // window instead of what a settled graph has left.
        data.graph.revive();
        reseed
    }
}

impl<'a> PanelData<'a> {
    /// Walks out of `data` everything the panel will read, the closure that builds it being able
    /// to borrow `data` only immutably.
    fn new(
        data: &'a AppEntities,
        fps: f32,
        sidebar: &Sidebar,
        frame_input: &FrameInput,
        ratio: f32,
    ) -> Self {
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
            // Only the open tab: matching a few hundred names for a list nobody is looking at is
            // work every frame would pay for.
            authors: match sidebar.tab {
                Tab::Authors => matching(
                    data.authors.iter().map(|by| by.name.names()),
                    &sidebar.authors,
                ),
                _ => Vec::new(),
            },
            versions: match sidebar.tab {
                Tab::Versions => matching(
                    data.versions
                        .iter()
                        .map(|version| std::iter::once(version.name.as_str())),
                    &sidebar.versions,
                ),
                _ => Vec::new(),
            },
            selected: data.selected,
            menu: data
                .menu
                .as_ref()
                .map(|menu| (menu.world, into_points(menu.at, frame_input, ratio))),
            // Settled after the panel ran last frame, the pointer only being resolved once the
            // panel has taken its share of the events. A frame behind, which nobody sees.
            hovered: data
                .hover
                .zip(data.cursor)
                .map(|(world, cursor)| (world, into_points(cursor, frame_input, ratio))),
        }
    }
}

/// A value written by a version that allowed a different range is brought back inside this one
/// rather than thrown away.
fn remembered_scale() -> f32 {
    store::read(UI_SCALE)
        .and_then(|scale| scale.parse().ok())
        .map_or(UI_SCALE_DEFAULT, |scale: f32| {
            scale.clamp(*UI_SCALE_RANGE.start(), *UI_SCALE_RANGE.end())
        })
}

/// A window position in physical pixels, counted from the bottom, into egui's points counted from
/// the top: the same conversion the integration puts every pointer event through.
fn into_points(at: PhysicalPoint, frame_input: &FrameInput, ratio: f32) -> egui::Pos2 {
    egui::pos2(
        at.x / ratio,
        (frame_input.viewport.height as f32 - at.y) / ratio,
    )
}

/// How wide a node comes out on screen, in physical pixels: across the quad through its own centre,
/// so it is the node as drawn rather than a sphere around it.
pub(super) fn drawn_width(
    camera: &Camera,
    across: Vec3,
    radius: f32,
    position: Vec3,
    center: PhysicalPoint,
) -> f32 {
    // Half the quad, doubled, rather than both edges projected: `across` is square to the view
    // direction, so the projection is linear between the edges. With `center` already in hand,
    // that is one projection per node instead of three.
    let edge = camera.pixel_at_position(position + across * (radius * thumbnails::ASPECT));
    2.0 * (edge.x - center.x).hypot(edge.y - center.y)
}

/// Takes the page's own loading line off the document, the once.
///
/// The wasm is a few megabytes and the canvas is black until it has come down and drawn, so
/// `index.html` carries the same line the loading frame does. Called after every frame and does
/// nothing after the first.
#[cfg(target_family = "wasm")]
pub(super) fn take_placeholder() {
    static TAKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if TAKEN.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Some(placeholder) = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id("loading"))
    {
        placeholder.remove();
    }
}

/// Drawn at `opacity` rather than always whole: [`App::draw_loading`] draws it whole over a cleared
/// screen, and [`Overlay::panel`] draws what is left over the graph that replaced it. See
/// [`App::veil`].
///
/// The veil goes on the foreground layer and the words a layer above. Two orders rather than two
/// layers of one: a layer painted through [`egui::Context::layer_painter`] is drawn last within
/// its order, which put the veil over the words.
pub(super) fn loading_frame(ctx: &egui::Context, says: &str, failed: Option<&str>, opacity: f32) {
    let opacity = opacity.clamp(0.0, 1.0);
    let channel = |channel: usize| (BACKGROUND_COLOR[channel] * 255.0) as u8;
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("loading-veil"),
    ))
    .rect_filled(
        // The whole window, notches and status bar included: the graph is drawn under those too,
        // and a veil stopping at the safe area would uncover a stripe of it early.
        ctx.viewport_rect(),
        egui::CornerRadius::ZERO,
        egui::Color32::from_rgba_unmultiplied(
            channel(0),
            channel(1),
            channel(2),
            (opacity * 255.0) as u8,
        ),
    );
    egui::Area::new(egui::Id::new("loading"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            ui.horizontal(|ui| {
                // Whatever the frame says, it says it about something still going on: a run that
                // could not reach the server is about to try again.
                ui.add(egui::Spinner::new());
                let line = ui.label(says);
                // Logged in full; on screen it is offered only to whoever goes looking, there
                // being nothing a reader can do with a URL and a status code.
                if let Some(error) = failed {
                    line.on_hover_text(error);
                }
            });
        });
}

/// A colour at a fraction of its own opacity.
fn fade(color: egui::Color32, opacity: u8) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    egui::Color32::from_rgba_unmultiplied(r, g, b, (a as u16 * opacity as u16 / 255) as u8)
}

/// How far the panel and the rocker stand off each edge of the window to clear the system's own
/// furniture, in egui's points. Zero where there is no safe area. See [`safe_rect`].
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

/// The part of the window no system decoration covers, in logical pixels, which everywhere but a
/// phone is the whole of it. On a phone the framework reports what the status and navigation bars
/// leave as the activity's content rect.
///
/// Only the panel is brought inside it; the graph is left to fill the window, which is what it is
/// drawn behind them for.
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

/// egui calls for typing without saying where it should come from, and on a phone the app has to
/// ask the system for a keyboard. Only the changes are passed on, each call being a trip through
/// the framework.
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

/// Hands a URL to whatever the person browses with: a new tab on the page, a command on a desktop,
/// an intent on a phone.
///
/// Nothing is awaited and a failure is only logged, the only thing the person can do about it
/// being to open the page themselves.
pub(super) fn open_in_browser(url: &str) {
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

        // There is no opener to run here: a program says it wants a URL viewed and the system
        // decides who views it, which is a Java call and so JNI.
        //
        // The context the activity glue published is the Application's, and an application context
        // may not start an activity into the task it is called from. Hence the flag, which gives
        // the browser a task of its own, so leaving it comes back here.
        const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;

        let context = ndk_context::android_context();
        // SAFETY: the glue publishes the VM and the context before it ever calls `android_main`,
        // and both outlive the app.
        #[allow(unsafe_code)]
        let vm = unsafe { jni::JavaVM::from_raw(context.vm().cast()) };
        let opened = vm.attach_current_thread(|env| -> Result<(), jni::errors::Error> {
            #[allow(unsafe_code)]
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
