//! What the panel says about whatever is lit: the world itself, the ways on from it, the walk
//! taken to reach it, and the lists a highlight names. See [`Panel::selection`].

use super::*;

/// The line down the left of a walk: a stop per world in the order they are walked, hollow where
/// the walk starts and solid where it arrives.
///
/// Drawn over the rows rather than between them, the height of a row not being known until it has
/// been laid out. `stops` is in the order they are drawn, top first.
fn rail(ui: &egui::Ui, stops: &[egui::Rect]) {
    // What a label is written in, the line being of a piece with the rows it runs beside.
    let color = ui.visuals().text_color();
    let stroke = egui::Stroke::new(RAIL_LINE, color);
    let painter = ui.painter();
    let at = |row: &egui::Rect| egui::pos2(row.left() + RAIL_WIDTH / 2.0, row.center().y);
    for (nth, pair) in stops.windows(2).enumerate() {
        let (mut from, to) = (at(&pair[0]), at(&pair[1]));
        // Away from the edge of the ring the walk starts at rather than out of the middle of it.
        // Every other stop is solid and drawn after the line, so it covers its own end.
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

/// The world a set of directions would run to or from: whatever is lit, which is what makes two
/// worlds out of one right-click. `None` while that is this world or nothing at all.
fn other_end(read: &PanelData, world: usize) -> Option<usize> {
    read.selected
        .and_then(Highlight::world)
        .filter(|&other| other != world)
}

/// What a world in a walk does when it is pressed.
enum Rows {
    /// Traces its own route home, which is what a route is read down to find.
    Traced,
    /// Nothing. The ways between two worlds are gone the moment something else is selected, so a
    /// reader following one down cannot be made to lose it by pressing what they are reading.
    Pointed,
}

impl Panel {
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
            let conditions = step
                .out
                .as_ref()
                .or(step.back.as_ref())
                .map(world::glyphs)
                .unwrap_or_default();
            let title = data.titles[step.world].show();
            let row = ui.add(
                Tile::new(read.selected == Some(lit), title)
                    .leading(world::arrow(step))
                    .trailing(conditions)
                    .hover(walk_of(step)),
            );
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
        }
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
                ui.label(t!("world-english-name"));
                let link = world::wiki_url(&data.titles[world].en);
                if ui
                    .hyperlink_to(&data.titles[world].en, &link)
                    .on_hover_text(t!("world-english-wiki"))
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

    /// Every list of worlds in the panel is drawn through here, so all of them point alike. The
    /// exception is [`Self::forward_connections`], whose rows are connections.
    ///
    /// [`Self::pointed`] is written rather than merged: egui hovers at most one row at a time.
    pub(super) fn world_row(
        &mut self,
        ui: &mut egui::Ui,
        world: usize,
        tile: Tile<'_>,
    ) -> egui::Response {
        let row = ui.add(tile);
        if row.hovered() {
            self.pointed = Some(world);
        }
        if row.clicked() {
            self.chosen = Some(world);
        }
        row
    }

    /// A right-click on a world, offering the two ways between it and whatever is lit. Nothing
    /// where there is no second world, an empty menu reading as a broken one.
    pub(super) fn with_directions(&mut self, row: &egui::Response, read: &PanelData, world: usize) {
        if other_end(read, world).is_none() {
            return;
        }
        // The app's own style rather than egui's menu style, which strips an item's fill and
        // stroke until it is hovered -- leaving a phone nothing to say these are controls. They
        // are the same two buttons `Panel::menu` already draws framed.
        egui::Popup::context_menu(row)
            .style(egui::style::StyleModifier::default())
            .show(|ui| {
                if self.directions_menu(ui, read, world) {
                    ui.close();
                }
            });
    }

    /// The two ways between a world and whatever is lit, and whether one of them was taken. The
    /// other end is named in the hover, a title being longer than a menu is wide.
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

    /// The worlds a walk goes through, origin first, each with what the step onto it demands.
    /// `walk` is arrival first, as [`AppEntities::route`] answers it.
    ///
    /// What a step demands is read off the world before it in this walk rather than off the
    /// canonical parent: the same thing for a route home, not for a way between two other worlds.
    fn walked(&mut self, ui: &mut egui::Ui, read: &PanelData, walk: &[usize], rows: Rows) {
        let data = read.data;
        let mut stops = Vec::with_capacity(walk.len());
        for (at, &world) in walk.iter().enumerate().rev() {
            let from = walk.get(at + 1).copied();
            let conditions = from
                .and_then(|from| world::step_conditions(&data.connections, data.hub, from, world))
                .filter(|step| step.gate != Gate::Free);
            // The connection itself, so a row here reads as a row of the ways on from a world
            // does: both directions named, even where they demand nothing. The escape home is the
            // one step that is no listed connection.
            let walked = from.and_then(|from| {
                data.connections[from]
                    .iter()
                    .find(|step| step.world == world)
            });
            let row = ui.horizontal(|ui| {
                // Left clear for the line, which is drawn once the rows have settled where they
                // are. What the step demands belongs against the world it is met on the way to.
                ui.add_space(RAIL_WIDTH);
                let leading = conditions.as_ref().map(world::glyphs).unwrap_or_default();
                let title = data.titles[world].show();
                let mut tile = Tile::new(false, title).leading(leading);
                if let Some(hover) = walked
                    .map(walk_of)
                    .or_else(|| conditions.as_ref().map(world::sentences))
                {
                    tile = tile.hover(hover);
                }
                match rows {
                    Rows::Traced => {
                        let row = self.world_row(ui, world, tile);
                        self.with_directions(&row, read, world);
                    }
                    Rows::Pointed => {
                        // visually hoverable only
                        if ui.add(tile).contains_pointer() {
                            self.pointed = Some(world);
                        }
                    }
                }
            });
            stops.push(row.response.rect);
        }
        rail(ui, &stops);
    }

    /// The ways between two worlds a reader can pick between, in a window rather than the sidebar,
    /// which reads the one being walked.
    ///
    /// Nothing where there is one way or none: an alternative is what this offers. Closing it
    /// drops the directions, there being nothing left to pick between.
    pub(super) fn directions(&mut self, ui: &mut egui::Ui, read: &PanelData) {
        let data = read.data;
        let Some(Highlight::Path(from, to)) = read.selected.filter(|_| data.ways.len() > 1) else {
            return;
        };
        // Copied out and back: `Window::open` holds its flag for as long as the closure runs.
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
                let connections = way.walk.len() - 1;
                // Where this way parts from the one above it, and from there the first world that
                // has not already named another: ways of a class share a tail, so what is peculiar
                // to one is looked for from where it parts.
                let parts = way
                    .walk
                    .iter()
                    .zip(above)
                    .position(|(here, there)| here != there)
                    .unwrap_or(above.len())
                    .max(1);
                let onward = way.walk.get(parts..).unwrap_or_default();
                let apart = onward
                    .iter()
                    .find(|world| !named.contains(world))
                    .or(onward.first());
                named.extend(apart);
                above = &way.walk;
                ui.horizontal(|ui| {
                    if ui
                        .selectable_label(at == data.way, t!("way-length", count = connections))
                        .clicked()
                    {
                        self.way = Some(at);
                    }
                    for pair in way.walk.windows(2) {
                        let step =
                            world::step_conditions(&data.connections, data.hub, pair[0], pair[1])
                                .filter(|step| step.gate != Gate::Free);
                        if let Some(step) = &step {
                            ui.label(world::glyphs(step))
                                .on_hover_text(world::sentences(step));
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

    pub(super) fn selection(&mut self, ui: &mut egui::Ui, read: &PanelData) {
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
                self.walked(ui, read, &read.route, Rows::Traced);
            }
            Some(Highlight::Path(from, world)) => {
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
                // Pressed, a world here would be selected and the ways between the two worlds
                // gone with the selection that called for them. Nothing else is read at that cost.
                self.walked(ui, read, &read.route, Rows::Pointed);
            }
            // The connection is the subject, but the ways on are listed for the world it is walked
            // from: picking another is a second way out of that world, not a step onward.
            Some(Highlight::Connection(at, far)) => {
                self.world_info(ui, data, at);
                let step = data.connections[at]
                    .iter()
                    .find(|step| step.world == far)
                    .expect("a lit connection is one of its own world's connections");
                self.forward_connections(ui, read, at);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(world::arrow(step));
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
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
            // A release is a list like an author's work, and read the same way.
            Some(Highlight::Version(version)) => {
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
            // The order is the whole of what this list says, so its rows are bare titles like
            // every other list of worlds: what ranked them is said once, above them.
            Some(Highlight::Untaken) => {
                ui.strong(t!("untaken-worlds"));
                ui.label(t!("untaken-worlds-hint"));
                ui.separator();
                for &world in &data.untaken {
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
            // A layer is a shell rather than a list, so the panel only says which is lit and how
            // much of the game sits on it.
            Some(Highlight::Layer(depth)) => {
                ui.strong(t!("layer-depth", depth = depth));
                ui.label(worlds(read.listed.len()));
                for &world in &read.listed {
                    self.world_row(ui, world, Tile::new(false, data.titles[world].show()));
                }
            }
        }
    }
}

/// What a connection can be walked, in a sentence. The two directions are named apart, a connection
/// being free one way and locked the other.
fn walk_of(step: &world::Step) -> String {
    let said = |conditions: &world::Conditions| match world::sentences(conditions) {
        said if said.is_empty() => t!("walk-freely"),
        said => said,
    };
    match (step.out.as_ref(), step.back.as_ref()) {
        (
            Some(Conditions {
                gate: Gate::Free, ..
            }),
            Some(Conditions {
                gate: Gate::Free, ..
            }),
        ) => t!("walk-free-both"),
        (
            Some(Conditions {
                gate: Gate::DeadEnd,
                ..
            }),
            _,
        ) => t!("walk-dead-end"),
        (
            Some(Conditions {
                gate: Gate::Isolated,
                ..
            }),
            _,
        ) => t!("walk-isolated"),
        // Only where the other direction demands nothing: a way out that wants an effect and back
        // that is locked has to say both, which `walk-both` does.
        (
            Some(Conditions {
                gate: Gate::Locked, ..
            }),
            Some(Conditions {
                gate: Gate::Free, ..
            })
            | None,
        ) => t!("walk-locked-out"),
        (
            Some(Conditions {
                gate: Gate::Free, ..
            })
            | None,
            Some(Conditions {
                gate: Gate::Locked, ..
            }),
        ) => t!("walk-locked-back"),
        (Some(out), Some(back)) => t!("walk-both", out = said(out), back = said(back)),
        (
            Some(Conditions {
                gate: Gate::Free, ..
            }),
            None,
        ) => t!("walk-one-way"),
        (Some(out), None) => t!("walk-out-only", out = said(out)),
        (
            None,
            Some(Conditions {
                gate: Gate::Free, ..
            }),
        ) => t!("walk-no-entry"),
        (None, Some(back)) => t!("walk-back-only", back = said(back)),
        (None, None) => t!("walk-none"),
    }
}
