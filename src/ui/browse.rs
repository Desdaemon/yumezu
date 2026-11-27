//! The tabs that read the whole graph rather than a selection -- the worlds, the authors, the
//! releases, the settings -- and the windows they stand in on a wide screen and a narrow one.

use super::*;

/// A right-click menu on a release, opening the release's history page. The Japanese wiki keeps
/// a separate history, so a Japanese user is offered both.
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

/// Pressing the open tab again folds the list away; on a narrow screen, the window closes.
fn tabs(ui: &mut egui::Ui, browse: &mut Browse) {
    let tabs = [
        (Tab::Worlds, t!("tab-worlds")),
        (Tab::Authors, t!("tab-authors")),
        (Tab::Versions, t!("tab-versions")),
        (Tab::Settings, ICON_SETTINGS.into()),
    ];
    let spacing = ui.spacing().item_spacing.x;
    let cell = (ui.available_width() - spacing * (tabs.len() - 1) as f32) / tabs.len() as f32;
    ui.horizontal(|ui| {
        for (tab, label) in tabs {
            let open = browse.tab == tab;
            let button = egui::Button::selectable(open, label).min_size(egui::vec2(cell, 0.0));
            if ui.add(button).clicked() {
                browse.collapsed = open && !browse.collapsed;
                browse.tab = tab;
            }
        }
    });
}

impl Panel {
    /// Down the left of a wide window, the tabs above whatever the open one holds.
    pub(super) fn browse_window(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        browse: &mut Browse,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        placement: &Placement,
    ) {
        let anchor = egui::Align2::LEFT_TOP;
        anchored_window(
            ui.ctx(),
            "browse",
            anchor,
            placement.insets,
            placement.opacity,
            |ui| {
                ui.set_width(BROWSE_WIDTH);
                ui.set_max_height(placement.height);
                tabs(ui, browse);
                ui.separator();
                self.tab_body(ui, read, browse, account, dump);
            },
        );
    }

    /// A tab bar along the foot, under the open tab or, once folded, the selection.
    /// Answers the insets left for everything else.
    pub(super) fn narrow_windows(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        browse: &mut Browse,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
        placement: &Placement,
    ) -> Insets {
        let inner_width =
            placement.width - 2.0 * (f32::from(PANEL_MARGIN) + f32::from(skin::WINDOW_PADDING));
        let anchor = egui::Align2::CENTER_BOTTOM;
        let bar = anchored_window(ui.ctx(), "tab bar", anchor, placement.insets, 1.0, |ui| {
            ui.set_width(inner_width);
            tabs(ui, browse);
        });
        let remaining_insets = Insets {
            bottom: placement.insets.bottom + bar.response.rect.height() + f32::from(PANEL_MARGIN),
            ..placement.insets
        };
        let height = placement.height * SHEET_SHARE;
        match (browse.collapsed, read.selected) {
            (false, _) => {
                anchored_window(
                    ui.ctx(),
                    "sheet",
                    anchor,
                    remaining_insets,
                    placement.opacity,
                    |ui| {
                        ui.set_width(inner_width);
                        ui.set_max_height(height);
                        self.tab_body(ui, read, browse, account, dump);
                    },
                );
            }
            (true, Some(highlight)) => {
                anchored_window(
                    ui.ctx(),
                    "sheet",
                    anchor,
                    remaining_insets,
                    placement.opacity,
                    |ui| {
                        ui.set_width(inner_width);
                        ui.set_max_height(height);
                        egui::ScrollArea::vertical()
                            .max_height(height)
                            .auto_shrink([false, true])
                            .show(ui, |ui| self.selection(ui, read, highlight));
                    },
                );
            }
            (true, None) => {}
        }
        remaining_insets
    }

    fn tab_body(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        browse: &mut Browse,
        account: &mut yno::Account,
        dump: Option<&world::Dump>,
    ) {
        match browse.tab {
            Tab::Worlds => self.worlds_tab(ui, read, &mut browse.worlds, &mut browse.collapsed),
            Tab::Authors => self.authors_tab(ui, read, &mut browse.authors, &mut browse.collapsed),
            Tab::Versions => {
                self.versions_tab(ui, read, &mut browse.versions, &mut browse.collapsed)
            }
            // Never folded: settings hold no list a selection needs room from.
            Tab::Settings => {
                egui::ScrollArea::vertical()
                    .max_height(ui.available_height())
                    .auto_shrink([false, true])
                    .show(ui, |ui| self.settings(ui, browse, account, dump));
            }
        }
    }

    /// The open tab's list or, while a selection has the list folded, a line that unfolds the list.
    fn foldable_list(
        &mut self,
        ui: &mut egui::Ui,
        collapsed: &mut bool,
        summary: String,
        rows: impl FnOnce(&mut Self, &mut egui::Ui),
    ) {
        if *collapsed {
            if ui
                .button(format!("{}  {summary}", ICON_EXPAND_MORE.codepoint))
                .clicked()
            {
                *collapsed = false;
            }
            return;
        }
        ui.weak(summary);
        egui::ScrollArea::vertical()
            .max_height(ui.available_height())
            .auto_shrink([false, true])
            .show(ui, |ui| rows(self, ui));
    }

    fn worlds_tab(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        search: &mut String,
        collapsed: &mut bool,
    ) {
        let data = read.data;
        egui::ComboBox::from_label(t!("arrangement"))
            .selected_text(self.arrangement.name())
            .show_ui(ui, |ui| {
                for (heading, group) in [
                    (t!("dimensions-2d"), &Arrangement::PLANAR[..]),
                    (t!("dimensions-3d"), &Arrangement::SPATIAL[..]),
                ] {
                    ui.weak(heading);
                    for &arrangement in group {
                        ui.selectable_value(&mut self.arrangement, arrangement, arrangement.name());
                    }
                }
            });
        ui.weak(format!(
            "{}  ·  {}",
            t!(
                "graph-size",
                worlds = data.titles.len(),
                connections = data.graph.edge_count()
            ),
            t!("fps", fps = format!("{:.0}", read.fps))
        ));
        if read.selected.is_none() {
            ui.label(t!("nothing-selected"));
            if !data.untaken.is_empty()
                && ui
                    .link(t!("untaken-worlds"))
                    .on_hover_text(t!("untaken-worlds-hint"))
                    .clicked()
            {
                self.lit = Some(Some(Highlight::Untaken));
            }
        }
        ui.separator();
        if search_box(ui, search, "worlds", t!("search-worlds")) {
            *collapsed = false;
        }
        if read.candidates.is_empty() {
            return;
        }
        self.foldable_list(ui, collapsed, worlds(read.candidates.len()), |panel, ui| {
            for &world in &read.candidates {
                let selected = read.selected.and_then(Highlight::world) == Some(world);
                let row =
                    panel.world_row(ui, world, Tile::new(selected, data.titles[world].show()));
                panel.with_directions(&row, read, world);
            }
        });
    }

    fn authors_tab(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        search: &mut String,
        collapsed: &mut bool,
    ) {
        let data = read.data;
        if search_box(ui, search, "authors", t!("search-authors")) {
            *collapsed = false;
        }
        let summary = showing(read.authors.len(), data.authors.len(), "authors");
        self.foldable_list(ui, collapsed, summary, |panel, ui| {
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
                    panel.lit = Some(Some(Highlight::Author(author)));
                }
            }
        });
    }

    /// Lists every release, as one is found as often by reading down the history as by name.
    fn versions_tab(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        search: &mut String,
        collapsed: &mut bool,
    ) {
        let data = read.data;
        if search_box(ui, search, "versions", t!("search-versions")) {
            *collapsed = false;
        }
        let summary = showing(read.versions.len(), data.versions.len(), "versions");
        self.foldable_list(ui, collapsed, summary, |panel, ui| {
            for &version in &read.versions {
                panel.version(ui, read, version);
            }
        });
    }

    /// The row highlights the release, and each picture in the row selects that picture's world.
    fn version(&mut self, ui: &mut egui::Ui, read: &PanelData, version: usize) {
        let data = read.data;
        let release = &data.versions[version];
        // egui ids derive from content, so the release index keeps identical rows apart.
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

/// `of` is the box's egui id, never shown; `hint` shows in the empty box.
/// Answers whether anything was typed or cleared.
fn search_box(ui: &mut egui::Ui, search: &mut String, of: &str, hint: String) -> bool {
    let clear_id = egui::Id::new(of);
    let clear_size = egui::Vec2::splat(ui.spacing().interact_size.y);
    let output = egui::TextEdit::singleline(search)
        .hint_text(hint)
        .prefix(ICON_SEARCH)
        .suffix(egui::Atom::custom(clear_id, clear_size))
        .desired_width(f32::INFINITY)
        .show(ui);
    let typed = output.response.changed();
    if let Some(rect) = output.response.rect(clear_id)
        && ui.place(rect, egui::Label::new("❌")).clicked()
    {
        search.clear();
        return true;
    }
    typed
}

/// `kind` names the list, which has a message per form: a language may word "3 authors" and
/// "3 of 9 authors" differently.
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
