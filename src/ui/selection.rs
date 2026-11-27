//! The panel for the highlighted world: the world, the ways onward, the route that led there,
//! and the lists a highlight names. See [`Panel::selection`].

use super::*;

/// The line down the left of a route: a stop per world, hollow at the origin, solid at the arrival.
/// Drawn over the rows once layout has set their heights. `stops` is top first.
fn rail(ui: &egui::Ui, stops: &[egui::Rect]) {
    let color = ui.visuals().text_color();
    let stroke = egui::Stroke::new(RAIL_LINE, color);
    let painter = ui.painter();
    let at = |row: &egui::Rect| egui::pos2(row.left() + RAIL_WIDTH / 2.0, row.center().y);
    for (nth, pair) in stops.windows(2).enumerate() {
        let (mut from, to) = (at(&pair[0]), at(&pair[1]));
        // The line starts at the hollow origin's rim; the solid stops, drawn after, cover the rest.
        if nth == 0 {
            from.y += RAIL_END;
        }
        painter.line_segment([from, to], stroke);
    }
    for (nth, row) in stops.iter().enumerate() {
        match (nth, nth + 1 == stops.len()) {
            (0, _) => painter.circle_stroke(at(row), RAIL_END - RAIL_LINE / 2.0, stroke),
            (_, true) => painter.circle_filled(at(row), RAIL_END, color),
            _ => painter.circle_filled(at(row), RAIL_STOP, color),
        };
    }
}

/// The highlighted world, at the other end of directions from `world`; `None` when nothing is
/// highlighted or `world` is.
fn other_end(read: &PanelData, world: usize) -> Option<usize> {
    read.selected
        .and_then(Highlight::world)
        .filter(|&other| other != world)
}

/// What pressing a world on a route does.
enum Rows {
    /// Traces the pressed world's route home.
    Traced,
    /// Nothing: selecting another world would drop the directions being read.
    Pointed,
}

impl Panel {
    /// Leaves out the step back to the route's previous world, which the route above names.
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
            // For a two-way connection, the exit.
            let conditions = step
                .outgoing
                .as_ref()
                .or(step.incoming.as_ref())
                .map(world::conditions_glyphs)
                .unwrap_or_default();
            let title = data.titles[step.world].show();
            let row = ui.add(
                Tile::new(read.selected == Some(lit), title)
                    .leading(world::direction_icon(step))
                    .trailing(conditions)
                    .hover(passage_sentence(step)),
            );
            if row.hovered() {
                self.hovered_row = Some(step.world);
            }
            if row.clicked() {
                if read.selected == Some(lit) {
                    self.lit = Some(Some(Highlight::Route(world)));
                } else {
                    self.lit = Some(Some(lit));
                }
            }
        }
    }

    fn world_info(&mut self, ui: &mut egui::Ui, data: &AppEntities, world: usize) {
        let mut lit = None;
        ui.horizontal(|ui| {
            ui.strong(data.titles[world].show());
            // The wiki page would reveal an unvisited world's withheld name.
            if data.titles[world].is_searchable()
                && ui
                    .button(ICON_OPEN_IN_NEW)
                    .on_hover_text(t!("menu-open-wiki"))
                    .clicked()
            {
                open_in_browser(&data.titles[world].wiki_url());
            }
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
        if speaking_japanese() && data.titles[world].is_searchable() {
            ui.horizontal(|ui| {
                ui.label(t!("world-english-name"));
                let link = world::wiki_url(&data.titles[world].en);
                browser_link(ui, &data.titles[world].en, &link)
                    .on_hover_text(t!("world-english-wiki"));
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
            if data.descendant_counts[world] > 0
                && ui
                    .link(t!(
                        "world-descendants",
                        count = data.descendant_counts[world]
                    ))
                    .clicked()
            {
                lit = Some(Highlight::Descendants(world));
            } else if data.descendant_counts[world] == 0 {
                ui.label(t!("dead-end"));
            }
        });
        self.lit = self.lit.or(lit.map(Some));
    }

    /// A row of any panel list of worlds, so all lists answer hover and click alike.
    /// Overwrites [`Self::hovered_row`]: egui hovers at most one row at a time.
    pub(super) fn world_row(
        &mut self,
        ui: &mut egui::Ui,
        world: usize,
        tile: Tile<'_>,
    ) -> egui::Response {
        let row = ui.add(tile);
        if row.hovered() {
            self.hovered_row = Some(world);
        }
        if row.clicked() {
            self.chosen = Some(world);
        }
        row
    }

    /// A right-click menu on a world's row, with directions to and from the highlighted world.
    /// No menu without a second world: an empty menu looks broken.
    pub(super) fn with_directions(&mut self, row: &egui::Response, read: &PanelData, world: usize) {
        if other_end(read, world).is_none() {
            return;
        }
        // egui's menu style hides fill and stroke until hover, so a phone shows no buttons.
        egui::Popup::context_menu(row)
            .style(egui::style::StyleModifier::default())
            .show(|ui| {
                if self.directions_menu(ui, read, world) {
                    ui.close();
                }
            });
    }

    /// Directions to and from the highlighted world, answering whether either was chosen.
    /// The hover names the other world, since a title is wider than the menu.
    pub(super) fn directions_menu(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        world: usize,
    ) -> bool {
        let Some(other) = other_end(read, world) else {
            return false;
        };
        let named = read.data.titles[other].show();
        let mut taken = false;
        if ui
            .button(t!("menu-directions-from"))
            .on_hover_text(t!("menu-directions-from-hint", world = named))
            .clicked()
        {
            self.lit = Some(Some(Highlight::Path(world, other)));
            taken = true;
        }
        if ui
            .button(t!("menu-directions-to"))
            .on_hover_text(t!("menu-directions-to-hint", world = named))
            .clicked()
        {
            self.lit = Some(Some(Highlight::Path(other, world)));
            taken = true;
        }
        taken
    }

    /// A route's worlds, origin first, each with what the step onto that world demands.
    /// `route` is arrival first; each step starts at the prior world in `route`.
    fn route_rows(&mut self, ui: &mut egui::Ui, read: &PanelData, route: &[usize], rows: Rows) {
        let data = read.data;
        let mut stops = Vec::with_capacity(route.len());
        for (at, &world) in route.iter().enumerate().rev() {
            let from = route.get(at + 1).copied();
            let conditions = from
                .and_then(|from| world::step_conditions(&data.connections, data.hub, from, world))
                .filter(|step| step.gate != Gate::Free);
            // Both directions, as in forward connections; `None` for the unlisted escape home.
            let step_onto =
                from.and_then(|from| world::step_between(&data.connections, from, world));
            let row = ui.horizontal(|ui| {
                // Room for the rail, drawn after layout.
                ui.add_space(RAIL_WIDTH);
                let leading = conditions
                    .as_ref()
                    .map(world::conditions_glyphs)
                    .unwrap_or_default();
                let title = data.titles[world].show();
                let mut tile = Tile::new(false, title).leading(leading);
                if let Some(hover) = step_onto
                    .map(passage_sentence)
                    .or_else(|| conditions.as_ref().map(world::conditions_display))
                {
                    tile = tile.hover(hover);
                }
                match rows {
                    Rows::Traced => {
                        let row = self.world_row(ui, world, tile);
                        self.with_directions(&row, read, world);
                    }
                    Rows::Pointed => {
                        if ui.add(tile).contains_pointer() {
                            self.hovered_row = Some(world);
                        }
                    }
                }
            });
            stops.push(row.response.rect);
        }
        rail(ui, &stops);
    }

    /// A window listing the alternative ways between two worlds, apart from the selection window.
    /// Shown only for more than one way; closing the window drops the directions.
    pub(super) fn directions(&mut self, ui: &mut egui::Ui, read: &PanelData) {
        let data = read.data;
        let Some(Highlight::Path(from, to)) = read.selected.filter(|_| data.ways.len() > 1) else {
            return;
        };
        // Copied: `Window::open` borrows the flag while the closure runs.
        let mut showing = true;
        egui::Window::new(t!(
            "directions-title",
            origin = data.titles[from].show(),
            destination = data.titles[to].show()
        ))
        .id(egui::Id::new(DIRECTIONS_ID))
        .open(&mut showing)
        .constrain(true)
        .default_size(DIRECTIONS_SIZE)
        .show(ui.ctx(), |ui| {
            let (mut named, mut above): (Vec<usize>, &[usize]) = (Vec::new(), &[]);
            for (at, way) in data.ways.iter().enumerate() {
                let connections = way.worlds.len() - 1;
                // A way shares leading worlds with the one above; the distinct part follows them.
                let parts = way
                    .worlds
                    .iter()
                    .zip(above)
                    .position(|(here, there)| here != there)
                    .unwrap_or(above.len())
                    .max(1);
                let onward = way.worlds.get(parts..).unwrap_or_default();
                let apart = onward
                    .iter()
                    .find(|world| !named.contains(world))
                    .or(onward.first());
                named.extend(apart);
                above = &way.worlds;
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(at == data.way, t!("way-length", count = connections))
                        .clicked()
                    {
                        self.way = Some(at);
                    }
                    for pair in way.worlds.windows(2) {
                        let step =
                            world::step_conditions(&data.connections, data.hub, pair[0], pair[1])
                                .filter(|step| step.gate != Gate::Free);
                        if let Some(step) = &step {
                            ui.label(world::conditions_glyphs(step))
                                .on_hover_text(world::conditions_display(step));
                        }
                    }
                    if let Some(&apart) = apart {
                        ui.weak(t!("way-via", world = data.titles[apart].show()));
                    }
                });
            }
        });
        if !showing {
            self.lit = Some(None);
        }
    }

    /// Down the right of a wide window, beside the graph and clear of the list.
    pub(super) fn selection_window(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        highlight: Highlight,
        placement: &Placement,
    ) {
        let anchor = egui::Align2::RIGHT_TOP;
        anchored_window(
            ui.ctx(),
            "selection",
            anchor,
            placement.insets,
            placement.opacity,
            |ui| {
                ui.set_width(SELECTION_WIDTH);
                ui.set_max_height(placement.height);
                egui::ScrollArea::vertical()
                    .max_height(placement.height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.selection(ui, read, highlight));
            },
        );
    }

    pub(super) fn selection(&mut self, ui: &mut egui::Ui, read: &PanelData, highlight: Highlight) {
        let data = read.data;
        match highlight {
            Highlight::Route(world) => {
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
                self.route_rows(ui, read, &read.route, Rows::Traced);
            }
            Highlight::Path(from, world) => {
                self.world_info(ui, data, world);
                self.forward_connections(ui, read, world);
                ui.separator();
                if read.route.is_empty() {
                    ui.label(t!("no-path"));
                    return;
                }
                ui.label(t!(
                    "path-length",
                    count = read.route.len() - 1,
                    origin = data.titles[from].show()
                ));
                // Pressing would select the world and drop the ways between the two.
                self.route_rows(ui, read, &read.route, Rows::Pointed);
            }
            // Lists the forward connections of the connection's source world.
            Highlight::Connection(at, far) => {
                self.world_info(ui, data, at);
                let step = world::step_between(&data.connections, at, far)
                    .expect("a lit connection is one of its own world's connections");
                self.forward_connections(ui, read, at);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(world::direction_icon(step));
                    if ui
                        .link(data.titles[far].show())
                        .on_hover_text(t!("trace-route"))
                        .clicked()
                    {
                        self.chosen = Some(far);
                    }
                });
                ui.label(passage_sentence(step));
            }
            Highlight::Descendants(world) => {
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
                        Tile::new(
                            false,
                            t!(
                                "notable-world",
                                title = data.titles[world].show(),
                                kind = kind,
                                degree = degree
                            ),
                        ),
                    );
                }
                ui.separator();
                self.forward_connections(ui, read, world);
            }
            Highlight::Author(author) => {
                let by = &data.authors[author];
                ui.horizontal(|ui| {
                    ui.strong(by.name.show());
                    if ui.button(ICON_OPEN_IN_NEW).clicked() {
                        open_in_browser(&by.wiki_url());
                    }
                });
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
            Highlight::Version(version) => {
                let release = &data.versions[version];
                ui.horizontal(|ui| {
                    ui.strong(&release.name);
                    if ui
                        .button(ICON_OPEN_IN_NEW)
                        .on_hover_text(t!("menu-open-wiki"))
                        .clicked()
                    {
                        open_in_browser(&release.wiki_url());
                    }
                });
                if !release.released.is_empty() {
                    ui.label(t!("version-released", released = &release.released));
                }
                ui.label(t!("version-added", worlds = worlds(read.listed.len())));
                for &world in &read.listed {
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
            // Bare titles: the ranking is said once, above.
            Highlight::Untaken => {
                ui.strong(t!("untaken-worlds"));
                ui.label(t!("untaken-worlds-hint"));
                ui.separator();
                for &world in &data.untaken {
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
            Highlight::Layer(depth) => {
                ui.strong(t!("layer-depth", depth = depth));
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
        }
    }
}

/// How a connection can be passed, in a sentence naming each direction, since the two can differ.
fn passage_sentence(step: &world::Step) -> String {
    let said = |conditions: &world::Conditions| match world::conditions_display(conditions) {
        said if said.is_empty() => t!("passage-freely"),
        said => said,
    };
    match (step.outgoing.as_ref(), step.incoming.as_ref()) {
        (
            Some(Conditions {
                gate: Gate::Free, ..
            }),
            Some(Conditions {
                gate: Gate::Free, ..
            }),
        ) => t!("passage-free-both"),
        (
            Some(Conditions {
                gate: Gate::DeadEnd,
                ..
            }),
            _,
        ) => t!("passage-dead-end"),
        (
            Some(Conditions {
                gate: Gate::Isolated,
                ..
            }),
            _,
        ) => t!("passage-isolated"),
        (
            Some(Conditions {
                gate: Gate::Locked, ..
            }),
            Some(Conditions {
                gate: Gate::Free, ..
            })
            | None,
        ) => t!("passage-locked-out"),
        (
            Some(Conditions {
                gate: Gate::Free, ..
            })
            | None,
            Some(Conditions {
                gate: Gate::Locked, ..
            }),
        ) => t!("passage-locked-back"),
        (Some(out), Some(back)) => t!("passage-both", out = said(out), back = said(back)),
        (
            Some(Conditions {
                gate: Gate::Free, ..
            }),
            None,
        ) => t!("passage-one-way"),
        (Some(out), None) => t!("passage-out-only", out = said(out)),
        (
            None,
            Some(Conditions {
                gate: Gate::Free, ..
            }),
        ) => t!("passage-no-entry"),
        (None, Some(back)) => t!("passage-back-only", back = said(back)),
        (None, None) => t!("passage-none"),
    }
}
