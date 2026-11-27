//! The controls and words overlaid on the graph, outside the windows: the layer rocker, the
//! right-click menu, and the message box naming what is hovered.

use super::*;

impl Panel {
    /// Steps the highlighted layer in and out from the origin.
    ///
    /// An [`egui::Area`] in a corner of the graph, visible whatever the windows show. The middle
    /// position is off, dropping the layer without clicking a world.
    pub(super) fn rocker(
        &mut self,
        ui: &mut egui::Ui,
        read: &PanelData,
        anchor: egui::Align2,
        insets: Insets,
        opacity: f32,
    ) {
        let deepest = read.data.deepest;
        let lit = match read.selected {
            Some(Highlight::Layer(depth)) => Some(depth),
            _ => None,
        };
        anchored_window(ui.ctx(), "layer rocker", anchor, insets, opacity, |ui| {
            // Scaled, so arrows and padding grow together with the UI scale.
            let style = ui.style_mut();
            for font in style.text_styles.values_mut() {
                font.size *= ROCKER_SCALE;
            }
            style.spacing.button_padding *= ROCKER_SCALE;
            style.spacing.item_spacing *= ROCKER_SCALE;
            // A skin's arrow takes the chevron's place without resizing the button.
            let glyph = egui::WidgetText::from(ICON_KEYBOARD_ARROW_UP)
                .into_galley(
                    ui,
                    Some(egui::TextWrapMode::Extend),
                    f32::INFINITY,
                    egui::TextStyle::Button,
                )
                .size();
            let button_size = glyph + 2.0 * ui.spacing().button_padding;
            let face = |arrow: skin::Arrow, icon: egui_material_icons::MaterialIcon| {
                match skin::skin_arrow(ui.ctx(), arrow, glyph.x) {
                    Some(image) => egui::Button::image(image).min_size(button_size),
                    None => egui::Button::new(icon),
                }
            };
            let (up, down) = (
                face(skin::Arrow::Up, ICON_KEYBOARD_ARROW_UP),
                face(skin::Arrow::Down, ICON_KEYBOARD_ARROW_DOWN),
            );
            ui.vertical_centered(|ui| {
                if ui
                    .add_enabled(lit != Some(0), up)
                    .on_hover_text(t!("rocker-shallower"))
                    .clicked()
                {
                    self.lit = Some(Some(Highlight::Layer(
                        lit.map_or(deepest, |depth| depth.saturating_sub(1)),
                    )));
                }
                if ui
                    .add_enabled(lit != Some(deepest), down)
                    .on_hover_text(t!("rocker-deeper"))
                    .clicked()
                {
                    self.lit = Some(Some(Highlight::Layer(
                        lit.map_or(0, |depth| (depth + 1).min(deepest)),
                    )));
                }
                ui.separator();
                let reset = egui::Button::new(ICON_HOME).min_size(button_size);
                if ui.add(reset).on_hover_text(t!("reset-camera")).clicked() {
                    self.reset_camera = true;
                }
            });
        });
    }

    pub(super) fn menu(&mut self, ui: &mut egui::Ui, read: &PanelData) {
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
                    if data.descendant_counts[world] > 0
                        && ui.button(t!("menu-descendants")).clicked()
                    {
                        self.lit = Some(Some(Highlight::Descendants(world)));
                    }
                    self.directions_menu(ui, read, world);
                    let named = data.titles[world].is_searchable();
                    if named && ui.button(t!("menu-open-wiki")).clicked() {
                        open_in_browser(&data.titles[world].wiki_url());
                        self.menu_taken = true;
                    }
                    if named && speaking_japanese() && ui.button(t!("world-english-wiki")).clicked()
                    {
                        open_in_browser(&world::wiki_url(&data.titles[world].en));
                        self.menu_taken = true;
                    }
                    if !data.maps[world].is_empty() && ui.button(t!("world-map-hint")).clicked() {
                        self.mapped = Some(world);
                        self.menu_taken = true;
                    }
                    // Such a world exists only while the graph is a frontier.
                    if !named
                        && ui
                            .button(t!("menu-reveal"))
                            .on_hover_text(t!("menu-reveal-hint"))
                            .clicked()
                    {
                        self.force_reveal_world = Some(world);
                        self.menu_taken = true;
                    }
                });
            });
    }

    /// The message box naming the hovered world, hidden while a menu over that world is open.
    /// Passes the pointer through, keeping the hover on the graph.
    pub(super) fn message(&self, ui: &mut egui::Ui, read: &PanelData, insets: Insets, width: f32) {
        let Some(world) = read.hovered.filter(|_| read.menu.is_none()) else {
            return;
        };
        let data = read.data;
        message_area("world message", insets)
            .interactable(false)
            .show(ui.ctx(), |ui| {
                skin::window(ui, |ui| {
                    ui.set_width(width);
                    ui.label(
                        egui::RichText::new(data.titles[world].show())
                            .color(ui.visuals().hyperlink_color),
                    );
                    ui.weak(data.authors[data.author_of[world]].name.show());
                });
            });
    }
}
