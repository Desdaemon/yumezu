//! Everything overlaid on the graph: the browse and selection windows, the rocker, the right-click
//! menu, and the message box. See [`Overlay`] and [`Panel`].

mod browse;
mod graph_overlay;
mod selection;
mod settings;
mod skin;
mod tile;

use browse::worlds;
use skin::Skin;
use tile::{Tile, release_held_hover};

use super::*;

/// The span frames are counted over for the rate; a per-frame number swings too much to read.
const FRAME_WINDOW_MS: f32 = 500.0;

/// How many of a release's world pictures the catalog shows, and their height in egui's points.
const CATALOG_THUMBNAILS: usize = 4;
const CATALOG_THUMBNAIL_HEIGHT: f32 = 34.0;

/// Gap the panel and the rocker keep off the edges of the safe area. See [`safe_insets`].
pub(super) const PANEL_MARGIN: i8 = 12;
/// The source of everything this app does with a YNOproject account, linked beside the sign-in.
const YNO_SOURCE: &str = "https://github.com/Desdaemon/yumezu/blob/main/src/yno.rs";

/// Widths of the windows down either side of a wide window, in egui's points.
const BROWSE_WIDTH: f32 = 300.0;
const SELECTION_WIDTH: f32 = 320.0;
/// Narrower than this, in egui's points, and the windows take turns over a tab bar.
const NARROW_BELOW: f32 = 640.0;
/// The message box between the side windows: no wider than a line reads well, and never narrower
/// than a world's title.
const MESSAGE_WIDTH: f32 = 560.0;
const MESSAGE_MIN_WIDTH: f32 = 240.0;
/// What the windows fade to while the pointer works the graph behind them.
const FADED_OPACITY: f32 = 0.4;
const WINDOWS_SHOWN_ID: &str = "windows shown";
/// How much of a phone's safe area the window over the tab bar may take.
const SHEET_SHARE: f32 = 0.4;
/// Width of the right-click menu, in egui's points, capping how far a long title stretches.
const POPUP_WIDTH: f32 = 240.0;
const SYSTEM_THEME_ID: &str = "system theme";
/// Four themes across, which a phone held upright fits with room to spare.
const SYSTEM_THEME_WIDTH: f32 = 280.0;
/// The smallest side of a theme's square, grown so the squares that fit fill the row.
const SYSTEM_THEME_SIDE: f32 = 64.0;
const SYSTEM_THEME_PREVIEW: egui::Vec2 = egui::vec2(40.0, 40.0);
const SYSTEM_THEME_HEIGHT_SHARE: f32 = 0.6;
const UI_SCALE: &str = "ui-scale";

/// The far ends are a phone held at arm's length and a desk monitor read from close up. The middle
/// is the size the system itself says a point is, which is already right for most screens.
const UI_SCALE_RANGE: std::ops::RangeInclusive<f32> = 0.6..=2.0;
const UI_SCALE_DEFAULT: f32 = 1.0;

// The gutter the route's line runs down, drawn as a transit map draws a route.
const RAIL_WIDTH: f32 = 18.0;
const RAIL_LINE: f32 = 2.0;
const RAIL_STOP: f32 = 3.0;
const RAIL_END: f32 = 5.0;

/// Extra list row height on a touch screen: room for a thumb, small enough for long lists.
/// See [`Tile`].
const TILE_PADDING_TOUCH: f32 = 6.0;
/// How long a finger rests before the press reads as a hover; must stay under the 800 ms that
/// makes a press a right-click.
const HOVER_HOLD_SECONDS: f64 = 0.4;
/// How far a finger may wander and still hold the same row. See [`Tile::hover`].
const HOVER_HOLD_SLACK: f32 = 16.0;
/// How far a row's hover text stands off the fingertip on a touch screen.
const HOVER_FINGER_GAP: f32 = 16.0;
/// Below the row only when there is no room above: a phone is held from below, so the hand covers
/// what is under the row.
const HOVER_ALIGNS: [egui::RectAlign; 1] = [egui::RectAlign::BOTTOM];
const HELD_HOVER_ID: &str = "held hover";

// Fixed: egui keys window position by id, and the title changes per destination.
const DIRECTIONS_ID: &str = "directions";
const DIRECTIONS_SIZE: [f32; 2] = [260.0, 300.0];

/// The rocker is the one wordless control and the one a thumb goes for. A multiplier, so the
/// rocker follows the UI scale.
const ROCKER_SCALE: f32 = 3.0;

/// Frames counted over a window, the arithmetic [`FrameStats`] reports.
#[derive(Default)]
struct Counted {
    frames: u32,
    /// Wall milliseconds the counted frames spanned.
    window: f32,
    /// The rate over the last full window, shown until the next one completes.
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
    browse: Browse,
    /// Whether the on-screen keyboard was last wanted. See [`track_keyboard`].
    pub(super) keyboard: bool,
    guide: guide::Guide,
    /// The offer of the phone build, shown to phones running the web build.
    download: download::Offer,
    maps: map::Maps,
    pub(super) japanese: japanese::Japanese,
    /// The user's UI scale on top of the system's point size. Multiplied into the overlay's pixel
    /// ratio only; the graph keeps the system's.
    pub(super) ui_scale: f32,
    /// The scale last written to the store, so a drag writes once, when the drag ends.
    ui_scale_stored: f32,
    /// The layout choices the store was last written with, for the same reason.
    layout: Layout,
    /// The user's antialiasing choice, which can differ from the running surface's. See
    /// [`Overlay::antialias_running`].
    antialias: bool,
    /// Whether the running surface was built antialiased; samples are fixed at creation, so a
    /// change waits for the next start.
    antialias_running: bool,
    /// Whether the camera follows the world under a hovered row. See [`load_row_follow_setting`].
    pub(super) row_follow: bool,
    skin: Skin,
}

/// What the browse window keeps between frames, apart from [`Panel`], which holds one frame's
/// decisions.
#[derive(Default)]
struct Browse {
    tab: Tab,
    /// Whether the open tab's contents are folded away, which a new selection does to make room.
    collapsed: bool,
    /// The selection [`Browse::collapsed`] last followed. See [`Browse::fold_for_selection`].
    followed: Option<Highlight>,
    /// What has been typed into each tab's search box, matched every frame: a scan of a few
    /// thousand titles costs little. One box per tab, as each tab searches different names.
    worlds: String,
    authors: String,
    versions: String,
    /// What has been typed into the sign-in fields, until the button is pressed. Neither outlives
    /// the app, and the password is never written down. See [`Panel::yno`].
    yno_user: String,
    yno_password: String,
    /// Whether the System Theme dialog is open.
    picking_theme: bool,
}

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    /// The graph in front of the user: what is selected, and what to look at next.
    #[default]
    Worlds,
    Authors,
    Versions,
    Settings,
}

/// What the panel was left at, once built and the frame's clicks have landed.
struct Panel {
    arrangement: Arrangement,
    hub_repulsion: f32,
    pull_across_branches: bool,
    link_reach: f32,
    ui_scale: f32,
    /// The antialiasing choice and the running surface's, so the tab can say when the two differ.
    /// See [`Overlay::antialias_running`].
    antialias: bool,
    antialias_running: bool,
    /// Current switch value. See [`load_row_follow_setting`].
    row_follow: bool,
    skin: Skin,
    /// A world picked out of one of the lists, to be routed to.
    chosen: Option<usize>,
    /// World of the hovered list row, highlighted in the graph. At most one.
    hovered_row: Option<usize>,
    /// What the menu, a link or the rocker chose to highlight, if any. The inner `None` is the
    /// rocker's off position, which clears the highlight.
    lit: Option<Option<Highlight>>,
    menu_taken: bool,
    /// Whether egui is using the pointer, so the frame's pointer events are the panel's.
    wants_pointer: bool,
    /// Whether the settings tab asked for the controls guide again.
    guide: bool,
    /// Whether the framing button was pressed, calling for a route to be framed the other way.
    refit: bool,
    /// Whether the reset button was pressed, taking the camera back to the opening view.
    reset_camera: bool,
    /// Whether the settings tab called for the worlds to be scattered and settled again.
    restart_layout: bool,
    /// The alternative way picked in the directions window.
    way: Option<usize>,
    /// The world whose maps were called for. A second press on the same world closes them again:
    /// see [`map::Maps::toggle`].
    mapped: Option<usize>,
    force_reveal_world: Option<usize>,
    /// Applied after the frame, so a frame is drawn in one language throughout.
    language: Option<i18n::Language>,
}

/// Everything the panel reads, gathered from [`AppEntities`] before the panel is built.
struct PanelData<'a> {
    data: &'a AppEntities,
    fps: f32,
    /// The route home from the highlight, origin last.
    route: Vec<usize>,
    /// The worlds worth naming among the descendants of the highlight.
    notable: Vec<usize>,
    /// The worlds a highlight names as a plain list: an author's work, a release's additions, a
    /// whole layer. Empty for the two highlights the panel reads some other way.
    listed: Vec<usize>,
    candidates: Vec<usize>,
    /// What the open tab's box matches, as indices into [`AppEntities::authors`] or
    /// [`AppEntities::versions`]. Empty for the tab that is not open, and the whole list for an
    /// empty box.
    authors: Vec<usize>,
    versions: Vec<usize>,
    selected: Option<Highlight>,
    /// The world a right-click opened a menu over, and where to draw the menu in egui's points.
    menu: Option<(usize, egui::Pos2)>,
    /// The world the pointer is over. `None` whenever nothing is hovered, and on a touch screen
    /// throughout.
    hovered: Option<usize>,
}

/// How far each edge of the safe area stands in from the window's, in egui's points. Not an
/// [`egui::Margin`], whose `i8` stops short of a phone's keyboard.
#[derive(Clone, Copy)]
pub(super) struct Insets {
    pub(super) left: f32,
    pub(super) right: f32,
    pub(super) top: f32,
    pub(super) bottom: f32,
}

impl Insets {
    pub(super) fn shrink_rect(self, rect: egui::Rect) -> egui::Rect {
        egui::Rect::from_min_max(
            rect.min + egui::vec2(self.left, self.top),
            rect.max - egui::vec2(self.right, self.bottom),
        )
    }
}

/// Where the windows stand, chosen by window width alone.
#[derive(Clone, Copy)]
enum PanelFit {
    /// The browse window down the left, the selection down the right, the graph between them.
    Wide,
    /// One window at a time over a tab bar, which is what a phone held upright has room for.
    Narrow,
}

/// What every window in a frame is fitted to, in egui's points.
struct Placement {
    insets: Insets,
    /// The safe area's width.
    width: f32,
    /// The most a window's contents may take down the safe area.
    height: f32,
    opacity: f32,
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
        let skin = Skin::remembered();
        skin::install_font(gui.context());
        skin::apply_skin(gui.context(), skin);
        let scale = remembered_scale();
        let remembered = load_antialias_setting();
        Self {
            gui,
            counted: Counted::default(),
            browse: Browse::default(),
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
            row_follow: load_row_follow_setting(),
            skin,
        }
    }

    /// Builds this frame's panel and consumes the events that land on it. The decisions are left on
    /// the [`Panel`] for [`Overlay::update`].
    fn panel(
        &mut self,
        window: &Window,
        frame_input: &mut FrameInput,
        data: &AppEntities,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        fading: Option<(String, f32)>,
    ) -> Panel {
        // egui multiplies the window's points per pixel by this; the graph uses the window's alone.
        self.gui.context().set_zoom_factor(self.ui_scale);
        // Rows time their own hold, see [`held_long_enough`]; a hand cannot hold still.
        if self.gui.context().input(|input| input.has_touch_screen()) {
            self.gui.context().all_styles_mut(|style| {
                style.interaction.tooltip_delay = HOVER_HOLD_SECONDS as f32;
                style.interaction.show_tooltips_only_when_still = false;
            });
            release_held_hover(self.gui.context());
        }
        let ratio = frame_input.device_pixel_ratio * self.ui_scale;
        let read = PanelData::new(data, self.counted.rate(), &self.browse, frame_input, ratio);
        // `parameters_mut` wakes the layout, so parameters are written back only on change.
        let mut panel = Panel {
            arrangement: data.arrangement,
            hub_repulsion: data.hub_repulsion,
            pull_across_branches: data.pull_across_branches,
            link_reach: data.link_reach,
            ui_scale: self.ui_scale,
            antialias: self.antialias,
            antialias_running: self.antialias_running,
            row_follow: self.row_follow,
            skin: self.skin,
            chosen: None,
            hovered_row: None,
            lit: None,
            menu_taken: false,
            wants_pointer: false,
            guide: false,
            refit: false,
            way: None,
            mapped: None,
            reset_camera: false,
            restart_layout: false,
            force_reveal_world: None,
            language: None,
        };
        // Bound out so the closure leaves `self.gui` free.
        let browse = &mut self.browse;
        let guide = &mut self.guide;
        let download = &mut self.download;
        let maps = &mut self.maps;
        let insets = safe_insets(frame_input.viewport, ratio);
        self.gui.update(window, |ui| {
            let ctx = ui.ctx().clone();
            let safe = insets.shrink_rect(ctx.content_rect());
            let (width, height) = (safe.width(), safe.height());
            let fit = PanelFit::for_width(width);
            let placement = Placement {
                insets,
                width,
                height: height - 2.0 * (f32::from(PANEL_MARGIN) + f32::from(skin::WINDOW_PADDING)),
                opacity: window_opacity(&ctx),
            };
            browse.fold_for_selection(read.selected);
            let remaining_insets = match fit {
                PanelFit::Wide => {
                    panel.browse_window(ui, &read, browse, account, dump, &placement);
                    if let Some(highlight) = read.selected {
                        panel.selection_window(ui, &read, highlight, &placement);
                    }
                    insets
                }
                PanelFit::Narrow => {
                    panel.narrow_windows(ui, &read, browse, account, dump, &placement)
                }
            };
            panel.rocker(
                ui,
                &read,
                fit.rocker_anchor(),
                remaining_insets,
                placement.opacity,
            );
            panel.directions(ui, &read);
            panel.menu(ui, &read);
            panel.system_theme_dialog(&ctx, &mut browse.picking_theme);
            panel.message(ui, &read, remaining_insets, fit.message_width(width));
            // In the frame that set `mapped`, so the window opens on release.
            if let Some(world) = panel.mapped {
                maps.toggle(world, data.titles[world].show(), &data.maps[world]);
            }
            maps.show(&ctx, remaining_insets);
            download.show(&ctx, remaining_insets);
            // Last, so the guide is drawn over the controls being explained.
            if panel.guide {
                guide.reopen();
            }
            guide.show(&ctx);
            // Over the windows too: the veil stood in for the whole interface.
            if let Some((says, opacity)) = &fading {
                loading_frame(&ctx, says, None, *opacity);
            }
        });
        // Only after the frame does egui know what lies under the pointer.
        panel.wants_pointer = self.gui.context().egui_wants_pointer_input()
            || self.gui.context().egui_wants_keyboard_input();
        track_keyboard(
            self.gui.context().egui_wants_keyboard_input(),
            &mut self.keyboard,
        );
        panel
    }

    /// One frame of the panel, with the decisions applied. Answers whether the layout starts
    /// over, after an arrangement switch or a restart.
    pub(super) fn update(
        &mut self,
        window: &Window,
        frame_input: &mut FrameInput,
        data: &mut AppEntities,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        fading: Option<(String, f32)>,
    ) -> bool {
        // The previous frame's time.
        self.timed(frame_input.elapsed_time as f32);

        let panel = self.panel(window, frame_input, data, account, dump, fading);
        if let Some(language) = panel.language {
            i18n::speak(language);
        }
        // Cloned: installing the face borrows the field.
        let context = self.gui.context().clone();
        self.japanese.serve(&context);
        let mut menu_taken = panel.menu_taken;
        if let Some(selection) = panel.lit {
            data.select(selection);
            menu_taken = true;
        }
        if menu_taken {
            data.menu = None;
        }
        if let Some(world) = panel.chosen {
            data.select(Some(Highlight::Route(world)));
        }
        // Every frame, so unhovering clears the row.
        data.hovered_row = panel.hovered_row;
        // The English name is the join key with the account.
        if let Some(world) = panel.force_reveal_world {
            account.force_reveal(data.titles[world].en.clone());
        }
        // Toggled, so pressing reframes even after the camera has arrived.
        if panel.refit {
            data.frame_route = !data.frame_route;
            data.framing = true;
        }
        if panel.reset_camera {
            data.framing = false;
            data.resetting_camera = true;
        }
        if let Some(way) = panel.way {
            data.take_way(way);
        }
        // three-d's `GUI` yields the pointer only mid-widget, so panel scrolls would zoom.
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
            save_antialias_setting(self.antialias);
        }
        if panel.skin != self.skin {
            self.skin = panel.skin;
            skin::apply_skin(self.gui.context(), self.skin);
            self.skin.remember();
        }
        if panel.row_follow != self.row_follow {
            self.row_follow = panel.row_follow;
            save_row_follow_setting(self.row_follow);
        }
        // Stored only once the drag ends.
        self.ui_scale = panel.ui_scale;
        if self.ui_scale != self.ui_scale_stored && !self.gui.context().egui_is_using_pointer() {
            self.ui_scale_stored = self.ui_scale;
            store::write(UI_SCALE, Some(&self.ui_scale.to_string()));
        }

        // Ahead of the early return: two of the four change no simulation parameter.
        let switched = panel.arrangement != data.arrangement;
        let layout = Layout {
            arrangement: panel.arrangement,
            hub_repulsion: panel.hub_repulsion,
            pull_across_branches: panel.pull_across_branches,
            link_reach: match switched {
                true => panel.arrangement.remembered_link_reach(),
                false => panel.link_reach,
            },
        };
        // Followed live without a reseed.
        if (panel.hub_repulsion, panel.pull_across_branches)
            != (data.hub_repulsion, data.pull_across_branches)
        {
            data.hub_repulsion = panel.hub_repulsion;
            data.pull_across_branches = panel.pull_across_branches;
            data.set_masses_and_leads();
        }
        if layout.link_reach != data.link_reach {
            data.link_reach = layout.link_reach;
            data.graph.parameters_mut().link_distance_max = layout.parameters().link_distance_max;
        }
        if layout != self.layout && !self.gui.context().egui_is_using_pointer() {
            self.layout = layout;
            layout.remember();
        }

        if !switched && !panel.restart_layout {
            return false;
        }
        if switched {
            data.arrangement = panel.arrangement;
            let parameters = data.graph.parameters_mut();
            parameters.dimensions = panel.arrangement.dimensions();
            let solve = layout.parameters();
            parameters.dag_level_distance = solve.dag_level_distance;
            parameters.dag_layers = solve.dag_layers;
            parameters.dag_level_slack = solve.dag_level_slack;
            parameters.force_charge = solve.force_charge;
            parameters.link_distance_max = solve.link_distance_max;
            parameters.repulsion_range = solve.repulsion_range;
            data.set_masses_and_leads();
            data.fade_lines_by_depth();
        }
        // A full settling window: the layout starts over.
        data.graph.revive();
        true
    }
}

impl<'a> PanelData<'a> {
    /// Gathers from `data` everything the panel reads, as the panel's closure borrows `data`
    /// immutably.
    fn new(
        data: &'a AppEntities,
        fps: f32,
        browse: &Browse,
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
            candidates: data.search(&browse.worlds),
            // Only the open tab.
            authors: match browse.tab {
                Tab::Authors => world::ranked_matches(
                    data.authors.iter().map(|by| by.name.names()),
                    &browse.authors,
                ),
                _ => Vec::new(),
            },
            versions: match browse.tab {
                Tab::Versions => world::ranked_matches(
                    data.versions
                        .iter()
                        .map(|version| std::iter::once(version.name.as_str())),
                    &browse.versions,
                ),
                _ => Vec::new(),
            },
            selected: data.selected,
            menu: data
                .menu
                .as_ref()
                .map(|menu| (menu.world, into_points(menu.at, frame_input, ratio))),
            hovered: data.hover,
        }
    }
}

impl Browse {
    /// Folds the open tab away when something new is selected, and unfolds the tab when nothing is.
    fn fold_for_selection(&mut self, selected: Option<Highlight>) {
        if selected == self.followed {
            return;
        }
        self.followed = selected;
        self.collapsed = selected.is_some();
    }
}

impl PanelFit {
    fn for_width(width: f32) -> Self {
        match width < NARROW_BELOW {
            true => Self::Narrow,
            false => Self::Wide,
        }
    }

    /// Clear of the tab bar and the window over the bar on a narrow screen.
    fn rocker_anchor(self) -> egui::Align2 {
        match self {
            Self::Wide => egui::Align2::RIGHT_BOTTOM,
            Self::Narrow => egui::Align2::RIGHT_TOP,
        }
    }

    /// How wide the message box's contents are, given the safe area's width.
    fn message_width(self, width: f32) -> f32 {
        let room = width - 2.0 * f32::from(PANEL_MARGIN);
        let beside = BROWSE_WIDTH.max(SELECTION_WIDTH) + f32::from(PANEL_MARGIN);
        let outer = match self {
            Self::Wide => (room - 2.0 * beside).clamp(MESSAGE_MIN_WIDTH, MESSAGE_WIDTH),
            Self::Narrow => room,
        };
        outer - 2.0 * f32::from(skin::WINDOW_PADDING)
    }
}

/// Faded while the pointer is on the graph, so the windows hide less of the graph. Never on a
/// touch screen, where nothing hovers to bring them back.
fn window_opacity(ctx: &egui::Context) -> f32 {
    // A finger only points while down; a mouse points wherever it rests.
    let pointing = ctx.input(|input| {
        input.pointer.has_pointer() && (!input.has_touch_screen() || input.pointer.any_down())
    });
    let on_graph = pointing && !ctx.is_pointer_over_egui() && !ctx.egui_wants_keyboard_input();
    let shown = ctx.animate_bool(egui::Id::new(WINDOWS_SHOWN_ID), !on_graph);
    egui::lerp(FADED_OPACITY..=1.0, shown)
}

/// The message box for the graph, at the foot of the safe area like a game's dialogue box.
fn message_area(id: &str, insets: Insets) -> egui::Area {
    let above = insets.bottom + f32::from(PANEL_MARGIN);
    egui::Area::new(egui::Id::new(id))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_BOTTOM, [0.0, -above])
}

fn remembered_scale() -> f32 {
    store::read_clamped(UI_SCALE, UI_SCALE_RANGE).unwrap_or(UI_SCALE_DEFAULT)
}

/// A window position in physical pixels, counted from the bottom, into egui's points counted from
/// the top: the same conversion the integration puts every pointer event through.
fn into_points(at: PhysicalPoint, frame_input: &FrameInput, ratio: f32) -> egui::Pos2 {
    egui::pos2(
        at.x / ratio,
        (frame_input.viewport.height as f32 - at.y) / ratio,
    )
}

/// How wide a node is drawn on screen, in physical pixels, across the quad through the node's
/// centre.
pub(super) fn drawn_width(
    camera: &Camera,
    across: Vec3,
    radius: f32,
    position: Vec3,
    center: PhysicalPoint,
) -> f32 {
    // `across` is square to the view, so the projection is linear and one projection suffices.
    let edge = camera.pixel_at_position(position + across * (radius * thumbnails::ASPECT));
    2.0 * pixel_distance(edge, center)
}

/// Removes `index.html`'s loading screen, shown while the wasm downloads. Called after every
/// frame; only the first call acts.
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

/// Puts `says` in `index.html`'s loading screen and stops the walking sprite, for a failure
/// before the app can draw.
#[cfg(target_family = "wasm")]
pub(super) fn say_in_placeholder(says: &str) {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(line) = document.get_element_by_id("loading-text") {
        line.set_text_content(Some(says));
    }
    if let Some(placeholder) = document.get_element_by_id("loading") {
        placeholder.set_class_name("failed");
    }
}

/// The loading screen at `opacity`: opaque in [`App::draw_loading`], fading in [`Overlay::panel`].
/// The veil sits an order below the message box: `layer_painter` paints last within an order.
pub(super) fn loading_frame(ctx: &egui::Context, says: &str, failed: Option<&str>, opacity: f32) {
    let opacity = opacity.clamp(0.0, 1.0);
    let channel = |channel: usize| (BACKGROUND_COLOR[channel] * 255.0) as u8;
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("loading-veil"),
    ))
    .rect_filled(
        // The whole window: the graph is drawn under the notch and status bar too.
        ctx.viewport_rect(),
        egui::CornerRadius::ZERO,
        egui::Color32::from_rgba_unmultiplied(
            channel(0),
            channel(1),
            channel(2),
            (opacity * 255.0) as u8,
        ),
    );
    let width = ctx.content_rect().width();
    let width = PanelFit::for_width(width).message_width(width);
    egui::Area::new(egui::Id::new("loading"))
        .order(egui::Order::Tooltip)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.set_opacity(opacity);
            skin::window(ui, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    // Shown even on failure: a retry is always pending.
                    skin::walking_sprite(ui);
                    let line = ui.label(says);
                    // Logged in full; on screen only on hover.
                    if let Some(error) = failed {
                        line.on_hover_text(error);
                    }
                });
            });
        });
}

/// How far the panel and the rocker stand off each edge of the window to clear the system's own
/// furniture, in egui's points. Zero where there is no safe area. See [`safe_rect`].
pub(super) fn safe_insets(viewport: Viewport, device_pixel_ratio: f32) -> Insets {
    let safe = safe_rect(viewport, device_pixel_ratio);
    let width = viewport.width as f32 / device_pixel_ratio;
    let height = viewport.height as f32 / device_pixel_ratio;
    Insets {
        left: safe.min.x.max(0.0),
        right: (width - safe.max.x).max(0.0),
        top: safe.min.y.max(0.0),
        bottom: (height - safe.max.y).max(0.0),
    }
}

/// The part of the window no system bar covers, in logical pixels: the whole window except on
/// Android, where the activity's content rect excludes the status and navigation bars.
fn safe_rect(viewport: Viewport, device_pixel_ratio: f32) -> egui::Rect {
    #[cfg(target_os = "android")]
    if let Some(app) = ANDROID.get() {
        let rect = app.content_rect();
        // Empty for the first frame or two.
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

/// Shows or hides the on-screen keyboard on Android as egui wants typing, passing only changes.
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

/// A window in an [`egui::Area`], at `anchor` of the window and inset like the panel to clear the
/// system bars.
pub(super) fn anchored_window<R>(
    ctx: &egui::Context,
    id: &str,
    anchor: egui::Align2,
    insets: Insets,
    opacity: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let offset = |align: egui::Align, near: f32, far: f32| match align {
        egui::Align::Min => near + f32::from(PANEL_MARGIN),
        egui::Align::Center => 0.0,
        egui::Align::Max => -(far + f32::from(PANEL_MARGIN)),
    };
    let x = offset(anchor.x(), insets.left, insets.right);
    let y = offset(anchor.y(), insets.top, insets.bottom);
    egui::Area::new(egui::Id::new(id))
        .anchor(anchor, [x, y])
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity);
            skin::window(ui, add_contents).inner
        })
}

/// An [`anchored_window`] that never fades.
pub(super) fn anchored_popup<R>(
    ctx: &egui::Context,
    id: &str,
    anchor: egui::Align2,
    insets: Insets,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    anchored_window(ctx, id, anchor, insets, 1.0, add_contents).inner
}

/// A hyperlink that opens through [`open_in_browser`], which egui's own does not reach.
pub(super) fn browser_link(
    ui: &mut egui::Ui,
    label: impl Into<egui::WidgetText>,
    url: &str,
) -> egui::Response {
    let link = ui.hyperlink_to(label, url);
    if link.clicked() {
        open_in_browser(url);
    }
    link
}

/// Opens a URL in the user's browser: a new tab in the web build, a command on a desktop, an
/// intent on Android. Nothing is awaited, and a failure is only logged.
pub(super) fn open_in_browser(url: &str) {
    #[cfg(target_family = "wasm")]
    {
        if let Some(window) = web_sys::window() {
            let _ = window.open_with_url_and_target(url, "_blank");
        }
    }
    #[cfg(target_os = "android")]
    {
        use jni::objects::{JObject, JValue};
        use jni::{jni_sig, jni_str};

        // Viewing a URL is a Java intent, so JNI. An application context needs
        // `FLAG_ACTIVITY_NEW_TASK` to start an activity.
        const FLAG_ACTIVITY_NEW_TASK: i32 = 0x1000_0000;

        let context = ndk_context::android_context();
        // SAFETY: the glue publishes both before `android_main`, and both outlive the app.
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
