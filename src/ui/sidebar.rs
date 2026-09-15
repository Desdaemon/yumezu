//! The three tabs that read the graph rather than a selection in it -- the worlds, the authors,
//! the releases -- and the boxes and counts they are searched through. See [`Panel::window`].

use super::*;

/// A right-click on a release, offering the history page it is written up on. The Japanese wiki
/// writes its own history rather than translating the English one, so both are offered.
fn wiki_menu(row: &egui::Response, release: &world::Version) {
    egui::Popup::context_menu(row)
        .style(egui::style::StyleModifier::default())
        .show(|ui| {
            if ui.button(t!("menu-open-wiki")).clicked() {
                open_in_browser(&release.wiki_url());
                ui.close();
            }
            if speaking_japanese() && ui.button(t!("world-english-wiki")).clicked() {
                open_in_browser(&world::version_url(&release.name));
                ui.close();
            }
        });
}

impl Panel {
    /// The tab bar stands outside the scroll, so it stays reachable however far down a list the
    /// person has read.
    pub(super) fn window(
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
            let row = self.world_row(ui, world, Tile::new(selected, data.titles[world].show()));
            self.with_directions(&row, read, world);
        }
        ui.separator();
        self.selection(ui, read);
    }

    fn authors(&mut self, ui: &mut egui::Ui, read: &PanelData, search: &mut String) {
        let data = read.data;
        search_box(ui, search, "authors", t!("search-authors"));
        ui.label(showing(read.authors.len(), data.authors.len(), "authors"));
        ui.separator();
        for &author in &read.authors {
            let by = &data.authors[author];
            let tile = Tile::new(
                read.selected == Some(Highlight::Author(author)),
                t!(
                    "author-row",
                    name = by.name.show(),
                    worlds = by.worlds.len()
                ),
            );
            if ui.add(tile).clicked() {
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
            let tile = Tile::new(
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
            );
            let row = ui.add(tile);
            if row.clicked() {
                self.lit = Some(Some(Highlight::Version(version)));
            }
            wiki_menu(&row, release);
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
}

/// The names `needle` matches, best first, or the whole list where nothing is typed.
///
/// Ranked the way [`AppEntities::search`] ranks titles, but uncut, unlike that one: these lists
/// are already kept in the order they are worth reading down in.
pub(super) fn matching<'a>(
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
pub(super) fn sidebar_opener(ui: &mut egui::Ui, sidebar: &mut Sidebar, insets: egui::Margin) {
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

/// `kind` names the list rather than the noun: a language need not say "3 authors" with the same
/// word it says "3 of 9 authors" with, so each list has a message for each.
fn showing(shown: usize, of: usize, kind: &str) -> String {
    match shown == of {
        true => i18n::format(&format!("showing-{kind}"), Some(&count_args(shown, of))),
        false => i18n::format(&format!("showing-{kind}-cut"), Some(&count_args(shown, of))),
    }
}

/// The two values either half of [`showing`] names.
fn count_args(shown: usize, of: usize) -> fluent_bundle::FluentArgs<'static> {
    let mut args = fluent_bundle::FluentArgs::new();
    args.set("shown", shown);
    args.set("total", of);
    args
}

/// Said the way the language being read says a count.
pub(super) fn worlds(count: usize) -> String {
    t!("worlds", count = count)
}
