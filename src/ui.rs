//! Everything drawn over the graph rather than in it: the panel and its sidebar, the rocker, the
//! right-click menu, the hover tooltip, and the frame that stands in for all of it while the dump
//! is still on its way. See [`Overlay`] and [`Panel`].

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
/// The graph runs underneath the sidebar rather than stopping at its edge, so a sidebar that hid
/// its share of the layout would make the person move the camera to read what they just selected.
/// Still opaque enough to keep text legible over a bright stretch of graph.
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

/// The rocker is the one control with no words on it and the one a thumb goes for on a phone. A
/// multiplier rather than a size, so it stays this much of whatever the UI scale made everything
/// else.
const ROCKER_SCALE: f32 = 3.0;

/// Frames counted over a window.
///
/// Counted rather than averaged frame by frame. An exponential average has to weight each sample
/// by how long it stood for or its time constant moves with the frame rate, and that weighting
/// gives a single long frame a share of the answer proportional to how long it was: one 100 ms
/// stall is a fifth of a 500 ms window, so a view drawing every other frame perfectly still reads
/// as though it were not. Counting answers the question actually being asked -- how many frames
/// arrived in this long -- and is the same arithmetic [`FrameStats`] reports.
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
    /// Whether the on-screen keyboard was last asked for. See [`track_keyboard`].
    pub(super) keyboard: bool,
    guide: guide::Guide,
    /// The offer of the phone build, made to the phones reading the page.
    download: download::Offer,
    maps: map::Maps,
    pub(super) japanese: japanese::Japanese,
    /// What the person has scaled the interface by, on top of whatever the system already says a
    /// point is worth. Multiplied into the pixel ratio the overlay is laid out and read at, and
    /// nothing else -- the graph is a place rather than an interface, and its distances are the
    /// screen's own.
    pub(super) ui_scale: f32,
    /// The scale the store was last written with, so a drag writes once at the end of itself
    /// rather than once a frame all the way along it.
    ui_scale_stored: f32,
    /// The layout choices the store was last written with, for the same reason.
    layout: Layout,
    /// Whether the person has asked for the edges to be smoothed, which is not always whether
    /// they are being smoothed. See [`Overlay::antialias_running`].
    antialias: bool,
    /// What the running surface was built for. Kept because a surface is asked for its samples
    /// once, as it is built, so this is a choice only the next start can honour.
    antialias_running: bool,
}

/// What the sidebar keeps between frames: held apart from [`Panel`], which is this frame's
/// decisions, because these are the person's and outlive the frame that made them.
#[derive(Default)]
struct Sidebar {
    /// This way round so that the default is open.
    closed: bool,
    tab: Tab,
    /// What has been typed into each tab's own box. Matched every frame rather than cached, and
    /// only for the open tab: a scan of a few thousand titles is cheaper than the frame it happens
    /// in.
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
    /// A world picked out of one of the lists, to be routed to.
    chosen: Option<usize>,
    /// A world a list row is pointing at, to be brightened where it sits in the graph. At most one:
    /// the pointer is over one row or none.
    pointed: Option<usize>,
    /// What the menu, a link, or the rocker asked to light, if any of them asked at all. The inner
    /// `None` is the rocker's off position, which asks for nothing to be lit.
    lit: Option<Option<Highlight>>,
    /// Whether the menu was acted on and should close.
    menu_taken: bool,
    /// Whether egui is working a widget and the frame's pointer events are the panel's.
    wants_pointer: bool,
    /// Whether the settings tab asked for the controls to be named again.
    guide: bool,
    /// Whether the framing button was pressed, asking for a route to be framed the other way.
    refit: bool,
    /// The world whose maps were asked for. A second press on the same world closes them again:
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
    /// [`AppEntities::versions`]. Empty for the tab that is not open, and the whole list, in the
    /// order it is kept in, for a box with nothing in it.
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
        }
    }

    /// Builds this frame's panel and consumes the events that land on it.
    ///
    /// The closure can only borrow `data` immutably, so everything the panel reads is walked out
    /// into [`PanelData`] first, and everything it decides is left on the [`Panel`] it is handed
    /// for [`Overlay::run`] to apply.
    fn panel(
        &mut self,
        window: &Window,
        frame_input: &mut FrameInput,
        data: &AppEntities,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        fading: Option<(String, f32)>,
    ) -> Panel {
        // egui's zoom factor multiplies whatever the window says a point is worth, which is
        // exactly what this scale means. So the panel is laid out and read at the product, and the
        // graph behind it at the window's ratio alone.
        self.gui.context().set_zoom_factor(self.ui_scale);
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
            chosen: None,
            pointed: None,
            lit: None,
            menu_taken: false,
            wants_pointer: false,
            guide: false,
            refit: false,
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
            // edges: a sidebar stopping short of the status bar would leave a stripe of its own
            // colour above it.
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
            panel.menu(ui, &read);
            panel.tooltip(ui, &read);
            // Read within the frame that asked, so the window is on screen the moment the button
            // is let go of rather than the frame after.
            if let Some(world) = panel.mapped {
                maps.toggle(world, data.titles[world].show(), &data.maps[world]);
            }
            maps.show(ui.ctx(), insets);
            download.show(ui.ctx(), insets);
            // Last, so it is drawn over everything it explains. A frame late when the settings tab
            // has just asked for it, which is a frame nobody sees.
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
        // Asked of egui after the frame, the only point at which it knows what was laid out under
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
        // three-d's own `GUI` surrenders the pointer only while egui is actively working a widget,
        // so a scroll over the panel would reach the camera as a zoom and a press on a bare label
        // the graph as a selection. Whatever egui wants the pointer for is the panel's.
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
        // Followed live, so the panel resizes under the hand, but written through only once the
        // hand is off it: a store is not something to write every frame of a drag.
        self.ui_scale = panel.ui_scale;
        if self.ui_scale != self.ui_scale_stored && !self.gui.context().egui_is_using_pointer() {
            self.ui_scale_stored = self.ui_scale;
            store::write(UI_SCALE, Some(&self.ui_scale.to_string()));
        }

        // Ahead of the early return below, because two of the four do not change the simulation's
        // own parameters and would never be written down if this waited for one that did. Held off
        // while a slider is under the hand, for the reason the UI scale is.
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
            // panel has taken its share of the events. A frame behind the cursor, which nobody
            // sees at this rate.
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

impl Panel {
    /// The tab bar stands outside the scroll, so it stays reachable however far down a list the
    /// person has read.
    fn window(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        sidebar: &mut Sidebar,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        ui.horizontal(|ui| {
            ui.selectable_value(&mut sidebar.tab, Tab::Worlds, t!("tab-worlds"));
            ui.selectable_value(&mut sidebar.tab, Tab::Authors, t!("tab-authors"));
            ui.selectable_value(&mut sidebar.tab, Tab::Versions, t!("tab-versions"));
            ui.selectable_value(&mut sidebar.tab, Tab::Settings, ICON_SETTINGS);
            // Against the far edge of the row, so it is nowhere near the tabs it is not one of.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button(ICON_CHEVRON_LEFT)
                    .on_hover_text(t!("hide-sidebar"))
                    .clicked()
                {
                    sidebar.closed = true;
                }
            });
        });
        ui.separator();
        // One scroll for the whole tab rather than one per list: a list scrolling inside a column
        // that also scrolled would fight the drag that reached it.
        egui::ScrollArea::vertical().show(ui, |ui| match sidebar.tab {
            Tab::Worlds => self.graph(ui, read, &mut sidebar.worlds),
            Tab::Authors => self.authors(ui, read, &mut sidebar.authors),
            Tab::Versions => self.versions(ui, read, &mut sidebar.versions),
            Tab::Settings => self.settings(ui, sidebar, account, dump),
        });
    }

    fn graph(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        ui.label(t!("fps", fps = format!("{:.0}", read.fps)));
        ui.label(t!(
            "graph-size",
            worlds = data.titles.len(),
            connections = data.graph.edge_count()
        ));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.dimensions, Dimensions::Two, t!("dimensions-2d"));
            ui.selectable_value(&mut self.dimensions, Dimensions::Three, t!("dimensions-3d"));
            ui.separator();
            ui.checkbox(&mut self.layered, t!("layered"))
                .on_hover_text(t!("layered-hint"));
        });
        ui.separator();
        search_box(ui, search, "worlds", t!("search-worlds"));
        for &world in &read.candidates {
            let selected = read.selected.and_then(Highlight::world) == Some(world);
            self.world_row(ui, world, selected, data.titles[world].show());
        }
        ui.separator();
        self.selection(ui, read);
    }

    /// The step back to the world the route came through is left out, the route above already
    /// naming it.
    fn forward_connections(&mut self, ui: &mut egui::Ui, read: &PanelData, world: usize) {
        let data = read.data;
        let parent = data.routes.parents[world];
        let ways: Vec<_> = data.connections[world]
            .iter()
            .filter(|step| Some(step.world) != parent)
            .collect();
        if ways.is_empty() {
            ui.label(t!("no-forward-connections"));
            return;
        }
        ui.label(t!("forward-connections", count = ways.len()));
        for step in ways {
            let lit = Highlight::Connection(world, step.world);
            // For a two-way connection the way out: a reader looking at the ways on from a world
            // is looking at leaving it.
            let asks = step
                .out
                .as_ref()
                .or(step.back.as_ref())
                .map(world::Ask::asks_emoji)
                .unwrap_or_default();
            let title = data.titles[step.world].show();
            ui.horizontal(|ui| {
                ui.label(step.arrow());
                let row = ui
                    .selectable_label(read.selected == Some(lit), title)
                    .on_hover_text(walk_of(step));
                if row.hovered() {
                    self.pointed = Some(step.world);
                }
                if row.clicked() {
                    if read.selected == Some(lit) {
                        self.lit = Some(Some(Highlight::Route(world)));
                    } else {
                        self.lit = Some(Some(lit));
                    }
                }
                ui.label(asks);
            });
        }
    }

    fn authors(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        search_box(ui, search, "authors", t!("search-authors"));
        ui.label(showing(read.authors.len(), data.authors.len(), "authors"));
        ui.separator();
        for &author in &read.authors {
            let by = &data.authors[author];
            if ui
                .selectable_label(
                    read.selected == Some(Highlight::Author(author)),
                    t!(
                        "author-row",
                        name = by.name.show(),
                        worlds = by.worlds.len()
                    ),
                )
                .clicked()
            {
                self.lit = Some(Some(Highlight::Author(author)));
            }
        }
    }

    /// Listed whole rather than cut to the best few: a release is looked up as often by reading
    /// down the history as by name.
    fn versions(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        search_box(ui, search, "versions", t!("search-versions"));
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

    /// The whole row lights the release, and each picture in it goes to its own world, so the
    /// catalog is a way into the graph rather than only a way to read it.
    fn version(&mut self, ui: &mut egui::Ui, read: &PanelData, version: usize) {
        let data = read.data;
        let release = &data.versions[version];
        // Every row draws the same widgets and egui names a widget by what is in it, so the
        // release's own place in the list is what keeps two rows apart.
        ui.push_id(version, |ui| {
            let lit = read.selected == Some(Highlight::Version(version));
            if ui
                .selectable_label(
                    lit,
                    match release.released.is_empty() {
                        true => t!(
                            "version-row",
                            name = &release.name,
                            worlds = worlds(release.worlds.len())
                        ),
                        false => t!(
                            "version-row-dated",
                            name = &release.name,
                            worlds = worlds(release.worlds.len()),
                            released = &release.released
                        ),
                    },
                )
                .clicked()
            {
                self.lit = Some(Some(Highlight::Version(version)));
            }
            // Nothing until the atlas arrives, and nothing ever if it cannot be had: the rest of
            // the row already says what the release is.
            let Some(sheet) = &data.sheet else { return };
            ui.horizontal(|ui| {
                for &world in release.worlds.iter().take(CATALOG_THUMBNAILS) {
                    if ui
                        .add(
                            sheet
                                .picture(data.cells[world], CATALOG_THUMBNAIL_HEIGHT)
                                .sense(egui::Sense::click()),
                        )
                        .on_hover_text(data.titles[world].show())
                        .clicked()
                    {
                        self.chosen = Some(world);
                    }
                }
            });
        });
    }

    fn world_info(&mut self, ui: &mut egui::Ui, data: &AppEntities, world: usize) {
        let mut lit = None;
        ui.horizontal(|ui| {
            ui.strong(data.titles[world].show());
            // A world the player has not been to is not named here, and a page opened on it would
            // say what this is holding back.
            if data.titles[world].known()
                && ui
                    .button(ICON_OPEN_IN_NEW)
                    .on_hover_text(t!("menu-open-wiki"))
                    .clicked()
            {
                open_in_browser(&data.titles[world].wiki_url());
            }
            // A few hundred worlds have never been drawn.
            if !data.maps[world].is_empty()
                && ui
                    .button(ICON_MAP)
                    .on_hover_text(t!("world-map-hint"))
                    .clicked()
            {
                self.mapped = Some(world);
            }
            if let Some(parent) = data.routes.parents[world]
                && ui
                    .button(ICON_ARROW_UPWARD)
                    .on_hover_text(t!("world-move-up"))
                    .clicked()
            {
                lit = Some(Highlight::Connection(parent, world));
            }
        });
        if speaking_japanese() && data.titles[world].known() {
            ui.horizontal(|ui| {
                ui.label("英名");
                let link = world::wiki_url(&data.titles[world].en);
                if ui
                    .hyperlink_to(&data.titles[world].en, &link)
                    .on_hover_text("海外wikiで見る")
                    .clicked()
                {
                    open_in_browser(&link);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(t!("world-author"));
            if ui
                .link(data.authors[data.author_of[world]].name.show())
                .on_hover_text(t!("world-author-hint"))
                .clicked()
            {
                lit = Some(Highlight::Author(data.author_of[world]));
            }
        });
        ui.horizontal(|ui| {
            ui.label(t!("world-connections", count = data.degrees[world]));
            if data.descendants[world] > 0
                && ui
                    .link(t!("world-descendants", count = data.descendants[world]))
                    .clicked()
            {
                lit = Some(Highlight::Descendants(world));
            } else if data.descendants[world] == 0 {
                ui.label(t!("dead-end"));
            }
        });
        self.lit = self.lit.or(lit.map(Some));
    }

    /// A tab of its own because the knobs are set once and then left alone.
    fn settings(
        &mut self,
        ui: &mut egui::Ui,
        sidebar: &mut Sidebar,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        self.language(ui);
        ui.add(
            egui::Slider::new(&mut self.hub_repulsion, HUB_REPULSION_RANGE).text(t!("hub-push")),
        )
        .on_hover_text(t!("hub-push-hint"));
        ui.add(egui::Slider::new(&mut self.link_reach, LINK_REACH_RANGE).text(t!("link-reach")))
            .on_hover_text(t!("link-reach-hint"));
        ui.add(egui::Slider::new(&mut self.ui_scale, UI_SCALE_RANGE).text(t!("ui-scale")))
            .on_hover_text(t!("ui-scale-hint"));
        self.antialiasing(ui);
        profile::controls(ui);
        // The way back to a panel that was dismissed for good, so ticking that box is not a door
        // that locks behind the person who ticked it.
        self.guide |= ui.button(t!("show-controls")).clicked();
        Self::clear_cache(ui);
        update::controls(ui);
        Self::freshness(ui, dump);
        Self::yno(ui, sidebar, account, dump);

        if ui
            .hyperlink_to(
                format!("{GITHUB}  {}", t!("github-link")),
                "https://github.com/Desdaemon/yumezu",
            )
            .clicked()
        {
            open_in_browser("https://github.com/Desdaemon/yumezu");
        }

        if let Some(platform) = download::Platform::detected()
            && ui
                .hyperlink_to(
                    format!(
                        "{}  {}",
                        platform.icon().codepoint,
                        t!("download-for", platform = platform.name())
                    ),
                    download::RELEASES,
                )
                .clicked()
        {
            open_in_browser(download::RELEASES);
        }
    }

    /// How old the graph is: the dump's own two stamps, which say when it was built and when the
    /// wiki behind it was last read whole. A dump can be hours old and still be missing an edit an
    /// incremental read never asked about, so both are worth reading.
    fn freshness(ui: &mut egui::Ui, dump: Option<&world::Dump>) {
        let Some(dump) = dump else {
            return;
        };
        if let Some(built) = dump.last_update.as_deref() {
            ui.label(t!("last-update", when = Self::stamped(built)))
                .on_hover_text(t!("last-update-hint"));
        }
        if let Some(whole) = dump.last_full_update.as_deref() {
            ui.label(t!("last-full-update", when = Self::stamped(whole)))
                .on_hover_text(t!("last-full-update-hint"));
        }
    }

    /// `2026-09-07T00:05:25.000Z` read as `2026-09-07 00:05`. To the minute, the dump being built
    /// a few times a day at most, and left in UTC as the dump stamps it: the reader's own zone
    /// would need a calendar this app does not carry.
    fn stamped(iso: &str) -> String {
        iso.get(..16).unwrap_or(iso).replace('T', " ")
    }

    /// Signing in to YNOproject, and drawing only as much of the graph as that account has seen.
    ///
    /// Nothing of `self`, because none of it is a decision the frame takes and applies afterwards:
    /// the account is told directly, and what comes of it is a graph built again rather than
    /// anything this panel draws.
    fn yno(
        ui: &mut egui::Ui,
        sidebar: &mut Sidebar,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        ui.separator();
        ui.strong(t!("yno"));
        match account.state() {
            yno::State::SignedOut => {}
            yno::State::Working => {
                ui.label(t!("yno-working"));
            }
            yno::State::SignedIn => {
                ui.label(t!("yno-signed-in"));
            }
            // The server's own words, which say whether it was the password or the network. Read
            // before the label, the color being borrowed out of the same `ui`.
            yno::State::Failed(why) => {
                let color = ui.visuals().error_fg_color;
                ui.colored_label(color, why);
            }
        }
        // The refresh below would drop a second ask on the floor while one is already out.
        let working = matches!(account.state(), yno::State::Working);
        if account.signed_in() {
            Self::completion(ui, account, dump);
            let mut frontier = account.frontier_wanted();
            if ui
                .checkbox(&mut frontier, t!("frontier"))
                .on_hover_text(t!("frontier-hint"))
                .changed()
            {
                account.set_frontier(frontier);
            }
            ui.horizontal(|ui| {
                // The account is read once at startup and not again, while the person is off
                // playing the game and walking into places.
                if ui
                    .add_enabled(!working, egui::Button::new(t!("yno-refresh")))
                    .on_hover_text(t!("yno-refresh-hint"))
                    .clicked()
                {
                    account.refresh();
                }
                // Forgotten here and left standing on YNOproject: ending it there would also sign
                // out whatever browser the person plays the game in.
                if ui.button(t!("yno-sign-out")).clicked() {
                    account.sign_out();
                }
            });
            return;
        }
        ui.label(t!("yno-hint"));
        Self::promise(ui);
        ui.add(egui::TextEdit::singleline(&mut sidebar.yno_user).hint_text(t!("yno-user")));
        ui.add(
            egui::TextEdit::singleline(&mut sidebar.yno_password)
                .password(true)
                .hint_text(t!("yno-password")),
        );
        let ready = !sidebar.yno_user.is_empty() && !sidebar.yno_password.is_empty();
        if ui
            .add_enabled(ready, egui::Button::new(t!("yno-sign-in")))
            .clicked()
        {
            // Taken rather than copied: the password has no second use, and the field it was typed
            // into is the only place it was ever held.
            account.sign_in(
                std::mem::take(&mut sidebar.yno_user),
                std::mem::take(&mut sidebar.yno_password),
            );
        }
    }

    /// How much of the game this account has seen, measured against the whole dump rather than the
    /// graph beside it: the graph may be the frontier, and a bar reading a hundred per cent the
    /// moment the switch was thrown would measure the person against themselves.
    ///
    /// Recomputed each frame it is drawn -- one pass over the titles, only while this tab is open
    /// -- because anything kept would go stale on either side.
    fn completion(ui: &mut egui::Ui, account: &yno::Account, dump: Option<&world::Dump>) {
        let (Some(visited), Some(dump)) = (account.visited(), dump) else {
            return;
        };
        let worlds = dump.worlds.len();
        let seen = dump.visited(visited);
        let share = seen as f32 / worlds.max(1) as f32;
        ui.add(egui::ProgressBar::new(share).text(t!(
            "yno-completion",
            seen = seen,
            worlds = worlds,
            percent = format!("{:.1}", share * 100.0)
        )))
        .on_hover_text(t!("yno-completion-hint"));
    }

    /// What signing in does with the account, beside the fields rather than behind a link: it is
    /// the one thing a person has to know at the moment they are deciding. The link points at the
    /// file that makes the promise rather than at the project.
    fn promise(ui: &mut egui::Ui) {
        ui.label(t!("yno-promise"));
        if ui
            .hyperlink_to(format!("{GITHUB}  {}", t!("yno-source")), YNO_SOURCE)
            .clicked()
        {
            open_in_browser(YNO_SOURCE);
        }
    }

    /// Nothing of `self`, the cache's own state being the only state there is: one store behind
    /// one client, and [`fetch::cleared`] is its answer to everyone.
    ///
    /// Native only. The page's cache is the browser's, which this app neither built nor may empty.
    #[cfg(not(target_family = "wasm"))]
    fn clear_cache(ui: &mut egui::Ui) {
        let cleared = fetch::cleared();
        let clearing = cleared == fetch::Cleared::Clearing;
        if ui
            .add_enabled(!clearing, egui::Button::new(t!("clear-cache")))
            .on_hover_text(t!("clear-cache-hint"))
            .clicked()
        {
            fetch::clear();
        }
        // Under the button rather than in it: the button says what it does, and this says what
        // came of the last press.
        match cleared {
            fetch::Cleared::Never => {}
            fetch::Cleared::Clearing => {
                ui.label(t!("clear-cache-clearing"));
            }
            fetch::Cleared::Done => {
                ui.label(t!("clear-cache-done"));
            }
            fetch::Cleared::Failed => {
                ui.label(t!("clear-cache-failed"));
            }
        }
    }

    /// Nothing to draw: see the native [`Self::clear_cache`] above.
    #[cfg(target_family = "wasm")]
    fn clear_cache(_: &mut egui::Ui) {}

    /// The one setting whose cost is worth more than its look, so the choice is the person's:
    /// see [`MULTISAMPLES`].
    fn antialiasing(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.antialias, t!("antialias"))
            .on_hover_text(t!("antialias-hint"));
        if self.antialias != self.antialias_running {
            ui.label(t!("antialias-restart"));
        }
    }

    /// Every language names itself, so someone who cannot read the one the app opened in can still
    /// find theirs in the list. The choice is left on the panel rather than taken here, so a frame
    /// half drawn in one language is not finished in another.
    fn language(&mut self, ui: &mut egui::Ui) {
        let speaking = i18n::speaking();
        let mut chosen = speaking;
        egui::ComboBox::from_label(t!("language"))
            .selected_text(speaking.name())
            .show_ui(ui, |ui| {
                for other in i18n::Language::ALL {
                    ui.selectable_value(&mut chosen, other, other.name());
                }
            });
        if chosen != speaking {
            self.language = Some(chosen);
        }
    }

    /// Every list of worlds in the panel is drawn through here, so all of them point alike. The
    /// exception is the ways on from a world -- see [`Self::forward_connections`], whose rows are
    /// connections and which lights the connection rather than either end of it.
    ///
    /// [`Self::pointed`] is written rather than merged: egui hovers at most one row at a time.
    fn world_row(
        &mut self,
        ui: &mut egui::Ui,
        world: usize,
        selected: bool,
        text: impl Into<egui::WidgetText>,
    ) {
        let row = ui.selectable_label(selected, text);
        if row.hovered() {
            self.pointed = Some(world);
        }
        if row.clicked() {
            self.chosen = Some(world);
        }
    }

    fn selection(&mut self, ui: &mut egui::Ui, read: &PanelData) {
        let data = read.data;
        match read.selected {
            None => {
                ui.label(t!("nothing-selected"));
                // Only a run drawing a frontier with an unwalked way left in it has any to offer.
                if !data.untaken.is_empty()
                    && ui
                        .link(t!("untaken-worlds"))
                        .on_hover_text(t!("untaken-worlds-hint"))
                        .clicked()
                {
                    self.lit = Some(Some(Highlight::Untaken));
                }
            }
            Some(Highlight::Route(world)) => {
                self.world_info(ui, data, world);
                self.forward_connections(ui, read, world);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(t!("route-length", count = read.route.len() - 1));
                    let (icon, hint) = match data.frame_route {
                        true => (ICON_ZOOM_IN, t!("zoom-in-world")),
                        false => (ICON_ZOOM_OUT, t!("zoom-out-route")),
                    };
                    self.refit |= ui.button(icon).on_hover_text(hint).clicked();
                });
                // Origin first, so the list reads in the order it is walked.
                for &world in read.route.iter().rev() {
                    // What the step into this world asks, in the direction the route walks it.
                    let asks = data.routes.parents[world]
                        .and_then(|from| {
                            data.connections[from]
                                .iter()
                                .find(|step| step.world == world)
                        })
                        .and_then(|step| step.out.as_ref())
                        .filter(|ask| ask.gate != Gate::Free);
                    ui.horizontal(|ui| {
                        self.world_row(ui, world, false, data.titles[world].show());
                        if let Some(ask) = asks {
                            ui.label(ask.asks_emoji()).on_hover_text(ask.asks());
                        }
                    });
                }
            }
            // The connection is the subject, but the ways on are still listed for the world it is
            // walked from: picking another from here is a second way out of the same world, not a
            // step onward from the first.
            Some(Highlight::Connection(at, far)) => {
                self.world_info(ui, data, at);
                let step = data.connections[at]
                    .iter()
                    .find(|step| step.world == far)
                    .expect("a lit connection is one of its own world's connections");
                self.forward_connections(ui, read, at);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(step.arrow());
                    if ui
                        .link(data.titles[far].show())
                        .on_hover_text(t!("trace-route"))
                        .clicked()
                    {
                        self.chosen = Some(far);
                    }
                });
                ui.label(walk_of(step));
            }
            Some(Highlight::Descendants(world)) => {
                self.world_info(ui, data, world);
                if read.notable.is_empty() {
                    ui.label(t!("no-notable-descendants"));
                } else {
                    ui.label(t!("notable-descendants"));
                }
                for &world in &read.notable {
                    let degree = data.degrees[world];
                    let kind = match degree {
                        ..NOTABLE_HUB_CONNECTIONS => t!("dead-end"),
                        _ => t!("junction"),
                    };
                    self.world_row(
                        ui,
                        world,
                        false,
                        t!(
                            "notable-world",
                            title = data.titles[world].show(),
                            kind = kind,
                            degree = degree
                        ),
                    );
                }
                ui.separator();
                self.forward_connections(ui, read, world);
            }
            // The author is the subject here, not the world that named them, so that world's own
            // information gives way to their whole body of work.
            Some(Highlight::Author(author)) => {
                let by = &data.authors[author];
                ui.horizontal(|ui| {
                    ui.strong(by.name.show());
                    if ui.button(ICON_OPEN_IN_NEW).clicked() {
                        open_in_browser(&by.wiki_url());
                    }
                });
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    self.world_row(ui, world, false, data.titles[world].show());
                }
            }
            // A release is a list like an author's work, and read the same way.
            Some(Highlight::Version(version)) => {
                let release = &data.versions[version];
                ui.strong(&release.name);
                if !release.released.is_empty() {
                    ui.label(t!("version-released", released = &release.released));
                }
                ui.label(t!("version-added", worlds = worlds(read.listed.len())));
                for &world in &read.listed {
                    self.world_row(ui, world, false, data.titles[world].show());
                }
            }
            // The order is the whole of what this list says, so its rows are bare titles like
            // every other list of worlds: what ranked them is said once, above them.
            Some(Highlight::Untaken) => {
                ui.strong(t!("untaken-worlds"));
                ui.label(t!("untaken-worlds-hint"));
                ui.separator();
                for &world in &data.untaken {
                    self.world_row(ui, world, false, data.titles[world].show());
                }
            }
            // A layer is a shell rather than a list, so the panel only says which is lit and how
            // much of the game sits on it.
            Some(Highlight::Layer(depth)) => {
                ui.strong(t!("layer-depth", depth = depth));
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    self.world_row(ui, world, false, data.titles[world].show());
                }
            }
        }
    }

    /// Steps the lit layer in and out from the origin.
    ///
    /// An [`egui::Area`] rather than part of the sidebar: it is a control over the graph, and it
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
                    // Scaled rather than sized: the arrows and the padding around them grow
                    // together, so the buttons keep their shape and still follow the UI scale.
                    let style = ui.style_mut();
                    for font in style.text_styles.values_mut() {
                        font.size *= ROCKER_SCALE;
                    }
                    style.spacing.button_padding *= ROCKER_SCALE;
                    style.spacing.item_spacing *= ROCKER_SCALE;
                    ui.vertical_centered(|ui| {
                        if ui
                            .add_enabled(lit != Some(0), egui::Button::new(ICON_KEYBOARD_ARROW_UP))
                            .on_hover_text(t!("rocker-shallower"))
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
                            .on_hover_text(t!("rocker-deeper"))
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
                    ui.strong(data.titles[world].show());
                    ui.label(data.authors[data.author_of[world]].name.show());
                    if data.descendants[world] > 0 && ui.button(t!("menu-descendants")).clicked() {
                        self.lit = Some(Some(Highlight::Descendants(world)));
                    }
                    let named = data.titles[world].known();
                    if named && ui.button(t!("menu-open-wiki")).clicked() {
                        open_in_browser(&data.titles[world].wiki_url());
                        self.menu_taken = true;
                    }
                    if named && speaking_japanese() && ui.button("海外wikiで見る").clicked() {
                        open_in_browser(&world::wiki_url(&data.titles[world].en));
                        self.menu_taken = true;
                    }
                    if !data.maps[world].is_empty() && ui.button(t!("world-map-hint")).clicked() {
                        self.mapped = Some(world);
                        self.menu_taken = true;
                    }
                    // Such a world only exists while the graph is a frontier, so the entry is not
                    // there to be wondered at in an ordinary run.
                    if !named
                        && ui
                            .button(t!("menu-reveal"))
                            .on_hover_text(t!("menu-reveal-hint"))
                            .clicked()
                    {
                        self.revealed = Some(world);
                        self.menu_taken = true;
                    }
                });
            });
    }

    /// Not while a menu is open: the menu is over the same world and already names it.
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
                    ui.strong(read.data.titles[world].show());
                    ui.label(read.data.authors[read.data.author_of[world]].name.show());
                });
            });
    }
}

/// How wide a node comes out on screen, in physical pixels: across the quad through its own
/// centre, so it is the node as drawn rather than a sphere around it, which is what says whether
/// the atlas still holds as much detail as the screen is asking of it.
pub(super) fn drawn_width(
    camera: &Camera,
    across: Vec3,
    radius: f32,
    position: Vec3,
    center: PhysicalPoint,
) -> f32 {
    // Half the quad, doubled, rather than both edges projected: `across` is square to the view
    // direction, so the two edges are the same distance from the camera and the projection is
    // linear between them. `center` is already in hand from the test that the node is on screen
    // at all, which leaves one projection per node here instead of three.
    let edge = camera.pixel_at_position(position + across * (radius * thumbnails::ASPECT));
    2.0 * (edge.x - center.x).hypot(edge.y - center.y)
}

/// The names `needle` matches, best first, or the whole list where nothing is asked for.
///
/// Ranked the way [`AppEntities::search`] ranks titles, but uncut, unlike that one: these lists
/// are already kept in the order they are worth reading down in.
fn matching<'a>(
    names: impl Iterator<Item = impl Iterator<Item = &'a str>>,
    needle: &str,
) -> Vec<usize> {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return (0..names.count()).collect();
    }
    let mut hits: Vec<_> = names
        .enumerate()
        .filter_map(|(at, name)| {
            // Whichever of an entry's names fits best, so someone reading in one language still
            // finds what they typed in the other.
            let (found, length) = name
                .filter_map(|name| Some((name.to_lowercase().find(&needle)?, name.len())))
                .min()?;
            Some((found, length, at))
        })
        .collect();
    hits.sort_unstable();
    hits.into_iter().map(|(_, _, at)| at).collect()
}

/// In the corner the sidebar's own button was in, so it reads as the same button facing the other
/// way. An [`egui::Area`] like the rocker, there being no panel left to hang it inside.
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
                    .on_hover_text(t!("show-sidebar"))
                    .clicked()
                {
                    sidebar.closed = false;
                }
            });
        });
}

/// `of` names the box to egui and never reaches the screen. `hint` is read in it while it is
/// empty, and so is one of the messages.
fn search_box(ui: &mut egui::Ui, search: &mut String, of: &str, hint: String) {
    let clear_id = egui::Id::new(of);
    let clear_size = egui::Vec2::splat(ui.spacing().interact_size.y);
    let output = egui::TextEdit::singleline(search)
        .hint_text(hint)
        .prefix(ICON_SEARCH)
        .suffix(egui::Atom::custom(clear_id, clear_size))
        .show(ui);
    if let Some(rect) = output.response.rect(clear_id)
        && ui.place(rect, egui::Label::new("❌")).clicked()
    {
        search.clear();
    }
}

/// `kind` names the list rather than the noun: a language does not necessarily say "3 authors"
/// with the same word it says "3 of 9 authors" with, so each list has a message for each and this
/// only picks between them.
fn showing(shown: usize, of: usize, kind: &str) -> String {
    match shown == of {
        true => i18n::format(&format!("showing-{kind}"), Some(&count_args(shown, of))),
        false => i18n::format(&format!("showing-{kind}-cut"), Some(&count_args(shown, of))),
    }
}

/// The two values either half of [`showing`] can ask for.
fn count_args(shown: usize, of: usize) -> fluent_bundle::FluentArgs<'static> {
    let mut args = fluent_bundle::FluentArgs::new();
    args.set("shown", shown);
    args.set("total", of);
    args
}

/// Said the way the language being read says a count.
fn worlds(count: usize) -> String {
    t!("worlds", count = count)
}

/// Takes the page's own loading line off the document, the once.
///
/// The wasm is a few megabytes and the canvas is black until it has come down and drawn, so
/// `index.html` carries the same line the loading frame does to stand in until then. Called after
/// every frame and does nothing after the first, a lookup that finds nothing not being worth doing
/// sixty times a second.
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

/// Drawn at `opacity` rather than always whole, because the frame is not switched off when the
/// dump lands: [`App::draw_loading`] draws it whole over a cleared screen, and [`Overlay::panel`]
/// draws what is left of it over the graph and the panel that replaced it. See [`App::veil`].
///
/// The veil goes on the foreground layer, over the panel and every window on it, and the words a
/// layer above. Two orders rather than two layers of one: a layer painted straight through
/// [`egui::Context::layer_painter`] is not one of the areas egui sorts among itself, so within an
/// order it is drawn last -- which put the veil over the words and made the frame a black screen.
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
                // could not reach the server is about to ask again.
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

/// What a connection can be walked, in a sentence. The two directions are named apart, because a
/// connection can be free one way and locked the other, and a reader deciding whether to walk it
/// needs the way they are about to walk.
fn walk_of(step: &world::Step) -> String {
    let asks = |ask: &world::Ask| match ask.asks() {
        asks if asks.is_empty() => t!("walk-freely"),
        asks => asks,
    };
    match (step.out.as_ref(), step.back.as_ref()) {
        (
            Some(Ask {
                gate: Gate::Free, ..
            }),
            Some(Ask {
                gate: Gate::Free, ..
            }),
        ) => t!("walk-free-both"),
        (
            Some(Ask {
                gate: Gate::DeadEnd,
                ..
            }),
            _,
        ) => t!("walk-dead-end"),
        (
            Some(Ask {
                gate: Gate::Isolated,
                ..
            }),
            _,
        ) => t!("walk-isolated"),
        (
            Some(Ask {
                gate: Gate::Locked, ..
            }),
            _,
        ) => t!("walk-locked-out"),
        (
            _,
            Some(Ask {
                gate: Gate::Locked, ..
            }),
        ) => t!("walk-locked-back"),
        (Some(out), Some(back)) => t!("walk-both", out = asks(out), back = asks(back)),
        (
            Some(Ask {
                gate: Gate::Free, ..
            }),
            None,
        ) => t!("walk-one-way"),
        (Some(out), None) => t!("walk-out-only", out = asks(out)),
        (
            None,
            Some(Ask {
                gate: Gate::Free, ..
            }),
        ) => t!("walk-no-entry"),
        (None, Some(back)) => t!("walk-back-only", back = asks(back)),
        (None, None) => t!("walk-none"),
    }
}

/// How far the panel and the rocker have to stand off each edge of the window to clear the
/// system's own furniture, in egui's points. Zero on every side where there is no safe area to
/// keep. See [`safe_rect`].
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
/// phone is the whole of it. A phone keeps a status bar over the top and a navigation bar under
/// the bottom, and the framework reports what is left as the activity's content rect.
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

/// egui asks for typing without saying where the typing should come from: on a desk there is
/// nothing to do, on a phone the app has to ask the system to draw a keyboard. Only the changes
/// are passed on, both calls being a trip through the framework.
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
        // The context the activity glue published is the Application's rather than the Activity's,
        // and an application context may not start an activity into the task it is asked from.
        // Hence the flag, which gives the browser a task of its own -- so leaving it comes back
        // here.
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
